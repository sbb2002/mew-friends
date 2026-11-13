//! 폴링 — 75초마다 preview.json · tweets.json 을 받아 메인 스레드로 넘긴다.
//! 스레드 1개 + 동기 HTTP(ureq). 실패는 조용히 넘기고 다음 주기에 다시 받는다.

use std::sync::mpsc::Sender;
use std::time::Duration;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_APP};

use crate::model::{self, Preview, TweetsFile};

/// 폴링 결과가 채널에 들어왔음을 메인 스레드에 알리는 메시지.
pub const WM_APP_POLL: u32 = WM_APP + 1;

pub struct PollResult {
    pub preview: Option<Preview>,
    pub tweets: Option<TweetsFile>,
}

fn get(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    agent
        .get(url)
        .header("User-Agent", "mew-friends")
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())
}

/// 한 번 받는다(둘 중 하나만 성공해도 그 값은 넘긴다).
pub fn fetch_once(agent: &ureq::Agent) -> PollResult {
    let preview = get(agent, model::PREVIEW_URL).and_then(|s| model::parse_preview(&s));
    let tweets = get(agent, model::TWEETS_URL).and_then(|s| model::parse_tweets(&s));
    if let Err(e) = &preview {
        eprintln!("[poll] preview: {e}");
    }
    if let Err(e) = &tweets {
        eprintln!("[poll] tweets: {e}");
    }
    PollResult { preview: preview.ok(), tweets: tweets.ok() }
}

/// 폴링 스레드를 띄운다. 받을 때마다 `tx` 로 보내고 메인 스레드에 `WM_APP_POLL` 을 붙인다.
pub fn spawn(tx: Sender<PollResult>, main_thread_id: u32) {
    std::thread::spawn(move || {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(20)))
            .build()
            .into();
        loop {
            let r = fetch_once(&agent);
            if tx.send(r).is_err() {
                return; // 메인 종료
            }
            unsafe {
                let _ = PostThreadMessageW(main_thread_id, WM_APP_POLL, WPARAM(0), LPARAM(0));
            }
            std::thread::sleep(Duration::from_secs(model::POLL_SECS));
        }
    });
}
