//! 트레이 아이콘 · 메뉴. 명세: docs/IMPL_SPEC.md §6. [담당: 작업 B]

use crate::state::Settings;

#[derive(Debug, Clone, PartialEq)]
pub enum TrayAction {
    OpenPanel(String),
    SetLiveAlerts(bool),
    SetTweetAlerts(bool),
    SetAutostart(bool),
    Quit,
}

pub struct Tray {
    _private: (),
}

impl Tray {
    /// `order` = 멤버 key 순서. 메인 스레드(메시지 루프가 도는 스레드)에서 만든다.
    pub fn new(order: &[String], settings: &Settings, autostart: bool) -> Result<Tray, String> {
        let _ = (order, settings, autostart);
        todo!("작업 B")
    }

    /// 쌓인 메뉴 동작을 꺼낸다(막히지 않음). 메시지 루프가 한 바퀴 돌 때마다 부른다.
    pub fn poll(&mut self) -> Vec<TrayAction> {
        todo!("작업 B")
    }
}
