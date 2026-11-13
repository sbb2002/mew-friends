//! 내려받기 — 트윗 그림(pbs.twimg.com) · 원문 정보(fxtwitter). 명세: docs/IMPL_SPEC.md §8-4.
//!
//! 일꾼 스레드 4개가 받아 해독까지 하고, 메인 스레드는 `drain()` 으로 꺼내 캐시에 넣는다.
//! 다 받으면 등록된 창(말풍선 · 영상 창)에 `WM_APP_NET` 을 붙여 다시 그리게 한다.
//! 같은 주소는 한 번만 받는다(실패도 기억 — 앱을 다시 켜면 다시 시도).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tiny_skia::Pixmap;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// 받은 것이 있다는 알림(창 메시지).
pub const WM_APP_NET: u32 = WM_APP + 20;
/// 그림 긴 변 상한(메모리 · 그리기 비용).
const MAX_SIDE: u32 = 1200;
const UA: &str = "mew-friends";

/// 원문 영상 하나.
#[derive(Debug, Clone, PartialEq)]
pub struct FxVideo {
    pub thumb: String,
    pub mp4: String,
    pub w: u32,
    pub h: u32,
    /// GIF(반복 재생 · 소리 없음)
    pub gif: bool,
}

/// fxtwitter 로 받은 원문 정보(X 카드 면에 쓴다).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Fx {
    pub name: String,
    pub handle: String,
    pub avatar: Option<String>,
    pub likes: u64,
    pub replies: u64,
    pub retweets: u64,
    /// 게시 시각(UTC ms)
    pub created_ms: Option<i64>,
    pub videos: Vec<FxVideo>,
    pub quote_name: Option<String>,
    pub quote_handle: Option<String>,
    pub quote_avatar: Option<String>,
    pub quote_videos: Vec<FxVideo>,
}

pub enum Slot<T> {
    Loading,
    Ready(Rc<T>),
    Failed,
}

enum Job {
    Img(String),
    Fx(String),
}

enum Done {
    Img(String, Option<Pixmap>),
    Fx(String, Option<Fx>),
}

struct Net {
    jobs: Sender<Job>,
    done: Receiver<Done>,
    imgs: HashMap<String, Slot<Pixmap>>,
    fxs: HashMap<String, Slot<Fx>>,
}

/// 알림 받을 창(HWND 값). 일꾼 스레드에서 읽는다.
static TARGETS: Mutex<Vec<isize>> = Mutex::new(Vec::new());

thread_local! {
    static NET: RefCell<Option<Net>> = const { RefCell::new(None) };
}

/// 받은 것이 생기면 이 창에 알린다.
pub fn register(hwnd: HWND) {
    if let Ok(mut t) = TARGETS.lock() {
        let v = hwnd.0 as isize;
        if !t.contains(&v) {
            t.push(v);
        }
    }
}

pub fn unregister(hwnd: HWND) {
    if let Ok(mut t) = TARGETS.lock() {
        t.retain(|v| *v != hwnd.0 as isize);
    }
}

fn notify() {
    let targets = TARGETS.lock().map(|t| t.clone()).unwrap_or_default();
    for h in targets {
        unsafe {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_APP_NET, WPARAM(0), LPARAM(0));
        }
    }
}

fn with_net<T>(f: impl FnOnce(&mut Net) -> T) -> T {
    NET.with(|c| {
        let mut g = c.borrow_mut();
        let net = g.get_or_insert_with(start);
        f(net)
    })
}

