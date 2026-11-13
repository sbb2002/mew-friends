//! 시트 데이터 — tweets.json · preview.json → 말풍선에 보일 내용(순수 함수).
//! 원본 tweetlogic.js 의 시각 표기 · 글타래 묶기와 같은 규칙.

use chrono::{DateTime, Datelike, FixedOffset, Timelike, Utc};

use crate::model::{Preview, Quote, TweetsFile, short_name_ko};
use crate::util::parse_utc;

/// 번역 전 글에 쓰는 문구(웹과 같음).
pub const PENDING_TEXT: &str = "번역 준비 중…";
/// 시트에 올리는 트윗 최대 수(웹 MAX_THREAD 와 같음).
const MAX_TWEETS: usize = 50;
/// X Snowflake 기준 시각(ms).
const TW_EPOCH_MS: i64 = 1_288_834_974_657;

#[derive(Debug, Clone, PartialEq)]
pub struct PanelTweet {
    pub id: String,
    pub member: String,
    /// 번역, 없으면 "번역 준비 중…"
    pub text: String,
    /// 원문(일본어)
    pub orig: String,
    pub url: Option<String>,
    /// 표시 · 정렬 시각(UTC ms) — 게시 시각(posted_at → id 에서 계산) 우선, 없으면 받은 시각
    pub ms: Option<i64>,
    /// 게시 시각을 안다(모르면 받은 시각으로 「HH:MM 등록」)
    pub posted: bool,
    pub reply_to: Option<String>,
    pub name_ko: String,
    /// 원문 면 이름(fxtwitter 를 받기 전 — 채널 원어 이름)
    pub x_name: String,
    /// @ 없이
    pub handle: String,
    /// 첨부 그림 주소(영상은 썸네일)
    pub media: Vec<String>,
    pub quote: Option<Quote>,
}

impl PartialEq for Quote {
    fn eq(&self, o: &Quote) -> bool {
        self.text == o.text && self.text_ko == o.text_ko && self.media == o.media
    }
}

/// 한 덩어리(글타래 또는 단독 글) — 화면 위에서 아래 순서, 안쪽도 오래된 글이 위.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    /// 날짜 구분선 문구(마지막 글 기준, 「11월 13일 (금)」). 시각을 모르면 빈 문자열
    pub day: String,
    pub tweets: Vec<PanelTweet>,
}

/// 머리 줄 멤버 칩.
#[derive(Debug, Clone, PartialEq)]
pub struct Chip {
    pub key: String,
    /// 트윗이 있어 누를 수 있다
    pub enabled: bool,
}

/// 24시간 안 방송 예고(말풍선 아래 미니 예고 카드).
#[derive(Debug, Clone, PartialEq)]
pub struct Upcoming {
    pub title: String,
    /// 예정 시각(UTC ms)
    pub start_ms: i64,
    pub url: String,
    /// 작은 썸네일 주소(회원 전용이면 없음)
    pub thumb: Option<String>,
    pub collab: bool,
    pub membership: bool,
}

/// 도넛 타이머 모양(timetable.js updateRing).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ring {
    /// 1시간 넘게 남음 — 24시간 중 지난 비율(0~1)만큼 멤버 색으로 찬다
    Day(f32),
    /// 1시간 이내 — 남은 비율(0~1)만큼 노란 링이 남는다(12시부터 시계방향으로 비워짐)
    Min(f32),
    /// 시작 시각 지남(아직 방송 전) — 빈 링 + 「지각」
    Late,
}

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;
/// 시작 시각이 이만큼 지나도 방송이 안 잡히면 카드를 내린다
const LATE_KEEP_MS: i64 = 3 * HOUR_MS;

