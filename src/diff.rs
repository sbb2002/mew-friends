//! 알림 판정 — 순수 함수. `이전에 본 것 + 새 JSON → 알림 목록`.
//! 명세: docs/IMPL_SPEC.md §4. [담당: 작업 A]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{Item, Preview, Tweet, TweetsFile};
use crate::util;

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

/// 목록이 `SEEN_CAP` 을 넘으면 앞(오래된 것)부터 버린다.
fn cap(v: &mut Vec<String>) {
    if v.len() > SEEN_CAP {
        let n = v.len() - SEEN_CAP;
        v.drain(..n);
    }
}

/// 공백만 있는 값은 없는 것으로 본다.
fn non_empty(s: &Option<String>) -> Option<String> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// 방송 주소: item.url → 채널 주소 → 유튜브 홈. 주소를 지어내지 않는다.
fn live_url(p: &Preview, it: &Item) -> String {
    non_empty(&it.url)
        .or_else(|| p.channel_url(&it.channel_key))
        .unwrap_or_else(|| "https://www.youtube.com/".to_string())
}

pub fn diff_preview(seen: &mut Seen, p: &Preview) -> Vec<Notice> {
    let lives = || p.items.iter().filter(|i| i.state == "live");

    // 첫 실행: 이미 live 인 방송은 「본 것」으로만 넣고 알리지 않는다.
    if !seen.preview_ready {
        for it in lives() {
            if !seen.live_ids.contains(&it.id) {
                seen.live_ids.push(it.id.clone());
            }
        }
        cap(&mut seen.live_ids);
        seen.preview_ready = true;
        return Vec::new();
    }

    let mut out = Vec::new();
    for it in lives() {
        if seen.live_ids.contains(&it.id) {
            continue;
        }
        seen.live_ids.push(it.id.clone());
        out.push(Notice::LiveStarted {
            item_id: it.id.clone(),
            members: it.members(),
            title: it.display_title(),
            url: live_url(p, it),
        });
    }
    cap(&mut seen.live_ids);
    out
}

fn is_expired(tw: &Tweet, now: DateTime<Utc>) -> bool {
    // 못 읽는 시각이면 만료 아님.
    tw.expires_at.as_deref().and_then(util::parse_utc).is_some_and(|e| e <= now)
}

fn tweet_url(tw: &Tweet) -> String {
    non_empty(&tw.url).unwrap_or_else(|| format!("https://x.com/i/status/{}", tw.id))
}

fn new_tweet_notice(member: &str, tw: &Tweet, text_ko: &str) -> Notice {
    Notice::NewTweet {
        member: member.to_string(),
        tweet_id: tw.id.clone(),
        text_ko: text_ko.to_string(),
        url: tweet_url(tw),
    }
}