fn start() -> Net {
    let (jtx, jrx) = channel::<Job>();
    let (dtx, drx) = channel::<Done>();
    let jrx = Arc::new(Mutex::new(jrx));
    for _ in 0..4 {
        let jrx = Arc::clone(&jrx);
        let dtx = dtx.clone();
        std::thread::spawn(move || {
            let agent: ureq::Agent =
                ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into();
            loop {
                let job = match jrx.lock() {
                    Ok(rx) => match rx.recv() {
                        Ok(j) => j,
                        Err(_) => return,
                    },
                    Err(_) => return,
                };
                let done = match job {
                    Job::Img(url) => {
                        let pm = fetch_img(&agent, &url);
                        if pm.is_none() {
                            eprintln!("[net] 그림 실패: {url}");
                        }
                        Done::Img(url, pm)
                    }
                    Job::Fx(id) => {
                        let fx = fetch_fx(&agent, &id);
                        if fx.is_none() {
                            eprintln!("[net] fxtwitter 실패: {id}");
                        }
                        Done::Fx(id, fx)
                    }
                };
                if dtx.send(done).is_err() {
                    return;
                }
                notify();
            }
        });
    }
    Net { jobs: jtx, done: drx, imgs: HashMap::new(), fxs: HashMap::new() }
}

/// 다 받은 것을 캐시에 넣는다. 새로 들어온 게 있으면 true.
pub fn drain() -> bool {
    with_net(|n| {
        let mut any = false;
        while let Ok(d) = n.done.try_recv() {
            any = true;
            match d {
                Done::Img(u, Some(pm)) => {
                    n.imgs.insert(u, Slot::Ready(Rc::new(pm)));
                }
                Done::Img(u, None) => {
                    n.imgs.insert(u, Slot::Failed);
                }
                Done::Fx(id, Some(fx)) => {
                    n.fxs.insert(id, Slot::Ready(Rc::new(fx)));
                }
                Done::Fx(id, None) => {
                    n.fxs.insert(id, Slot::Failed);
                }
            }
        }
        any
    })
}

/// 그림(작은 크기 주소로). 없으면 받기를 걸고 None.
pub fn image(url: &str) -> Option<Rc<Pixmap>> {
    let url = small_url(url);
    with_net(|n| match n.imgs.get(&url) {
        Some(Slot::Ready(p)) => Some(Rc::clone(p)),
        Some(_) => None,
        None => {
            n.imgs.insert(url.clone(), Slot::Loading);
            let _ = n.jobs.send(Job::Img(url));
            None
        }
    })
}

/// 이미 받은 그림만(받기를 걸지 않는다) — 배치에서 1칸 비율을 정할 때.
pub fn image_ready(url: &str) -> Option<Rc<Pixmap>> {
    let url = small_url(url);
    with_net(|n| match n.imgs.get(&url) {
        Some(Slot::Ready(p)) => Some(Rc::clone(p)),
        _ => None,
    })
}

/// 원문 정보. 없으면 받기를 건다.
pub fn fx(id: &str) -> Slot<Fx> {
    with_net(|n| match n.fxs.get(id) {
        Some(Slot::Ready(f)) => Slot::Ready(Rc::clone(f)),
        Some(Slot::Failed) => Slot::Failed,
        Some(Slot::Loading) => Slot::Loading,
        None => {
            n.fxs.insert(id.to_string(), Slot::Loading);
            let _ = n.jobs.send(Job::Fx(id.to_string()));
            Slot::Loading
        }
    })
}

/// 이미 받은 원문 정보만(받기를 걸지 않는다).
pub fn fx_ready(id: &str) -> Option<Rc<Fx>> {
    with_net(|n| match n.fxs.get(id) {
        Some(Slot::Ready(f)) => Some(Rc::clone(f)),
        _ => None,
    })
}

/// pbs.twimg.com 그림은 작은 크기(name=small)로 요청한다(tweetlogic smallUrl). 다른 주소는 그대로.
pub fn small_url(u: &str) -> String {
    if !u.starts_with("https://pbs.twimg.com/") || u.contains("/profile_images/") {
        return u.to_string();
    }
    if let Some(i) = u.find("name=") {
        let end = u[i..].find('&').map(|e| i + e).unwrap_or(u.len());
        return format!("{}name=small{}", &u[..i], &u[end..]);
    }
    format!("{u}{}name=small", if u.contains('?') { "&" } else { "?" })
}