/// 남은 시간 → (링, 굵은 글자, 작은 글자). 「3시간 / 시작까지」 · 「25분 / 시작까지」 · 「지각」.
pub fn ring_of(left_ms: i64) -> (Ring, String, String) {
    if left_ms < 0 {
        return (Ring::Late, "지각".into(), String::new());
    }
    let min = (left_ms as f64 / 60000.0).round() as i64;
    if left_ms <= HOUR_MS {
        (Ring::Min(left_ms as f32 / HOUR_MS as f32), format!("{min}분"), "시작까지".into())
    } else {
        let pct = (1.0 - left_ms as f64 / DAY_MS as f64).clamp(0.0, 1.0);
        let pct = (pct * 100.0).round() / 100.0;
        (Ring::Day(pct as f32), format!("{}시간", min / 60), "시작까지".into())
    }
}

/// 「11/13(금) 20:00」 (KST)
pub fn when_label(ms: i64) -> String {
    const WD: [&str; 7] = ["월", "화", "수", "목", "금", "토", "일"];
    kst_of(ms)
        .map(|t| format!("{}/{}({}) {:02}:{:02}", t.month(), t.day(), WD[t.weekday().num_days_from_monday() as usize], t.hour(), t.minute()))
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelData {
    pub member: String,
    pub name_ko: String,
    /// 방송 중이면 (제목, 주소)
    pub live: Option<(String, String)>,
    /// 24시간 안 예고(가장 이른 것)
    pub upcoming: Option<Upcoming>,
    pub units: Vec<Unit>,
    pub chips: Vec<Chip>,
}

impl PanelData {
    /// 글 수
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.units.iter().map(|u| u.tweets.len()).sum()
    }
}

fn kst_of(ms: i64) -> Option<DateTime<FixedOffset>> {
    DateTime::<Utc>::from_timestamp_millis(ms).map(|d| d.with_timezone(&FixedOffset::east_opt(9 * 3600).unwrap()))
}

/// Snowflake id → 게시 시각(ms).
fn snowflake_ms(id: &str) -> Option<i64> {
    if id.len() < 15 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let v: u64 = id.parse().ok()?;
    Some((v >> 22) as i64 + TW_EPOCH_MS)
}

fn hm(ms: i64) -> String {
    kst_of(ms).map(|t| format!("{:02}:{:02}", t.hour(), t.minute())).unwrap_or_default()
}

/// 바닥 줄 시각 — 게시 시각 = 「HH:MM」, 받은 시각만 = 「HH:MM 등록」.
pub fn time_label(t: &PanelTweet) -> String {
    match (t.ms, t.posted) {
        (Some(ms), true) => hm(ms),
        (Some(ms), false) => format!("{} 등록", hm(ms)),
        _ => String::new(),
    }
}

/// 간격 문구: 「30분 뒤」 · 「7시간 31분 뒤」 · 「2일 뒤」.
pub fn gap_label(diff_ms: i64) -> String {
    let m = (diff_ms as f64 / 60000.0).round().max(0.0) as i64;
    if m < 60 {
        return format!("{m}분 뒤");
    }
    let (hh, mm) = (m / 60, m % 60);
    if hh < 24 {
        return if mm > 0 { format!("{hh}시간 {mm}분 뒤") } else { format!("{hh}시간 뒤") };
    }
    format!("{}일 뒤", hh / 24)
}

/// 「이어서」 줄 — 바로 위 글과의 간격, 모르면 「이어서 쓴 글」.
pub fn reply_label(t: &PanelTweet, prev: Option<&PanelTweet>) -> String {
    match (prev, t.posted, t.ms) {
        (Some(p), true, Some(ms)) => format!("이어서 · {}", gap_label(ms - p.ms.unwrap_or(0))),
        _ => "이어서 쓴 글".to_string(),
    }
}

/// 「11월 13일 (금)」
pub fn day_label(ms: i64) -> String {
    const WD: [&str; 7] = ["월", "화", "수", "목", "금", "토", "일"];
    kst_of(ms)
        .map(|t| format!("{}월 {}일 ({})", t.month(), t.day(), WD[t.weekday().num_days_from_monday() as usize]))
        .unwrap_or_default()
}

