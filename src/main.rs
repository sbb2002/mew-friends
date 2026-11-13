//! Mew Friends — 무겐다이 뮤타입 비공식 팬 제작 바탕화면 친구 (MVP).
//! 바탕화면(다른 창 뒤)에 멤버 동물 캐릭터가 살고, 방송 시작 · 새 번역 트윗을 토스트로 알린다.
//! 전체 구조 · 모듈 계약 = docs/IMPL_SPEC.md.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod critter;
mod diff;
mod model;
mod panel;
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
        if let Some(t) = r.tweets {
            let notices = diff::diff_tweets(&mut self.st.seen, &t, Utc::now());
            for n in notices {
                if let Notice::NewTweet { member, text_ko, url, .. } = n {
                    self.critters.set_unread(&member, true);
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
        state::save(&self.st);
    }

    fn open_panel(&mut self, member: &str) {
        let empty_p = Preview::default();
        let empty_t = TweetsFile::default();
        let data = panel::build_data(
            member,
            self.preview.as_ref().unwrap_or(&empty_p),
            self.tweets.as_ref().unwrap_or(&empty_t),
            Utc::now(),
        );
        panel::show(data);
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
