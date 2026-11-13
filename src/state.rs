//! 저장 — `%APPDATA%\mew_friends\state.json`. 명세: docs/IMPL_SPEC.md §4-3. [담당: 작업 A]

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diff::Seen;
use crate::util;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub live_alerts: bool,
    pub tweet_alerts: bool,
    /// 캐릭터 · 말풍선을 항상 위에(끄면 다른 창 뒤 바탕화면에만 — 방해하지 않기)
    pub always_on_top: bool,
    /// 새 트윗이 오면 그 캐릭터 위에 말풍선을 자동으로 5초 띄운다
    pub auto_bubble: bool,
    /// 트윗 시트 면 — true = 원문(X 카드) 면으로 연다(원본 mew:tllang)
    pub tweet_face_orig: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { live_alerts: true, tweet_alerts: true, always_on_top: false, auto_bubble: true, tweet_face_orig: false }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AppState {
    #[serde(default)]
    pub seen: Seen,
    #[serde(default)]
    pub settings: Settings,
}

fn state_path() -> PathBuf {
    util::app_dir().join("state.json")
}

/// 원문을 해석한다. 실패하면 Err(원문 그대로 둘 메시지).
fn parse(text: &str) -> Result<AppState, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}

/// `state.json` 을 읽는다. 없거나 읽기 실패면 기본값, 해석 실패면 원본을 `state.json.bad` 로 옮기고 기본값.
pub fn load() -> AppState {
    load_from(&state_path())
}

fn load_from(path: &Path) -> AppState {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("[state] 읽기 실패: {e}");
            }
            return AppState::default();
        }
    };
    match parse(&text) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[state] 해석 실패, state.json.bad 로 옮김: {e}");
            let bad = path.with_extension("json.bad");
            if let Err(e2) = std::fs::rename(path, &bad) {
                eprintln!("[state] 이름 바꾸기 실패: {e2}");
            }
            AppState::default()
        }
    }
}

/// `state.json` 에 쓴다. 같은 폴더의 `.tmp` 에 쓰고 바꿔치기(쓰다 꺼져도 깨지지 않게).
pub fn save(s: &AppState) {
    save_to(&state_path(), s);
}

fn save_to(path: &Path, s: &AppState) {
    let text = match serde_json::to_string_pretty(s) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[state] 직렬화 실패: {e}");
            return;
        }
    };
    let tmp = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp, text) {
        eprintln!("[state] 쓰기 실패: {e}");
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        eprintln!("[state] 바꿔치기 실패: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appstate_roundtrip() {
        let mut s = AppState::default();
        s.seen.preview_ready = true;
        s.seen.live_ids.push("x".into());
        s.settings.tweet_alerts = false;
        let text = serde_json::to_string_pretty(&s).unwrap();
        assert_eq!(parse(&text).unwrap(), s);
    }

    #[test]
    fn old_file_missing_fields_read() {
        let s = parse("{}").unwrap();
        assert_eq!(s, AppState::default());
        assert!(!s.seen.preview_ready);
        assert!(s.settings.live_alerts && s.settings.tweet_alerts);
        // 설정 일부만 있는 옛 파일
        let s = parse(r#"{"settings":{"live_alerts":false}}"#).unwrap();
        assert!(!s.settings.live_alerts);
        assert!(s.settings.tweet_alerts);
        // 새 설정 기본값: 항상 위 끔 · 자동 말풍선 켬
        assert!(!s.settings.always_on_top && s.settings.auto_bubble);
    }

    #[test]
    fn broken_json_is_error() {
        assert!(parse("{not json").is_err());
    }
}
