//! 윈도우 토스트 알림. 명세: docs/IMPL_SPEC.md §6. [담당: 작업 B]

use std::path::PathBuf;

use tauri_winrt_notification::{IconCrop, Toast};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegSetValueExW,
};
use windows::core::PCWSTR;

use crate::util;

/// 멤버 아이콘 5개 — 빌드에 포함해 두고 시작할 때 앱 폴더로 내려 쓴다.
const MEMBER_ICONS: [(&str, &[u8]); 5] = [
    ("arale", include_bytes!("../assets/icons/arale.png")),
    ("yuno", include_bytes!("../assets/icons/yuno.png")),
    ("nonoka", include_bytes!("../assets/icons/nonoka.png")),
    ("ritsu", include_bytes!("../assets/icons/ritsu.png")),
    ("miyako", include_bytes!("../assets/icons/miyako.png")),
];

/// 알림 앱 ID 레지스트리 경로(HKCU 아래).
const AUMID_SUBKEY: &str = r"Software\Classes\AppUserModelId\sbb2002.MewFriends";

/// 멤버 key 의 아이콘 바이트. 알려진 멤버가 아니면 None.
fn icon_bytes(key: &str) -> Option<&'static [u8]> {
    MEMBER_ICONS.iter().find(|(k, _)| *k == key).map(|(_, b)| *b)
}

/// 시작할 때 한 번 — 알림 앱 ID 등록 · 아이콘 파일 준비.
pub fn init() {
    let dir = util::app_dir().join("icons");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("[toast] 아이콘 폴더 생성 실패: {e}");
        return;
    }
    for (key, bytes) in MEMBER_ICONS {
        let path = dir.join(format!("{key}.png"));
        // 이미 있고 크기가 같으면 건너뛴다.
        let same = std::fs::metadata(&path)
            .map(|m| m.len() == bytes.len() as u64)
            .unwrap_or(false);
        if !same {
            if let Err(e) = std::fs::write(&path, bytes) {
                eprintln!("[toast] 아이콘 저장 실패({key}): {e}");
            }
        }
    }
    // 아이콘 파일을 먼저 만든 뒤 등록해야 IconUri 가 가리키는 파일이 있다.
    let icon_uri = dir.join("yuno.png").display().to_string();
    if let Err(e) = register_app_id("Mew Friends", &icon_uri) {
        eprintln!("[toast] 알림 앱 ID 등록 실패: {e}");
    }
}

/// `HKCU\...\AppUserModelId\sbb2002.MewFriends` 에 DisplayName · IconUri 기록.
fn register_app_id(display_name: &str, icon_uri: &str) -> Result<(), String> {
    let mut hkey = HKEY(std::ptr::null_mut());
    let sub = util::wide(AUMID_SUBKEY);
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
    if rc.0 != 0 {
        return Err(format!("RegCreateKeyExW 실패 코드 {}", rc.0));
    }
    let r1 = set_sz(hkey, "DisplayName", display_name);
    let r2 = set_sz(hkey, "IconUri", icon_uri);
    unsafe {
        let _ = RegCloseKey(hkey);
    }
    r1.and(r2)
}

/// REG_SZ 값 쓰기(UTF-16, 끝 널 포함).
fn set_sz(hkey: HKEY, name: &str, value: &str) -> Result<(), String> {
    let name_w = util::wide(name);
    let data: Vec<u8> = util::wide(value).iter().flat_map(|c| c.to_le_bytes()).collect();
    let rc = unsafe { RegSetValueExW(hkey, PCWSTR(name_w.as_ptr()), None, REG_SZ, Some(&data)) };
    if rc.0 == 0 {
        Ok(())
    } else {
        Err(format!("RegSetValueExW({name}) 실패 코드 {}", rc.0))
    }
}

/// 토스트를 띄운다. 누르면 `url` 을 연다. `member` 가 있으면 그 멤버 아이콘.
pub fn show(title: &str, body: &str, url: Option<String>, member: Option<&str>) {
    let mut toast = Toast::new(util::AUMID).title(title).text1(body);

    if let Some(key) = member {
        if icon_bytes(key).is_some() {
            let path: PathBuf = util::app_dir().join("icons").join(format!("{key}.png"));
            toast = toast.icon(&path, IconCrop::Circular, key);
        }
    }

    if let Some(u) = url {
        toast = toast.on_activated(move |_| {
            util::open_url(&u);
            Ok(())
        });
    }

    if let Err(e) = toast.show() {
        eprintln!("[toast] 토스트 표시 실패: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_icons_cover_five_keys() {
        for key in ["arale", "yuno", "nonoka", "ritsu", "miyako"] {
            let b = icon_bytes(key).expect("아이콘 있어야 함");
            // PNG 시그니처
            assert_eq!(&b[..4], &[0x89, b'P', b'N', b'G']);
        }
        assert!(icon_bytes("unknown").is_none());
    }
}
