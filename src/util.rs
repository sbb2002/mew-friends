//! 공용 도구 — 주소 열기 · 앱 폴더 · KST 시각 표기 · 와이드 문자열.

use std::path::PathBuf;

use chrono::{DateTime, Datelike, FixedOffset, Timelike, Utc};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{PCWSTR, w};

/// 토스트 · 트레이에서 쓰는 앱 이름 · 알림 앱 ID(AUMID).
pub const APP_NAME: &str = "Mew Friends";
pub const AUMID: &str = "sbb2002.MewFriends";

/// 널 종료 UTF-16 문자열.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 기본 브라우저로 주소를 연다. http(s) 주소만 허용한다.
pub fn open_url(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    let u = wide(url);
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(u.as_ptr()), None, None, SW_SHOWNORMAL);
    }
}

/// `%APPDATA%\mew_friends` (없으면 만든다).
pub fn app_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir());
    let d = base.join("mew_friends");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// ISO UTC(`...Z`) → DateTime<Utc>.
pub fn parse_utc(iso: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(iso).ok().map(|d| d.with_timezone(&Utc))
}

fn kst() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).unwrap()
}

/// ISO UTC → `11/13 (목) 22:00` (KST). 해석 실패면 None.
pub fn fmt_kst(iso: &str) -> Option<String> {
    let t = parse_utc(iso)?.with_timezone(&kst());
    const WD: [&str; 7] = ["월", "화", "수", "목", "금", "토", "일"];
    Some(format!(
        "{}/{} ({}) {:02}:{:02}",
        t.month(),
        t.day(),
        WD[t.weekday().num_days_from_monday() as usize],
        t.hour(),
        t.minute()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kst_format() {
        // 2026-11-13 13:00Z = KST 22:00 금요일
        assert_eq!(fmt_kst("2026-11-13T13:00:00Z").as_deref(), Some("11/13 (금) 22:00"));
        // 자정 넘김
        assert_eq!(fmt_kst("2026-11-13T15:30:00Z").as_deref(), Some("11/14 (토) 00:30"));
        assert_eq!(fmt_kst("bad"), None);
    }
}