/// 영상 썸네일 주소인가(tweetlogic isVideoUrl).
pub fn is_video_url(u: &str) -> bool {
    u.contains("video_thumb") || u.contains("amplify")
}

fn fetch_img(agent: &ureq::Agent, url: &str) -> Option<Pixmap> {
    let bytes = agent
        .get(url)
        .header("User-Agent", UA)
        .call()
        .ok()?
        .body_mut()
        .with_config()
        .limit(20 * 1024 * 1024)
        .read_to_vec()
        .ok()?;
    let mut img = image::load_from_memory(&bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    if w.max(h) > MAX_SIDE {
        let k = MAX_SIDE as f32 / w.max(h) as f32;
        img = image::imageops::resize(
            &img,
            ((w as f32 * k).round() as u32).max(1),
            ((h as f32 * k).round() as u32).max(1),
            image::imageops::FilterType::Triangle,
        );
    }
    let (w, h) = img.dimensions();
    let mut pm = Pixmap::new(w, h)?;
    for (s, d) in img.as_raw().chunks_exact(4).zip(pm.data_mut().chunks_exact_mut(4)) {
        let a = s[3] as u32;
        let pre = |v: u8| ((v as u32 * a + 127) / 255) as u8;
        d.copy_from_slice(&[pre(s[0]), pre(s[1]), pre(s[2]), s[3]]);
    }
    Some(pm)
}

fn fetch_fx(agent: &ureq::Agent, id: &str) -> Option<Fx> {
    let text = agent
        .get(&format!("https://api.fxtwitter.com/status/{id}"))
        .header("User-Agent", UA)
        .call()
        .ok()?
        .body_mut()
        .read_to_string()
        .ok()?;
    parse_fx(&text)
}

/// fxtwitter `/status/{id}` 응답 → Fx.
pub fn parse_fx(text: &str) -> Option<Fx> {
    let v: Value = serde_json::from_str(text).ok()?;
    let t = v.get("tweet")?;
    let s = |x: &Value, k: &str| x.get(k).and_then(Value::as_str).map(str::to_string);
    let n = |k: &str| t.get(k).and_then(Value::as_u64).unwrap_or(0);
    let author = t.get("author");
    let quote = t.get("quote").filter(|q| !q.is_null());
    let qa = quote.and_then(|q| q.get("author"));
    Some(Fx {
        name: author.and_then(|a| s(a, "name")).unwrap_or_default(),
        handle: author.and_then(|a| s(a, "screen_name")).unwrap_or_default(),
        avatar: author.and_then(|a| s(a, "avatar_url")),
        likes: n("likes"),
        replies: n("replies"),
        retweets: n("retweets"),
        created_ms: t.get("created_timestamp").and_then(Value::as_i64).map(|s| s * 1000),
        videos: videos_of(t),
        quote_name: qa.and_then(|a| s(a, "name")),
        quote_handle: qa.and_then(|a| s(a, "screen_name")),
        quote_avatar: qa.and_then(|a| s(a, "avatar_url")),
        quote_videos: quote.map(videos_of).unwrap_or_default(),
    })
}

fn videos_of(t: &Value) -> Vec<FxVideo> {
    let Some(list) = t.get("media").and_then(|m| m.get("videos")).and_then(Value::as_array) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|v| {
            let thumb = v.get("thumbnail_url")?.as_str()?.to_string();
            let mut mp4 = v.get("url")?.as_str()?.to_string();
            // 720p 이하 중 가장 좋은 mp4(1080p 이상은 위젯 창에 넘친다)
            if let Some(fs) = v.get("formats").and_then(Value::as_array) {
                let best = fs
                    .iter()
                    .filter(|f| f.get("container").and_then(Value::as_str) == Some("mp4"))
                    .filter_map(|f| {
                        let u = f.get("url")?.as_str()?;
                        let b = f.get("bitrate").and_then(Value::as_u64).unwrap_or(0);
                        Some((u, b, res_h(u)))
                    })
                    .filter(|(_, _, h)| h.is_none_or(|h| h <= 720))
                    .max_by_key(|(_, b, _)| *b);
                if let Some((u, _, _)) = best {
                    mp4 = u.to_string();
                }
            }
            Some(FxVideo {
                thumb,
                mp4,
                w: v.get("width").and_then(Value::as_u64).unwrap_or(16) as u32,
                h: v.get("height").and_then(Value::as_u64).unwrap_or(9) as u32,
                gif: v.get("type").and_then(Value::as_str) == Some("gif"),
            })
        })
        .collect()
}