pub fn diff_tweets(seen: &mut Seen, t: &TweetsFile, now: DateTime<Utc>) -> Vec<Notice> {
    // 멤버 key 를 정렬해 결과 순서를 결정적으로 만든다.
    let mut keys: Vec<&String> = t.tweets.keys().collect();
    keys.sort();
    let items: Vec<(&str, &Tweet)> = keys
        .iter()
        .flat_map(|k| t.tweets[*k].iter().map(move |tw| (k.as_str(), tw)))
        .collect();

    // 첫 실행: 모든 글을 본 것으로 넣고 알리지 않는다.
    if !seen.tweets_ready {
        for (_, tw) in &items {
            if !seen.tweet_ids.contains(&tw.id) {
                seen.tweet_ids.push(tw.id.clone());
            }
        }
        seen.pending_tl.clear();
        seen.tweets_ready = true;
        cap(&mut seen.tweet_ids);
        return Vec::new();
    }

    let mut out = Vec::new();
    for (key, tw) in &items {
        let id = tw.id.as_str();

        if is_expired(tw, now) {
            // 만료된 글은 알리지 않는다(번역이 늦게 와도 마찬가지).
            seen.pending_tl.retain(|x| x != id);
            if !seen.tweet_ids.iter().any(|x| x == id) {
                seen.tweet_ids.push(id.to_string());
            }
            continue;
        }

        if !seen.tweet_ids.iter().any(|x| x == id) {
            // 새 글.
            seen.tweet_ids.push(id.to_string());
            match tw.translated() {
                Some(tl) => out.push(new_tweet_notice(key, tw, tl)),
                None => {
                    if !seen.pending_tl.iter().any(|x| x == id) {
                        seen.pending_tl.push(id.to_string());
                    }
                }
            }
        } else if seen.pending_tl.iter().any(|x| x == id) {
            // 번역을 기다리던 글에 번역이 생겼으면 알린다.
            if let Some(tl) = tw.translated() {
                out.push(new_tweet_notice(key, tw, tl));
                seen.pending_tl.retain(|x| x != id);
            }
        }
    }

    // 이번 파일에서 사라진 글은 기다리지 않는다.
    seen.pending_tl.retain(|x| items.iter().any(|(_, tw)| tw.id == *x));
    cap(&mut seen.tweet_ids);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{parse_preview, parse_tweets};

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-13T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn preview(items: &str) -> Preview {
        let s = format!(
            r#"{{"channels":{{"yuno":{{"name_ko":"유노","handle":"yuno_h"}}}},"items":[{items}]}}"#
        );
        parse_preview(&s).unwrap()
    }

    fn live_item(id: &str, extra: &str) -> String {
        format!(r#"{{"id":"{id}","state":"live","channel_key":"yuno","title":"방송 {id}"{extra}}}"#)
    }

    #[test]
    fn first_run_silent_then_no_repeat() {
        let mut seen = Seen::default();
        let p = preview(&live_item("a", r#","url":"https://y/a""#));
        assert!(diff_preview(&mut seen, &p).is_empty());
        assert!(diff_preview(&mut seen, &p).is_empty());
    }

    #[test]
    fn new_live_notifies_once() {
        let mut seen = Seen::default();
        let p0 = preview("");
        assert!(diff_preview(&mut seen, &p0).is_empty());
        let p1 = preview(&live_item("b", r#","url":"https://y/b""#));
        let n = diff_preview(&mut seen, &p1);
        assert_eq!(n.len(), 1);
        assert!(matches!(&n[0], Notice::LiveStarted { item_id, .. } if item_id == "b"));
        assert!(diff_preview(&mut seen, &p1).is_empty());
    }

    #[test]
    fn collab_members_main_first() {
        let mut seen = Seen::default();
        diff_preview(&mut seen, &preview(""));
        let p = preview(&live_item("c", r#","collab_with":["ritsu","yuno"],"url":"https://y/c""#));
        let n = diff_preview(&mut seen, &p);
        match &n[0] {
            Notice::LiveStarted { members, .. } => {
                assert_eq!(members, &vec!["yuno".to_string(), "ritsu".to_string()])
            }
            _ => panic!("LiveStarted 여야 함"),
        }
    }

    #[test]
    fn live_url_falls_back_to_channel() {
        let mut seen = Seen::default();
        diff_preview(&mut seen, &preview(""));
        let p = preview(&live_item("d", ""));
        match &diff_preview(&mut seen, &p)[0] {
            Notice::LiveStarted { url, .. } => assert_eq!(url, "https://www.youtube.com/@yuno_h"),
            _ => panic!(),
        }
        // 채널도 없으면 유튜브 홈
        let mut seen = Seen::default();
        diff_preview(&mut seen, &Preview::default());
        let p = parse_preview(&format!(r#"{{"items":[{}]}}"#, live_item("e", ""))).unwrap();
        match &diff_preview(&mut seen, &p)[0] {
            Notice::LiveStarted { url, .. } => assert_eq!(url, "https://www.youtube.com/"),
            _ => panic!(),
        }
    }

    fn tweets(list: &str) -> TweetsFile {
        parse_tweets(&format!(r#"{{"tweets":{{"yuno":[{list}]}}}}"#)).unwrap()
    }

    fn tw(id: &str, tl: Option<&str>, expires: &str) -> String {
        let tl = tl.map(|t| format!(r#","text_ko":"{t}""#)).unwrap_or_default();
        format!(r#"{{"id":"{id}","url":"https://x/{id}","expires_at":"{expires}"{tl}}}"#)
    }

    const FUTURE: &str = "2026-11-15T00:00:00Z";
    const PAST: &str = "2026-11-12T00:00:00Z";

    #[test]
    fn tweets_first_run_silent_and_translated_new_once() {
        let mut seen = Seen::default();
        assert!(diff_tweets(&mut seen, &tweets(&tw("1", None, FUTURE)), now()).is_empty());
        let t = tweets(&format!("{},{}", tw("1", None, FUTURE), tw("2", Some("안녕"), FUTURE)));
        let n = diff_tweets(&mut seen, &t, now());
        assert_eq!(n.len(), 1);
        assert!(matches!(&n[0], Notice::NewTweet { tweet_id, text_ko, .. } if tweet_id == "2" && text_ko == "안녕"));
        assert!(diff_tweets(&mut seen, &t, now()).is_empty());
    }

    #[test]
    fn tweet_translation_later_notifies_then_stops() {
        let mut seen = Seen::default();
        assert!(diff_tweets(&mut seen, &tweets(""), now()).is_empty());
        // 번역 없이 등장 → 0건, 대기
        let t0 = tweets(&tw("3", None, FUTURE));
        assert!(diff_tweets(&mut seen, &t0, now()).is_empty());
        assert_eq!(seen.pending_tl, vec!["3".to_string()]);
        // 번역이 생김 → 1건
        let t1 = tweets(&tw("3", Some("번역"), FUTURE));
        assert_eq!(diff_tweets(&mut seen, &t1, now()).len(), 1);
        assert!(seen.pending_tl.is_empty());
        // 그다음 → 0건
        assert!(diff_tweets(&mut seen, &t1, now()).is_empty());
    }

    #[test]
    fn expired_before_translation_never_notifies() {
        let mut seen = Seen::default();
        diff_tweets(&mut seen, &tweets(""), now());
        diff_tweets(&mut seen, &tweets(&tw("4", None, PAST)), now());
        assert!(seen.pending_tl.is_empty());
        let t = tweets(&tw("4", Some("늦은 번역"), PAST));
        assert!(diff_tweets(&mut seen, &t, now()).is_empty());
        assert!(diff_tweets(&mut seen, &t, now()).is_empty());
    }

    #[test]
    fn pending_cleared_when_gone_from_file() {
        let mut seen = Seen::default();
        diff_tweets(&mut seen, &tweets(""), now());
        diff_tweets(&mut seen, &tweets(&tw("5", None, FUTURE)), now());
        assert_eq!(seen.pending_tl.len(), 1);
        // 파일에서 사라짐
        assert!(diff_tweets(&mut seen, &tweets(""), now()).is_empty());
        assert!(seen.pending_tl.is_empty());
        // 나중에 번역이 와도 알리지 않는다(사라진 글)
        let t = tweets(&tw("5", Some("뒤늦은 번역"), FUTURE));
        assert!(diff_tweets(&mut seen, &t, now()).is_empty());
    }

    #[test]
    fn tweet_url_fallback() {
        let mut seen = Seen::default();
        diff_tweets(&mut seen, &tweets(""), now());
        let t = tweets(r#"{"id":"6","text_ko":"x","expires_at":"2026-11-15T00:00:00Z"}"#);
        match &diff_tweets(&mut seen, &t, now())[0] {
            Notice::NewTweet { url, .. } => assert_eq!(url, "https://x.com/i/status/6"),
            _ => panic!(),
        }
    }

    #[test]
    fn seen_cap_keeps_length_and_drops_oldest() {
        let mut seen = Seen {
            preview_ready: true,
            tweets_ready: true,
            tweet_ids: (0..SEEN_CAP).map(|i| format!("old{i}")).collect(),
            ..Seen::default()
        };
        let t = tweets(&tw("new", None, FUTURE));
        diff_tweets(&mut seen, &t, now());
        assert_eq!(seen.tweet_ids.len(), SEEN_CAP);
        assert_eq!(seen.tweet_ids[0], "old1");
        assert_eq!(seen.tweet_ids.last().unwrap(), "new");
    }

    #[test]
    fn live_ids_cap() {
        let mut seen = Seen {
            preview_ready: true,
            live_ids: (0..SEEN_CAP).map(|i| format!("old{i}")).collect(),
            ..Seen::default()
        };
        let p = preview(&live_item("fresh", ""));
        assert_eq!(diff_preview(&mut seen, &p).len(), 1);
        assert_eq!(seen.live_ids.len(), SEEN_CAP);
        assert_eq!(seen.live_ids.last().unwrap(), "fresh");
    }

    #[test]
    fn seen_json_roundtrip() {
        let seen = Seen {
            preview_ready: true,
            tweets_ready: true,
            live_ids: vec!["a".into()],
            tweet_ids: vec!["b".into()],
            pending_tl: vec!["c".into()],
        };
        let s = serde_json::to_string(&seen).unwrap();
        let back: Seen = serde_json::from_str(&s).unwrap();
        assert_eq!(seen, back);
        let empty: Seen = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Seen::default());
    }
}
