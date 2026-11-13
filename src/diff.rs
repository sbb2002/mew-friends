//! 알림 판정 — 순수 함수. `이전에 본 것 + 새 JSON → 알림 목록`.
//! 명세: docs/IMPL_SPEC.md §4. [담당: 작업 A]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{Preview, TweetsFile};

/// 「본 것」 기록. state.json 에 저장된다.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Seen {
    #[serde(default)]
    pub preview_ready: bool,
    #[serde(default)]
    pub tweets_ready: bool,
    /// 이미 「방송 시작」을 알린(또는 첫 실행 때 이미 live 였던) 방송 id. 오래된 것부터 앞.
    #[serde(default)]
    pub live_ids: Vec<String>,
    /// 이미 본 트윗 id. 오래된 것부터 앞.
    #[serde(default)]
    pub tweet_ids: Vec<String>,
    /// 처음 볼 때 번역이 없어 토스트를 미룬 트윗 id.
    #[serde(default)]
    pub pending_tl: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    LiveStarted { item_id: String, members: Vec<String>, title: String, url: String },
    NewTweet { member: String, tweet_id: String, text_ko: String, url: String },
}

pub const SEEN_CAP: usize = 2000;

pub fn diff_preview(seen: &mut Seen, p: &Preview) -> Vec<Notice> {
    let _ = (seen, p);
    todo!("작업 A")
}

pub fn diff_tweets(seen: &mut Seen, t: &TweetsFile, now: DateTime<Utc>) -> Vec<Notice> {
    let _ = (seen, t, now);
    todo!("작업 A")
}
