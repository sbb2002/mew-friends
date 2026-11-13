//! 저장 — `%APPDATA%\mew_friends\state.json`. 명세: docs/IMPL_SPEC.md §5. [담당: 작업 A]

use serde::{Deserialize, Serialize};

use crate::diff::Seen;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub live_alerts: bool,
    pub tweet_alerts: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { live_alerts: true, tweet_alerts: true }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AppState {
    #[serde(default)]
    pub seen: Seen,
    #[serde(default)]
    pub settings: Settings,
}

pub fn load() -> AppState {
    todo!("작업 A")
}

pub fn save(s: &AppState) {
    let _ = s;
    todo!("작업 A")
}