/// 글타래 묶기(tweetlogic.js chains): reply_to 가 목록 안 글이면 같은 덩어리(뿌리까지).
/// 덩어리 안 = 시각순, 덩어리 자리 = 가장 늦은 글 시각. 순환은 그 글을 혼자 둔다.
pub fn chains(list: Vec<PanelTweet>) -> Vec<Vec<PanelTweet>> {
    use std::collections::{HashMap, HashSet};
    let by_id: HashMap<&str, &PanelTweet> = list.iter().map(|t| (t.id.as_str(), t)).collect();
    let root_of = |m: &PanelTweet| -> String {
        let mut seen: HashSet<&str> = HashSet::from([m.id.as_str()]);
        let mut cur = m;
        loop {
            let Some(pid) = cur.reply_to.as_deref().filter(|p| by_id.contains_key(p)) else {
                return cur.id.clone();
            };
            if !seen.insert(pid) {
                return m.id.clone();
            }
            cur = by_id[pid];
        }
    };
    let roots: Vec<String> = list.iter().map(root_of).collect();
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<PanelTweet>> = HashMap::new();
    for (t, r) in list.into_iter().zip(roots) {
        if !groups.contains_key(&r) {
            order.push(r.clone());
        }
        groups.entry(r).or_default().push(t);
    }
    let t_of = |t: &PanelTweet| t.ms.unwrap_or(0);
    let mut units: Vec<Vec<PanelTweet>> = order
        .into_iter()
        .filter_map(|k| groups.remove(&k))
        .map(|mut u| {
            u.sort_by_key(t_of);
            u
        })
        .collect();
    units.sort_by_key(|u| u.last().map(t_of).unwrap_or(0));
    units
}

