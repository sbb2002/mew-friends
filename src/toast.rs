//! 윈도우 토스트 알림. 명세: docs/IMPL_SPEC.md §6. [담당: 작업 B]

/// 시작할 때 한 번 — 알림 앱 ID 등록 · 아이콘 파일 준비.
pub fn init() {
    todo!("작업 B")
}

/// 토스트를 띄운다. 누르면 `url` 을 연다. `member` 가 있으면 그 멤버 아이콘.
pub fn show(title: &str, body: &str, url: Option<String>, member: Option<&str>) {
    let _ = (title, body, url, member);
    todo!("작업 B")
}