/// mp4 주소의 `/vid/avc1/1280x720/` → 720
fn res_h(u: &str) -> Option<u32> {
    let i = u.find("/vid/")?;
    u[i..].split('/').find_map(|seg| {
        let (w, h) = seg.split_once('x')?;
        w.parse::<u32>().ok()?;
        h.parse::<u32>().ok()
    })
}

/// 썸네일 주소로 영상 찾기(쿼리는 무시). 못 찾으면 같은 순번의 영상.
pub fn match_video<'a>(list: &'a [FxVideo], thumb: &str, idx: usize) -> Option<&'a FxVideo> {
    let base = |u: &str| u.split('?').next().unwrap_or(u).to_string();
    let t = base(thumb);
    list.iter().find(|v| base(&v.thumb) == t).or_else(|| list.get(idx)).or_else(|| list.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_url_rules() {
        assert_eq!(small_url("https://pbs.twimg.com/media/A.jpg"), "https://pbs.twimg.com/media/A.jpg?name=small");
        assert_eq!(
            small_url("https://pbs.twimg.com/media/A?format=jpg&name=large"),
            "https://pbs.twimg.com/media/A?format=jpg&name=small"
        );
        assert_eq!(small_url("https://example.com/a.jpg"), "https://example.com/a.jpg");
        assert!(is_video_url("https://pbs.twimg.com/amplify_video_thumb/1/img/x.jpg"));
        assert!(!is_video_url("https://pbs.twimg.com/media/x.jpg"));
    }

    #[test]
    fn parse_fx_picks_720p_and_quote() {
        let j = r#"{"tweet":{"author":{"name":"宮永ののか","screen_name":"nonoka_yumemita","avatar_url":"https://a/p.png"},
          "likes":639,"replies":10,"retweets":70,"created_timestamp":1794485257,
          "quote":{"author":{"name":"右京","screen_name":"ukyo"},"media":{"videos":[
            {"thumbnail_url":"https://pbs.twimg.com/amplify_video_thumb/1/img/t.jpg?x=1","url":"https://v/vid/avc1/3840x2160/big.mp4","width":3840,"height":2160,"type":"video",
             "formats":[{"container":"m3u8","url":"https://v/pl/a.m3u8"},
                        {"container":"mp4","bitrate":288000,"url":"https://v/vid/avc1/480x270/s.mp4"},
                        {"container":"mp4","bitrate":2176000,"url":"https://v/vid/avc1/1280x720/m.mp4"},
                        {"container":"mp4","bitrate":9000000,"url":"https://v/vid/avc1/1920x1080/l.mp4"}]}]}}}}"#;
        let f = parse_fx(j).unwrap();
        assert_eq!((f.name.as_str(), f.likes, f.replies), ("宮永ののか", 639, 10));
        assert_eq!(f.created_ms, Some(1_794_485_257_000));
        assert!(f.videos.is_empty());
        assert_eq!(f.quote_handle.as_deref(), Some("ukyo"));
        assert_eq!(f.quote_videos[0].mp4, "https://v/vid/avc1/1280x720/m.mp4");
        let v = match_video(&f.quote_videos, "https://pbs.twimg.com/amplify_video_thumb/1/img/t.jpg", 5).unwrap();
        assert_eq!(v.w, 3840);
    }
}
