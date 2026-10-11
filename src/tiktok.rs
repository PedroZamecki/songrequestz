//! TikTok chat from tikstream or TikFinity (`ws://127.0.0.1:21213/`): each `chat` event goes to
//! main.rs like Twitch chat; every other event is ignored. Nothing is ever sent back to TikTok.

use crate::commands::Who;
use crate::{say, Chat, Config};
use futures_util::StreamExt;
use serde::Deserialize;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

/// The fields used from a TikTok-Live-Connector style `{"event","data"}` message.
#[derive(Deserialize)]
struct Event {
    event: String,
    #[serde(default)]
    data: Data,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Data {
    unique_id: String,
    comment: String,
    is_moderator: bool,
    is_subscriber: bool,
    follow_role: u8,
}

/// A chat message, by its sender's @name (what TikTok mentions use). `host`: the streamer's @name.
fn chat_message(text: &str, host: &str) -> Option<Chat> {
    let e: Event = serde_json::from_str(text).ok()?;
    let d = e.data;
    if e.event != "chat" || d.unique_id.is_empty() {
        return None;
    }
    let host = host.trim().trim_start_matches('@');
    let is_host = !host.is_empty() && d.unique_id.eq_ignore_ascii_case(host);
    Some(Chat {
        platform: "tiktok",
        who: Who::tiktok(is_host, d.is_moderator, d.is_subscriber, d.follow_role),
        user: d.unique_id,
        text: d.comment,
    })
}

/// Stay connected to `tiktok_url`, retrying every 5 s (logged once), reconnecting when it changes.
pub async fn run(mut cfg: watch::Receiver<Config>, chat: mpsc::UnboundedSender<Chat>) {
    // A second receiver for reading the host live while `moved` holds the first.
    let live = cfg.clone();
    let mut was_up = true;
    loop {
        let url = cfg.borrow_and_update().tiktok_url.trim().to_string();
        if url.is_empty() {
            say("tiktok: off".into());
            moved(&mut cfg, &url).await;
            continue;
        }
        match tokio_tungstenite::connect_async(url.as_str()).await {
            Ok((mut ws, _)) => {
                say(format!("tiktok: connected to {url}"));
                was_up = true;
                loop {
                    tokio::select! {
                        m = ws.next() => match m {
                            Some(Ok(Message::Text(t))) => {
                                if let Some(c) = chat_message(&t, &live.borrow().tiktok_user) {
                                    let _ = chat.send(c);
                                }
                            }
                            Some(Ok(_)) => {}
                            _ => break,
                        },
                        _ = moved(&mut cfg, &url) => break,
                    }
                }
                say("tiktok: disconnected".into());
            }
            Err(e) => {
                if was_up {
                    say(format!("tiktok: can't connect to {url} ({e}), retrying every 5s"));
                }
                was_up = false;
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            _ = moved(&mut cfg, &url) => {}
        }
    }
}

/// Wait for a different URL.
async fn moved(cfg: &mut watch::Receiver<Config>, url: &str) {
    let _ = cfg.wait_for(|c| c.tiktok_url.trim() != url).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat() {
        // As tikstream sends it (dev/fake.py's shape).
        let t = r#"{"event":"chat","data":{"userId":"1","uniqueId":"ana","nickname":"Ana",
            "isModerator":false,"isSubscriber":true,"followRole":1,"comment":"!ssr never gonna"}}"#;
        let c = chat_message(t, "").unwrap();
        assert_eq!((c.platform, c.user.as_str(), c.who), ("tiktok", "ana", Who::Subs));
        assert_eq!(c.text, "!ssr never gonna");
        assert_eq!(chat_message(t, "@Ana").unwrap().who, Who::Broadcaster);
        assert!(chat_message(r#"{"event":"like","data":{"uniqueId":"ana","likeCount":5}}"#, "").is_none());
        assert!(chat_message("not json", "").is_none());
    }
}
