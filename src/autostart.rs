//! 시작 프로그램 등록 — HKCU Run. 명세: docs/IMPL_SPEC.md §6. [담당: 작업 B]

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW,
};
use windows::core::PCWSTR;

use crate::util;

const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "MewFriends";

/// 경로를 따옴표로 감싼다(공백 있는 경로 대비). 이미 감싸져 있으면 한 겹만 둔다.
fn quote_path(path: &str) -> String {
    format!("\"{}\"", path.trim().trim_matches('"'))
}

/// 저장된 값이 현재 경로와 같은지(따옴표 · 앞뒤 공백 · 대소문자 무시).
fn value_matches(stored: &str, current: &str) -> bool {
    let norm = |s: &str| s.trim().trim_matches('"').trim().to_lowercase();
    let a = norm(stored);
    !a.is_empty() && a == norm(current)
}

/// 현재 실행 파일 경로(문자열). 실패하면 None.
fn current_exe_str() -> Option<String> {
    std::env::current_exe().ok().map(|p| p.display().to_string())
}

/// Run 키의 MewFriends 값 읽기. 없으면 None.
fn read_value() -> Option<String> {
    let sub = util::wide(RUN_SUBKEY);
    let name = util::wide(VALUE_NAME);
    let mut hkey = HKEY(std::ptr::null_mut());
    let rc = unsafe {
        RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()), None, KEY_READ, &mut hkey)
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let result = (|| {
        // 1) 크기 조회
        let mut cb: u32 = 0;
        let rc = unsafe {
            RegQueryValueExW(hkey, PCWSTR(name.as_ptr()), None, None, None, Some(&mut cb as *mut u32))
        };
        if rc != ERROR_SUCCESS || cb == 0 {
            return None;
        }
        // 2) 실제 값
        let mut buf = vec![0u8; cb as usize];
        let rc = unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR(name.as_ptr()),
                None,
                None,
                Some(buf.as_mut_ptr()),
                Some(&mut cb as *mut u32),
            )
        };
        if rc != ERROR_SUCCESS {
            return None;
        }
        let units: Vec<u16> =
            buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Some(String::from_utf16_lossy(&units).trim_end_matches('\0').to_string())
    })();
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    result
}

pub fn is_enabled() -> bool {
    match (read_value(), current_exe_str()) {
        (Some(stored), Some(current)) => value_matches(&stored, &current),
        _ => false,
    }
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    let sub = util::wide(RUN_SUBKEY);
    let name = util::wide(VALUE_NAME);

    if on {
        let exe = current_exe_str().ok_or("현재 실행 파일 경로를 알 수 없음")?;
        let mut hkey = HKEY(std::ptr::null_mut());
        let rc = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(sub.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut hkey,
                None,
            )
        };
        if rc != ERROR_SUCCESS {
            return Err(format!("Run 키 열기 실패 코드 {}", rc.0));
        }
        let data: Vec<u8> = util::wide(&quote_path(&exe))
            .iter()
            .flat_map(|c| c.to_le_bytes())
            .collect();
        let rc = unsafe {
            RegSetValueExW(hkey, PCWSTR(name.as_ptr()), None, REG_SZ, Some(&data))
        };
        unsafe {
            let _ = RegCloseKey(hkey);
        }
        if rc != ERROR_SUCCESS {
            return Err(format!("Run 값 쓰기 실패 코드 {}", rc.0));
        }
        Ok(())
    } else {
        let mut hkey = HKEY(std::ptr::null_mut());
        let rc = unsafe {
            RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(sub.as_ptr()), None, KEY_SET_VALUE, &mut hkey)
        };
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(()); // 키 자체가 없으면 이미 꺼진 상태
        }
        if rc != ERROR_SUCCESS {
            return Err(format!("Run 키 열기 실패 코드 {}", rc.0));
        }
        let rc = unsafe { RegDeleteValueW(hkey, PCWSTR(name.as_ptr())) };
        unsafe {
            let _ = RegCloseKey(hkey);
        }
        // 값이 없어도 성공으로 본다.
        if rc == ERROR_SUCCESS || rc == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("Run 값 삭제 실패 코드 {}", rc.0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_adds_single_pair() {
        assert_eq!(quote_path(r"C:\Program Files\mew\mew_friends.exe"), r#""C:\Program Files\mew\mew_friends.exe""#);
        assert_eq!(quote_path(r#""C:\a b\x.exe""#), r#""C:\a b\x.exe""#);
    }

    #[test]
    fn value_matches_ignores_quotes_and_case() {
        let cur = r"C:\Users\User\Apps\Mew_Friends.exe";
        assert!(value_matches(&quote_path(cur), cur));
        assert!(value_matches(r#""c:\users\user\apps\mew_friends.exe""#, cur));
        assert!(!value_matches(r#""C:\old\mew_friends.exe""#, cur));
        assert!(!value_matches("", cur));
    }
}
