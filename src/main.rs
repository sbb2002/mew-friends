//! Mew Friends — 무겐다이 뮤타입 비공식 팬 제작 바탕화면 친구 (MVP).
//! 바탕화면(다른 창 뒤)에 멤버 동물 캐릭터가 살고, 방송 시작 · 새 번역 트윗을 토스트로 알린다.
//! 전체 구조 · 모듈 계약 = docs/IMPL_SPEC.md.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod critter;
mod diff;
mod model;
mod net;
mod panel;
mod player;
mod poll;
mod state;
mod toast;
mod tray;
mod util;

use std::sync::mpsc;

use chrono::Utc;
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, KillTimer, MSG, PostQuitMessage, SetTimer, TranslateMessage,
    WM_TIMER,
};
use windows::core::w;

use crate::diff::Notice;
use crate::model::{Preview, TweetsFile};

/// 토스트 본문 최대 글자 수.
const TOAST_BODY_MAX: usize = 120;

struct App {
    st: state::AppState,
    critters: critter::Critters,
    preview: Option<Preview>,
    tweets: Option<TweetsFile>,
}

impl App {
    fn on_poll(&mut self, r: poll::PollResult) {
        if let Some(p) = r.preview {
            let notices = diff::diff_preview(&mut self.st.seen, &p);
            // LIVE 표시는 알림과 별개로 지금 데이터 기준으로 매번 다시 맞춘다.
            for m in p.order() {
                let live = p.items.iter().any(|i| i.state == "live" && i.members().contains(&m));
                self.critters.set_live(&m, live);
            }
            for n in notices {
                if let Notice::LiveStarted { members, title, url, .. } = n {
                    if self.st.settings.live_alerts {
                        let names: Vec<String> = members.iter().map(|m| p.name_ko(m)).collect();
                        toast::show(
                            &format!("{} 방송 시작", names.join(" · ")),
                            &title,
                            Some(url),
                            members.first().map(|s| s.as_str()),
                        );
                    }
                }
            }
            self.preview = Some(p);
        }
        // 이번 폴링에서 새 트윗이 온 멤버(여럿이면 마지막) — 자동 말풍선 대상
        let mut auto_member: Option<String> = None;
        if let Some(t) = r.tweets {
            let notices = diff::diff_tweets(&mut self.st.seen, &t, Utc::now());
            for n in notices {
                if let Notice::NewTweet { member, text_ko, url, .. } = n {
                    self.critters.set_unread(&member, true);
                    auto_member = Some(member.clone());
                    if self.st.settings.tweet_alerts {
                        let name = self
                            .preview
                            .as_ref()
                            .map(|p| p.name_ko(&member))
                            .unwrap_or_else(|| model::short_name_ko(&member).to_string());
                        let body: String = text_ko.chars().take(TOAST_BODY_MAX).collect();
                        toast::show(&format!("{name} 새 트윗"), &body, Some(url), Some(&member));
                    }
                }
            }
            self.tweets = Some(t);
        }
        // 새 트윗 → 그 캐릭터 위에 자동 말풍선(전체 화면 앱 중에는 띄우지 않는다).
        // 그 밖엔 열린 말풍선을 새 데이터로 갈아 끼운다.
        let auto = auto_member
            .filter(|_| self.st.settings.auto_bubble && !self.critters.is_busy())
            .and_then(|m| self.critters.anchor(&m).map(|a| (m, a)));
        if let Some((m, a)) = auto {
            panel::show_auto(self.panel_data(&m), a);
        } else if let Some(m) = panel::current() {
            panel::update(self.panel_data(&m));
        }
        state::save(&self.st);
    }

    fn panel_data(&self, member: &str) -> panel::PanelData {
        let empty_p = Preview::default();
        let empty_t = TweetsFile::default();
        panel::build_data(
            member,
            self.preview.as_ref().unwrap_or(&empty_p),
            self.tweets.as_ref().unwrap_or(&empty_t),
            Utc::now(),
        )
    }

