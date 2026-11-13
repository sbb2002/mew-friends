//! 데이터 모델 — mewtype-scheduler 데이터 저장소(`data` 브랜치)의 공개 JSON.
//! 정본 스키마 = mewtype-scheduler `docs/SPEC.md` 계약 A(preview.json) · 계약 I(tweets.json).
//! 쓰는 필드만 받는다. 모르는 필드는 무시하고, 빠지거나 null 인 필드는 기본값으로 둔다.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer};

pub const PREVIEW_URL: &str =
    "https://raw.githubusercontent.com/sbb2002/mewtype-scheduler-data/data/preview.json";
pub const TWEETS_URL: &str =
    "https://raw.githubusercontent.com/sbb2002/mewtype-scheduler-data/data/tweets.json";

/// 웹 프론트 `config.js` 의 `POLL_MS`(75초)와 같다.
pub const POLL_SECS: u64 = 75;

/// 데이터가 비었을 때 쓰는 멤버 순서(= `config/channels.json` 의 `channel_order`).
pub const DEFAULT_ORDER: [&str; 5] = ["arale", "yuno", "nonoka", "ritsu", "miyako"];

/// 트레이 메뉴 등 데이터가 오기 전에도 필요한 짧은 이름.
pub fn short_name_ko(key: &str) -> &str {
    match key {
        "arale" => "아라레",
        "yuno" => "유노",
        "nonoka" => "노노카",
        "ritsu" => "리츠",
        "miyako" => "미야코",
        other => other,
    }
}

/// null 을 기본값으로 받는다(`"collab_with": null` 등).
fn null_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// `{ key: [글…] | 글 | null }` → `{ key: [글…] }`.
fn tweets_map<'de, D>(d: D) -> Result<HashMap<String, Vec<Tweet>>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        Many(Vec<Tweet>),
        One(Tweet),
    }
    let raw = Option::<HashMap<String, Option<OneOrMany>>>::deserialize(d)?.unwrap_or_default();
    Ok(raw
        .into_iter()
        .map(|(k, v)| {
            let list = match v {
                Some(OneOrMany::Many(v)) => v,
                Some(OneOrMany::One(t)) => vec![t],
                None => Vec::new(),
            };
            (k, list)
        })
        .collect())
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Preview {
    #[serde(default, deserialize_with = "null_default")]
    pub generated_at: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub channel_order: Vec<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub channels: HashMap<String, Channel>,
    #[serde(default, deserialize_with = "null_default")]
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Channel {
    /// 원어 이름(「千石ユノ -Sengoku Yuno-」) — 원문 면 이름 자리(fxtwitter 를 받기 전)
    #[serde(default, deserialize_with = "null_default")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub name_ko: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub handle: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub channel_url: Option<String>,
}

/// 예고 하나(계약 A `items[]`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Item {
    pub id: String,
    /// announced | upcoming | watching | live | end
    #[serde(default, deserialize_with = "null_default")]
    pub state: String,
    #[serde(default, deserialize_with = "null_default")]
    pub channel_key: String,
    /// 합동이면 참여 멤버(주 레인 제외). null → 빈 배열.
    #[serde(default, deserialize_with = "null_default")]
    pub collab_with: Vec<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub title_ko: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub url: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub scheduled_start: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub assumed_live: bool,
    /// 썸네일(YouTube maxresdefault)
    #[serde(default, deserialize_with = "null_default")]
    pub thumbnail: Option<String>,
    /// 회원 전용
    #[serde(default, deserialize_with = "null_default")]
    pub membership: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TweetsFile {
    #[serde(default, deserialize_with = "null_default")]
    pub generated_at: Option<String>,
    /// 멤버 key → 글 배열(최신이 뒤). 옛(v2.8) 단건 객체는 `[객체]` 로 감싼다
    /// (백엔드 `xtweet._as_list` 와 같은 하위호환 — SPEC 계약 I).
    #[serde(default, deserialize_with = "tweets_map")]
    pub tweets: HashMap<String, Vec<Tweet>>,
}

/// 멤버 개인 트윗 하나(계약 I).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Tweet {
    pub id: String,
    #[serde(default, deserialize_with = "null_default")]
    pub text: Option<String>,
    /// 번역. 비어 있으면 아직 번역 전(백엔드가 다음 tick 에 다시 시도한다).
    #[serde(default, deserialize_with = "null_default")]
    pub text_ko: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub url: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub expires_at: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub posted_at: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub received_at: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub reply_to_id: Option<String>,
    /// X 핸들(@ 없이). 트윗 시트 머리 줄에 쓴다.
    #[serde(default, deserialize_with = "null_default")]
    pub handle: Option<String>,
    /// 첨부 그림 주소(영상 · GIF 는 썸네일 주소)
    #[serde(default, deserialize_with = "null_default")]
    pub media: Vec<String>,
    /// 인용한 글
    #[serde(default, deserialize_with = "null_default")]
    pub quote: Option<Quote>,
}

