//! 멤버 패널 — 방송 보러 가기 · 트윗 목록. 명세: docs/IMPL_SPEC.md §8. [담당: 작업 D]

use chrono::{DateTime, Utc};

use crate::model::{Preview, TweetsFile};

#[derive(Debug, Clone, PartialEq)]
pub struct PanelTweet {
    /// 번역, 없으면 "번역 준비 중…"
    pub text: String,
    pub url: Option<String>,
    /// KST 표기(`11/13 (금) 16:27`), 모르면 빈 문자열
    pub when: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelData {
    pub member: String,
    pub name_ko: String,
    /// 방송 중이면 (제목, 주소)
    pub live: Option<(String, String)>,
    /// 최신이 앞
    pub tweets: Vec<PanelTweet>,
}

/// 순수 함수 — 패널에 보일 내용을 만든다.
pub fn build_data(member: &str, preview: &Preview, tweets: &TweetsFile, now: DateTime<Utc>) -> PanelData {
    let _ = (member, preview, tweets, now);
    todo!("작업 D")
}

/// 패널을 띄운다(이미 떠 있으면 내용을 바꾸고 앞으로). 메인 스레드에서 부른다.
pub fn show(data: PanelData) {
    let _ = data;
    todo!("작업 D")
}