    /// 캐릭터 머리 위 말풍선으로 트윗 시트를 연다(같은 멤버가 열려 있으면 닫는다).
    fn open_panel(&mut self, member: &str) {
        let Some(anchor) = self.critters.anchor(member) else {
            eprintln!("[main] {member} 캐릭터 위치를 몰라 말풍선을 띄우지 못함");
            return;
        };
        panel::show(self.panel_data(member), anchor);
        self.critters.set_unread(member, false);
    }
}

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // 한 번만 실행
        let _mutex = CreateMutexW(None, false, w!("Local\\MewFriendsSingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
    }

    toast::init();

    // 확인용: `mew_friends.exe --test-toast` → 시험 토스트 1개를 띄우고 60초 동안 클릭을 기다린다
    // (설치 없는 exe 에서 앱 이름 · 아이콘 · 누르면 주소 열림을 확인 — PLAN Phase 0).
    if std::env::args().any(|a| a == "--test-toast") {
        toast::show(
            "유노 방송 시작 (시험)",
            "이 알림을 누르면 mewtype-scheduler 사이트가 열려야 합니다.",
            Some("https://mewtype-schduler.vercel.app/".to_string()),
            Some("yuno"),
        );
        let start = std::time::Instant::now();
        let mut msg = MSG::default();
        let t = unsafe { SetTimer(None, 0, 500, None) };
        while start.elapsed().as_secs() < 60 {
            if unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 <= 0 {
                break;
            }
            unsafe { DispatchMessageW(&msg) };
        }
        unsafe {
            let _ = KillTimer(None, t);
        }
        return;
    }

    let st = state::load();
    let order: Vec<String> = model::DEFAULT_ORDER.iter().map(|s| s.to_string()).collect();

    let critters = match critter::Critters::new(&order) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[main] 캐릭터 창 생성 실패: {e}");
            return;
        }
    };
    let mut tray = match tray::Tray::new(&order, &st.settings, autostart::is_enabled()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[main] 트레이 생성 실패: {e}");
            return;
        }
    };

    let mut app = App { st, critters, preview: None, tweets: None };
    app.critters.set_topmost(app.st.settings.always_on_top);
    panel::set_topmost(app.st.settings.always_on_top);
    panel::set_face_pref(app.st.settings.tweet_face_orig);

    let (tx, rx) = mpsc::channel();
    poll::spawn(tx, unsafe { GetCurrentThreadId() });

    // 스레드 타이머(hwnd 없음) — 캐릭터 움직임 박자.
    let mut interval = app.critters.frame_interval_ms().max(16);
    let mut timer_id = unsafe { SetTimer(None, 0, interval, None) };

    let mut msg = MSG::default();
    loop {
        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if r.0 == 0 || r.0 == -1 {
            break;
        }
        if msg.hwnd == HWND::default() && msg.message == poll::WM_APP_POLL {
            while let Ok(res) = rx.try_recv() {
                app.on_poll(res);
            }
        } else if msg.hwnd == HWND::default() && msg.message == WM_TIMER {
            app.critters.tick();
            let next = app.critters.frame_interval_ms().max(16);
            if next != interval {
                unsafe {
                    let _ = KillTimer(None, timer_id);
                    timer_id = SetTimer(None, 0, next, None);
                }
                interval = next;
            }
        } else {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        for m in app.critters.take_clicks() {
            app.open_panel(&m);
        }
        for m in panel::take_requests() {
            app.open_panel(&m);
        }
        // 자동 말풍선을 눌러 고정 = 읽은 것으로(새 트윗 점 지우기)
        for m in panel::take_pinned() {
            app.critters.set_unread(&m, false);
        }
        // 「번역 · 원문」 탭 저장값
        if let Some(orig) = panel::take_face_change() {
            app.st.settings.tweet_face_orig = orig;
            state::save(&app.st);
        }
        // 말풍선이 캐릭터를 따라가고, 말하는 동안 유노는 멈춰 선다.
        let talking = panel::current();
        if let Some(a) = talking.as_deref().and_then(|m| app.critters.anchor(m)) {
            panel::follow(a);
        }
        app.critters.set_talking(talking.as_deref());
        for a in tray.poll() {
            match a {
                tray::TrayAction::OpenPanel(m) => app.open_panel(&m),
                tray::TrayAction::SetLiveAlerts(b) => {
                    app.st.settings.live_alerts = b;
                    state::save(&app.st);
                }
                tray::TrayAction::SetTweetAlerts(b) => {
                    app.st.settings.tweet_alerts = b;
                    state::save(&app.st);
                }
                tray::TrayAction::SetAlwaysOnTop(b) => {
                    app.st.settings.always_on_top = b;
                    app.critters.set_topmost(b);
                    panel::set_topmost(b);
                    state::save(&app.st);
                }
                tray::TrayAction::SetAutoBubble(b) => {
                    app.st.settings.auto_bubble = b;
                    state::save(&app.st);
                }
                tray::TrayAction::SetAutostart(b) => {
                    if let Err(e) = autostart::set_enabled(b) {
                        eprintln!("[main] 시작 프로그램 설정 실패: {e}");
                    }
                }
                tray::TrayAction::Quit => unsafe { PostQuitMessage(0) },
            }
        }
    }
}