/// 순수 함수 — 시트에 보일 내용을 만든다.
pub fn build_data(member: &str, preview: &Preview, tweets: &TweetsFile, now: DateTime<Utc>) -> PanelData {
    let name_of = |k: &str| {
        let n = preview.name_ko(k);
        if n == k { short_name_ko(k).to_string() } else { n }
    };
    let name_ko = name_of(member);

    // 방송 중: 이 멤버가 나오는 첫 live 항목(합동 포함).
    let live = preview
        .items
        .iter()
        .find(|i| i.state == "live" && i.members().iter().any(|m| m == member))
        .map(|item| {
            let url = item
                .url
                .clone()
                .filter(|u| !u.trim().is_empty())
                .or_else(|| preview.channel_url(&item.channel_key))
                .unwrap_or_else(|| "https://www.youtube.com/".to_string());
            (item.display_title(), url)
        });

    // 24시간 안 예고: 이 멤버가 나오는(합동 포함) 예정 방송 중 가장 이른 것. 시작 시각이 지났는데 아직 방송 전이면 3시간까지 「지각」.
    let now_ms = now.timestamp_millis();
    let upcoming = preview
        .items
        .iter()
        .filter(|i| matches!(i.state.as_str(), "upcoming" | "watching") && i.members().iter().any(|m| m == member))
        .filter_map(|i| Some((i, i.scheduled_start.as_deref().and_then(parse_utc)?.timestamp_millis())))
        .filter(|(_, st)| *st - now_ms <= DAY_MS && now_ms - *st <= LATE_KEEP_MS)
        .min_by_key(|(_, st)| *st)
        .map(|(i, st)| Upcoming {
            title: i.display_title(),
            start_ms: st,
            url: i
                .url
                .clone()
                .filter(|u| !u.trim().is_empty())
                .or_else(|| preview.channel_url(&i.channel_key))
                .unwrap_or_else(|| "https://www.youtube.com/".to_string()),
            thumb: i
                .thumbnail
                .clone()
                .filter(|_| !i.membership)
                .filter(|u| !u.is_empty())
                .map(|u| u.replace("maxresdefault", "mqdefault")),
            collab: i.members().len() > 1,
            membership: i.membership,
        });

    let alive = |k: &str| -> Vec<&crate::model::Tweet> {
        tweets
            .tweets
            .get(k)
            .map(|v| {
                v.iter()
                    .filter(|t| t.expires_at.as_deref().and_then(parse_utc).is_none_or(|exp| exp > now))
                    .collect()
            })
            .unwrap_or_default()
    };

    let ch = preview.channels.get(member);
    let ch_handle = ch.and_then(|c| c.handle.clone()).unwrap_or_default();
    let x_name = ch.and_then(|c| c.name.clone()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| name_ko.clone());
    let list = alive(member);
    let skip = list.len().saturating_sub(MAX_TWEETS);
    let has_quote = |q: &Quote| {
        q.text.as_deref().is_some_and(|s| !s.trim().is_empty()) || q.text_ko.as_deref().is_some_and(|s| !s.trim().is_empty())
    };
    let flat: Vec<PanelTweet> = list
        .into_iter()
        .skip(skip)
        .map(|t| {
            let posted = t.posted_at.as_deref().and_then(parse_utc).map(|d| d.timestamp_millis()).or_else(|| snowflake_ms(&t.id));
            let received = t.received_at.as_deref().and_then(parse_utc).map(|d| d.timestamp_millis());
            PanelTweet {
                id: t.id.clone(),
                member: member.to_string(),
                text: t.translated().unwrap_or(PENDING_TEXT).to_string(),
                orig: t.text.clone().unwrap_or_default(),
                url: Some(
                    t.url
                        .clone()
                        .filter(|u| !u.trim().is_empty())
                        .unwrap_or_else(|| format!("https://x.com/i/status/{}", t.id)),
                ),
                ms: posted.or(received),
                posted: posted.is_some(),
                reply_to: t.reply_to_id.clone().filter(|s| !s.is_empty()),
                name_ko: name_ko.clone(),
                x_name: x_name.clone(),
                handle: t.handle.clone().filter(|h| !h.is_empty()).unwrap_or_else(|| ch_handle.clone()),
                media: t.media.iter().filter(|u| !u.is_empty()).cloned().collect(),
                quote: t.quote.clone().filter(has_quote),
            }
        })
        .collect();
    let units = chains(flat)
        .into_iter()
        .map(|u| Unit { day: u.last().and_then(|t| t.ms).map(day_label).unwrap_or_default(), tweets: u })
        .collect();

    let chips = preview
        .order()
        .into_iter()
        .map(|k| Chip { enabled: k == member || !alive(&k).is_empty(), key: k })
        .collect();

    PanelData { member: member.to_string(), name_ko, live, upcoming, units, chips }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{parse_preview, parse_tweets};

    fn at(s: &str) -> DateTime<Utc> {
        parse_utc(s).unwrap()
    }

    const NOW: &str = "2026-11-13T12:00:00Z";

    fn texts(d: &PanelData) -> Vec<Vec<&str>> {
        d.units.iter().map(|u| u.tweets.iter().map(|t| t.text.as_str()).collect()).collect()
    }

    #[test]
    fn expired_hidden_oldest_first() {
        let t = parse_tweets(
            r#"{"tweets":{"yuno":[
              {"id":"1","text_ko":"오래된","expires_at":"2026-11-14T00:00:00Z","received_at":"2026-11-10T00:00:00Z"},
              {"id":"2","text_ko":"만료","expires_at":"2026-11-12T00:00:00Z","posted_at":"2026-11-11T00:00:00Z"},
              {"id":"3","text_ko":"새 글","expires_at":"2026-11-14T00:00:00Z","posted_at":"2026-11-13T11:00:00Z"}
            ]}}"#,
        )
        .unwrap();
        let d = build_data("yuno", &Preview::default(), &t, at(NOW));
        assert_eq!(texts(&d), vec![vec!["오래된"], vec!["새 글"]]);
        assert_eq!(time_label(&d.units[1].tweets[0]), "20:00");
        assert_eq!(d.units[1].day, "11월 13일 (금)");
        assert_eq!(time_label(&d.units[0].tweets[0]), "09:00 등록");
    }

    #[test]
    fn untranslated_snowflake_and_url() {
        let t = parse_tweets(
            r#"{"tweets":{"yuno":[
              {"id":"2096552878769152326","text":"あ"},
              {"id":"7","text_ko":"x"}
            ]}}"#,
        )
        .unwrap();
        let d = build_data("yuno", &Preview::default(), &t, at(NOW));
        let all: Vec<&PanelTweet> = d.units.iter().flat_map(|u| &u.tweets).collect();
        assert_eq!(all[0].text, "x");
        assert_eq!(time_label(all[0]), "");
        assert_eq!(all[1].text, PENDING_TEXT);
        assert_eq!(all[1].orig, "あ");
        assert!(all[1].posted && all[1].ms.is_some());
        assert_eq!(all[0].url.as_deref(), Some("https://x.com/i/status/7"));
    }

    #[test]
    fn threads_group_and_rise() {
        let t = parse_tweets(
            r#"{"tweets":{"yuno":[
              {"id":"1","text_ko":"원글","posted_at":"2026-11-13T01:00:00Z"},
              {"id":"2","text_ko":"단독","posted_at":"2026-11-13T02:00:00Z"},
              {"id":"3","text_ko":"답글","posted_at":"2026-11-13T03:30:00Z","reply_to_id":"1"},
              {"id":"4","text_ko":"순환","posted_at":"2026-11-13T00:00:00Z","reply_to_id":"4"}
            ]}}"#,
        )
        .unwrap();
        let d = build_data("yuno", &Preview::default(), &t, at(NOW));
        assert_eq!(texts(&d), vec![vec!["순환"], vec!["단독"], vec!["원글", "답글"]]);
        let u = &d.units[2].tweets;
        assert_eq!(reply_label(&u[1], Some(&u[0])), "이어서 · 2시간 30분 뒤");
        assert_eq!(gap_label(30 * 60000), "30분 뒤");
        assert_eq!(gap_label(3 * 86400000), "3일 뒤");
    }

    #[test]
    fn media_and_quote_kept() {
        let t = parse_tweets(
            r#"{"tweets":{"yuno":[
              {"id":"1","text":"写真","text_ko":"사진","media":["https://pbs.twimg.com/media/a.jpg",""],
               "quote":{"text":"引用","text_ko":"인용","media":["https://pbs.twimg.com/amplify_video_thumb/1/img/b.jpg"]}},
              {"id":"2","text_ko":"빈 인용","quote":{"text":"","media":[]}},
              {"id":"3","text_ko":"인용 없음","quote":null,"media":null}
            ]}}"#,
        )
        .unwrap();
        let p = parse_preview(r#"{"channels":{"yuno":{"name":"千石ユノ","name_ko":"센고쿠 유노"}},"items":[]}"#).unwrap();
        let d = build_data("yuno", &p, &t, at(NOW));
        let all: Vec<&PanelTweet> = d.units.iter().flat_map(|u| &u.tweets).collect();
        let a = all.iter().find(|t| t.id == "1").unwrap();
        assert_eq!(a.media, vec!["https://pbs.twimg.com/media/a.jpg".to_string()]);
        assert_eq!(a.quote.as_ref().unwrap().text_ko.as_deref(), Some("인용"));
        assert_eq!(a.x_name, "千石ユノ");
        assert!(all.iter().find(|t| t.id == "2").unwrap().quote.is_none());
        assert!(all.iter().find(|t| t.id == "3").unwrap().media.is_empty());
    }

    #[test]
    fn limit_50_keeps_newest() {
        let items: Vec<String> = (0..60)
            .map(|i| format!(r#"{{"id":"{i}","text_ko":"t{i}","received_at":"2026-11-13T00:{:02}:00Z"}}"#, i % 60))
            .collect();
        let t = parse_tweets(&format!(r#"{{"tweets":{{"yuno":[{}]}}}}"#, items.join(","))).unwrap();
        let d = build_data("yuno", &Preview::default(), &t, at(NOW));
        assert_eq!(d.len(), 50);
        assert_eq!(d.units.last().unwrap().tweets[0].text, "t59");
    }

    #[test]
    fn live_and_collab_and_fallback_url() {
        let p = parse_preview(
            r#"{"channels":{"yuno":{"name_ko":"유노","handle":"yuno_h"}},"items":[
              {"id":"a","state":"upcoming","channel_key":"yuno","title_ko":"예고"},
              {"id":"b","state":"live","channel_key":"nonoka","collab_with":["yuno"],"title_ko":"합동 방송"}
            ]}"#,
        )
        .unwrap();
        let t = TweetsFile::default();
        let d = build_data("yuno", &p, &t, at(NOW));
        assert_eq!(d.live, Some(("합동 방송".to_string(), "https://www.youtube.com/".to_string())));
        assert_eq!(d.name_ko, "유노");
        let p3 = parse_preview(
            r#"{"channels":{"yuno":{"handle":"yuno_h"}},"items":[{"id":"a","state":"live","channel_key":"yuno","url":null,"title":"T"}]}"#,
        )
        .unwrap();
        assert_eq!(build_data("yuno", &p3, &t, at(NOW)).live, Some(("T".to_string(), "https://www.youtube.com/@yuno_h".to_string())));
    }

    #[test]
    fn upcoming_within_24h() {
        let p = parse_preview(
            r#"{"channels":{"yuno":{"handle":"yuno_h"}},"items":[
              {"id":"far","state":"upcoming","channel_key":"yuno","title_ko":"먼 예고","scheduled_start":"2026-11-15T12:00:00Z"},
              {"id":"b","state":"upcoming","channel_key":"nonoka","collab_with":["yuno"],"title_ko":"합동","scheduled_start":"2026-11-13T20:00:00Z",
               "thumbnail":"https://i.ytimg.com/vi/X/maxresdefault.jpg","url":"https://www.youtube.com/watch?v=X"},
              {"id":"c","state":"upcoming","channel_key":"yuno","title_ko":"더 늦은","scheduled_start":"2026-11-14T03:00:00Z"},
              {"id":"old","state":"upcoming","channel_key":"yuno","title_ko":"한참 지남","scheduled_start":"2026-11-13T07:00:00Z"}
            ]}"#,
        )
        .unwrap();
        let d = build_data("yuno", &p, &TweetsFile::default(), at(NOW));
        let u = d.upcoming.unwrap();
        assert_eq!(u.title, "합동");
        assert!(u.collab);
        assert_eq!(u.thumb.as_deref(), Some("https://i.ytimg.com/vi/X/mqdefault.jpg"));
        // 2026-11-13T20:00Z = KST 11/14(토) 05:00
        assert_eq!(when_label(u.start_ms), "11/14(토) 05:00");
        assert!(build_data("miyako", &p, &TweetsFile::default(), at(NOW)).upcoming.is_none());
        // 도넛 규칙
        assert_eq!(ring_of(8 * 3_600_000), (Ring::Day(0.67), "8시간".into(), "시작까지".into()));
        assert_eq!(ring_of(25 * 60000).1, "25분");
        assert!(matches!(ring_of(30 * 60000).0, Ring::Min(p) if (p - 0.5).abs() < 1e-3));
        assert_eq!(ring_of(-60000), (Ring::Late, "지각".into(), String::new()));
    }

    #[test]
    fn chips_and_handle() {
        let p = parse_preview(r#"{"channels":{"yuno":{"handle":"yuno_h"}},"items":[]}"#).unwrap();
        let t = parse_tweets(r#"{"tweets":{"yuno":[{"id":"1","text_ko":"a"}],"arale":[{"id":"2","text_ko":"b","handle":"arale_x"}]}}"#).unwrap();
        let d = build_data("miyako", &p, &t, at(NOW));
        assert_eq!(d.name_ko, "미야코");
        assert!(d.units.is_empty());
        let en: Vec<(&str, bool)> = d.chips.iter().map(|c| (c.key.as_str(), c.enabled)).collect();
        assert_eq!(en, vec![("arale", true), ("yuno", true), ("nonoka", false), ("ritsu", false), ("miyako", true)]);
        assert_eq!(build_data("yuno", &p, &t, at(NOW)).units[0].tweets[0].handle, "yuno_h");
        assert_eq!(build_data("arale", &p, &t, at(NOW)).units[0].tweets[0].handle, "arale_x");
    }
}