/// 인용한 글(계약 I `quote`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Quote {
    #[serde(default, deserialize_with = "null_default")]
    pub text: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub text_ko: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub media: Vec<String>,
}

impl Preview {
    /// 멤버 순서 — 데이터에 없으면 `DEFAULT_ORDER`.
    pub fn order(&self) -> Vec<String> {
        if self.channel_order.is_empty() {
            DEFAULT_ORDER.iter().map(|s| s.to_string()).collect()
        } else {
            self.channel_order.clone()
        }
    }

    /// 표시 이름(name_ko) — 없으면 key.
    pub fn name_ko(&self, key: &str) -> String {
        self.channels
            .get(key)
            .and_then(|c| c.name_ko.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| key.to_string())
    }

    /// 채널 주소 — `channel_url`, 없으면 `https://www.youtube.com/@{handle}`.
    pub fn channel_url(&self, key: &str) -> Option<String> {
        let c = self.channels.get(key)?;
        c.channel_url
            .clone()
            .filter(|s| !s.is_empty())
            .or_else(|| c.handle.as_ref().map(|h| format!("https://www.youtube.com/@{h}")))
    }
}

impl Item {
    /// 이 방송에 나오는 멤버 = 주 레인 ∪ 합동(중복 제거, 순서 유지).
    pub fn members(&self) -> Vec<String> {
        let mut v = vec![self.channel_key.clone()];
        for m in &self.collab_with {
            if !v.contains(m) {
                v.push(m.clone());
            }
        }
        v
    }

    /// 표시 제목 — title_ko, 없으면 title, 둘 다 없으면 "(제목 미정)".
    pub fn display_title(&self) -> String {
        [&self.title_ko, &self.title]
            .into_iter()
            .flatten()
            .find(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| "(제목 미정)".to_string())
    }
}

impl Tweet {
    /// 번역이 있으면 Some(번역).
    pub fn translated(&self) -> Option<&str> {
        self.text_ko.as_deref().filter(|s| !s.trim().is_empty())
    }
}

pub fn parse_preview(s: &str) -> Result<Preview, String> {
    serde_json::from_str(s).map_err(|e| format!("preview.json 해석 실패: {e}"))
}

pub fn parse_tweets(s: &str) -> Result<TweetsFile, String> {
    serde_json::from_str(s).map_err(|e| format!("tweets.json 해석 실패: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixtures_parse() {
        let p = parse_preview(include_str!("../fixtures/preview.sample.json")).unwrap();
        assert!(!p.items.is_empty());
        assert!(p.items.iter().any(|i| i.state == "live"));
        let t = parse_tweets(include_str!("../fixtures/tweets.sample.json")).unwrap();
        assert!(!t.tweets.is_empty());
        // fixture 의 ritsu 는 옛 단건 객체 → 1건 배열로
        assert_eq!(t.tweets.get("ritsu").map(|v| v.len()), Some(1));
    }

    #[test]
    fn nulls_tolerated() {
        let p = parse_preview(r#"{"generated_at":null,"channel_order":null,"channels":{},"items":[{"id":"a","state":"live","channel_key":"yuno","collab_with":null,"title":null}]}"#).unwrap();
        assert_eq!(p.order().len(), 5);
        assert_eq!(p.items[0].members(), vec!["yuno".to_string()]);
        assert_eq!(p.items[0].display_title(), "(제목 미정)");
    }
}
