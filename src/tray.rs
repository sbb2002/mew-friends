//! 트레이 아이콘 · 메뉴. 명세: docs/IMPL_SPEC.md §6. [담당: 작업 B]

use tray_icon::menu::{
    CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MessageBoxW};
use windows::core::PCWSTR;

use crate::model;
use crate::state::Settings;
use crate::util;

const TRAY_ICON_PNG: &[u8] = include_bytes!("../assets/icons/yuno.png");

const ID_LIVE: &str = "live";
const ID_TWEET: &str = "tweet";
const ID_AUTOSTART: &str = "autostart";
const ID_TOPMOST: &str = "topmost";
const ID_AUTOBUBBLE: &str = "autobubble";
const ID_INFO: &str = "info";
const ID_QUIT: &str = "quit";
const ID_OPEN_PREFIX: &str = "open:";

#[derive(Debug, Clone, PartialEq)]
pub enum TrayAction {
    OpenPanel(String),
    SetLiveAlerts(bool),
    SetTweetAlerts(bool),
    SetAlwaysOnTop(bool),
    SetAutoBubble(bool),
    SetAutostart(bool),
    Quit,
}

pub struct Tray {
    // drop 되면 아이콘이 사라지므로 들고 있는다.
    _icon: TrayIcon,
    // 메뉴 항목 핸들 — 이벤트 시점의 체크 상태를 읽는 데 쓴다.
    live: CheckMenuItem,
    tweet: CheckMenuItem,
    topmost: CheckMenuItem,
    autobubble: CheckMenuItem,
    autostart: CheckMenuItem,
}

impl Tray {
    /// `order` = 멤버 key 순서. 메인 스레드(메시지 루프가 도는 스레드)에서 만든다.
    pub fn new(order: &[String], settings: &Settings, autostart: bool) -> Result<Tray, String> {
        let icon = load_icon()?;

        // 「패널 열기」 하위 메뉴 — order 순서의 멤버.
        let open_items: Vec<MenuItem> = order
            .iter()
            .map(|k| {
                MenuItem::with_id(
                    format!("{ID_OPEN_PREFIX}{k}"),
                    model::short_name_ko(k),
                    true,
                    None,
                )
            })
            .collect();
        let open_refs: Vec<&dyn IsMenuItem> =
            open_items.iter().map(|i| i as &dyn IsMenuItem).collect();
        let open_menu = Submenu::with_items("패널 열기", true, &open_refs)
            .map_err(|e| format!("하위 메뉴 생성 실패: {e}"))?;

        let live = CheckMenuItem::with_id(
            ID_LIVE,
            "방송 시작 알림",
            true,
            settings.live_alerts,
            None,
        );
        let tweet = CheckMenuItem::with_id(
            ID_TWEET,
            "새 트윗 알림",
            true,
            settings.tweet_alerts,
            None,
        );
        let autobubble = CheckMenuItem::with_id(
            ID_AUTOBUBBLE,
            "새 트윗 말풍선 자동으로 띄우기",
            true,
            settings.auto_bubble,
            None,
        );
        let topmost = CheckMenuItem::with_id(
            ID_TOPMOST,
            "캐릭터 · 말풍선 항상 위에",
            true,
            settings.always_on_top,
            None,
        );
        let autostart_item = CheckMenuItem::with_id(
            ID_AUTOSTART,
            "Windows 시작 시 실행",
            true,
            autostart,
            None,
        );
        let info = MenuItem::with_id(ID_INFO, "정보", true, None);
        let quit = MenuItem::with_id(ID_QUIT, "종료", true, None);
        let sep1 = PredefinedMenuItem::separator();
        let sep2 = PredefinedMenuItem::separator();
        let sep3 = PredefinedMenuItem::separator();

        let menu = Menu::with_items(&[
            &open_menu,
            &sep1,
            &live,
            &tweet,
            &autobubble,
            &sep3,
            &topmost,
            &autostart_item,
            &sep2,
            &info,
            &quit,
        ])
        .map_err(|e| format!("메뉴 생성 실패: {e}"))?;

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Mew Friends")
            .with_icon(icon)
            .with_menu_on_left_click(true)
            .build()
            .map_err(|e| format!("트레이 아이콘 생성 실패: {e}"))?;

        Ok(Tray { _icon: tray, live, tweet, topmost, autobubble, autostart: autostart_item })
    }

    /// 쌓인 메뉴 동작을 꺼낸다(막히지 않음). 메시지 루프가 한 바퀴 돌 때마다 부른다.
    pub fn poll(&mut self) -> Vec<TrayAction> {
        let mut out = Vec::new();
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            let id: &str = ev.id.as_ref();
            if let Some(key) = id.strip_prefix(ID_OPEN_PREFIX) {
                out.push(TrayAction::OpenPanel(key.to_string()));
                continue;
            }
            match id {
                ID_LIVE => out.push(TrayAction::SetLiveAlerts(self.live.is_checked())),
                ID_TWEET => out.push(TrayAction::SetTweetAlerts(self.tweet.is_checked())),
                ID_TOPMOST => out.push(TrayAction::SetAlwaysOnTop(self.topmost.is_checked())),
                ID_AUTOBUBBLE => out.push(TrayAction::SetAutoBubble(self.autobubble.is_checked())),
                ID_AUTOSTART => {
                    out.push(TrayAction::SetAutostart(self.autostart.is_checked()))
                }
                ID_INFO => show_info(),
                ID_QUIT => out.push(TrayAction::Quit),
                _ => {}
            }
        }
        out
    }
}

/// 트레이 아이콘 PNG → RGBA.
fn load_icon() -> Result<Icon, String> {
    let img = image::load_from_memory(TRAY_ICON_PNG)
        .map_err(|e| format!("트레이 아이콘 디코드 실패: {e}"))?
        .into_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).map_err(|e| format!("트레이 아이콘 변환 실패: {e}"))
}

/// 「정보」 — 메시지 상자. 이 동작은 TrayAction 을 내지 않는다.
fn show_info() {
    let text = util::wide(
        "Mew Friends (MVP)\n비공식 팬 제작 · AI 생성 일러스트 · 공식과 무관\n데이터: mewtype-schduler.vercel.app",
    );
    let caption = util::wide(util::APP_NAME);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            PCWSTR(caption.as_ptr()),
            MB_ICONINFORMATION,
        );
    }
}
