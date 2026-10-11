//! Streamer.bot client: authenticates, subscribes to Twitch chat (each message goes to main.rs) and
//! sends the requests main.rs makes (chat replies, DoAction).

use crate::commands::{Reply, Who};
use crate::{say, Chat, Config};
use base64::prelude::{Engine, BASE64_STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

/// DoAction request. `args` is written as-is, without copying it into a new JSON tree.
pub fn do_action(action: &str, args: &Value) -> String {
    format!(
        r#"{{"request":"DoAction","id":"action","action":{{"name":{}}},"args":{args}}}"#,
        json!(action)
    )
}

/// A chat reply. Without an action it goes to Twitch chat with SendMessage, which Streamer.bot
/// only accepts with its WebSocket authentication on. An action gets the text as `message`, plus
/// `command`, `result`, `platform` and the reply's parts (`user`, `title`, `pos`, ...).
pub fn chat_reply(r: &Reply, platform: &str, action: &str) -> String {
    if action.is_empty() {
        return json!({ "request": "SendMessage", "id": "reply", "platform": "twitch", "bot": false, "message": r.text })
            .to_string();
    }
    let mut args: serde_json::Map<String, Value> = r.args.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
    args.insert("message".into(), json!(r.text));
    args.insert("command".into(), json!(r.command));
    args.insert("result".into(), json!(r.result));
    args.insert("platform".into(), json!(platform));
    do_action(action, &Value::Object(args))
}

const SUBSCRIBE: &str = r#"{"request":"Subscribe","id":"subscribe","events":{"Twitch":["ChatMessage"]}}"#;

const NO_AUTH: &str = "streamer.bot: authentication is off, so replies can't go straight to chat. Either turn \
it on (Streamer.bot: Servers/Clients > WebSocket Server > Enable Authentication, set a password) and put the \
password in songrequestz.json (streamerbot_password), or give replies a Streamer.bot action (reply_action for all, \
or a command's own action) that sends %message% to chat; see the README";

/// Handle a message from Streamer.bot; returns requests to send back.
fn reply(text: &str, c: &Config, chat: &mpsc::UnboundedSender<Chat>) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    if let Some(m) = chat_message(&v) {
        let _ = chat.send(m);
        return Vec::new();
    }
    if v["request"] == "Hello" {
        let a = &v["authentication"];
        let (Some(challenge), Some(salt)) = (a["challenge"].as_str(), a["salt"].as_str()) else {
            if c.reply_action.is_empty() {
                say(NO_AUTH.into());
            }
            return vec![SUBSCRIBE.into()];
        };
        if c.streamerbot_password.is_empty() {
            say("streamer.bot: needs its password (streamerbot_password in songrequestz.json)".into());
            return Vec::new();
        }
        let auth = auth(&c.streamerbot_password, salt, challenge);
        let auth = json!({ "request": "Authenticate", "id": "auth", "authentication": auth });
        return vec![auth.to_string(), SUBSCRIBE.into()];
    }
    if v["status"] == "error" {
        let (id, e) = (v["id"].as_str().unwrap_or("?"), v["error"].as_str().unwrap_or("?"));
        say(format!("streamer.bot: {id} failed: {e}"));
    }
    Vec::new()
}

/// A Twitch chat message, from Streamer.bot's `Twitch.ChatMessage` event (user role: 1 viewer,
/// 2 VIP, 3 moderator, 4 broadcaster).
fn chat_message(v: &Value) -> Option<Chat> {
    if v["event"]["source"] != "Twitch" || v["event"]["type"] != "ChatMessage" {
        return None;
    }
    let (d, u) = (&v["data"], &v["data"]["user"]);
    Some(Chat {
        platform: "twitch",
        user: u["name"].as_str()?.into(),
        who: Who::twitch(
            u["role"].as_u64().unwrap_or(1) as u8,
            u["subscribed"].as_bool().unwrap_or(false),
        ),
        text: d["text"].as_str()?.into(),
    })
}

/// Streamer.bot's (OBS-style) challenge: base64(sha256(base64(sha256(password + salt)) + challenge)).
fn auth(password: &str, salt: &str, challenge: &str) -> String {
    let secret = BASE64_STANDARD.encode(Sha256::digest(format!("{password}{salt}")));
    BASE64_STANDARD.encode(Sha256::digest(format!("{secret}{challenge}")))
}

pub async fn run(
    mut cfg: watch::Receiver<Config>,
    mut cmd: mpsc::UnboundedReceiver<String>,
    chat: mpsc::UnboundedSender<Chat>,
) {
    let mut was_up = true;
    loop {
        // Wrong password (Streamer.bot closes with 4009): no use retrying until it changes.
        let mut stuck = false;
        let c = cfg.borrow_and_update().clone();
        let url = c.streamerbot_url.trim();
        if url.is_empty() {
            say("streamer.bot: off".into());
            tokio::select! {
                _ = cfg.changed() => {}
                // Requests while off: drop them.
                c = cmd.recv() => if c.is_none() { return },
            }
            continue;
        }
        match tokio_tungstenite::connect_async(url).await {
            Ok((ws, _)) => {
                say(format!("streamer.bot: connected ({url})"));
                was_up = true;
                // Requests from while it was down are stale: drop them.
                while cmd.try_recv().is_ok() {}
                let (mut w, mut r) = ws.split();
                let mut out: Vec<String> = Vec::new();
                loop {
                    let mut failed = false;
                    for m in out.drain(..) {
                        failed |= w.feed(Message::text(m)).await.is_err();
                    }
                    if failed || w.flush().await.is_err() {
                        break;
                    }
                    tokio::select! {
                        m = cmd.recv() => match m {
                            Some(req) => out.push(req),
                            None => return,
                        },
                        m = r.next() => match m {
                            Some(Ok(Message::Text(t))) => out = reply(&t, &c, &chat),
                            Some(Ok(Message::Close(Some(f)))) => stuck = u16::from(f.code) == 4009,
                            Some(Ok(_)) => {}
                            _ => break,
                        },
                        _ = moved(&mut cfg, &c) => break,
                    }
                }
                say(if stuck {
                    "streamer.bot: wrong password (streamerbot_password in songrequestz.json)".into()
                } else {
                    "streamer.bot: disconnected".into()
                });
            }
            Err(e) => {
                if was_up {
                    say(format!("streamer.bot: can't connect to {url} ({e}), retrying every 5s"));
                }
                was_up = false;
            }
        }
        let retry = tokio::time::sleep(Duration::from_secs(5));
        tokio::pin!(retry);
        loop {
            tokio::select! {
                _ = &mut retry, if !stuck => break,
                _ = moved(&mut cfg, &c) => break,
                // Can't send it now.
                m = cmd.recv() => if m.is_none() { return },
            }
        }
    }
}

/// Reconnect only if the URL or password changed; the rest is read live.
// Returns () so wait_for's read guard isn't held across awaits (it isn't Send).
async fn moved(cfg: &mut watch::Receiver<Config>, c: &Config) {
    let _ = cfg
        .wait_for(|n| n.streamerbot_url != c.streamerbot_url || n.streamerbot_password != c.streamerbot_password)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests() {
        // Same scheme as OBS websocket; vector computed with Python's hashlib/base64.
        assert_eq!(
            auth("pw", "salt", "chal"),
            "Q0/N7u40IGqt4Ynf9v3w9R2yCDZxba9npsZLdIyWi8M="
        );
        let r = Reply {
            command: "ssr",
            result: "queued",
            text: "hi \"x\"".into(),
            args: vec![("pos", "#2".into())],
        };
        let req: Value = serde_json::from_str(&chat_reply(&r, "twitch", "")).unwrap();
        assert_eq!(
            (&req["request"], &req["message"]),
            (&json!("SendMessage"), &json!("hi \"x\""))
        );
        let req: Value = serde_json::from_str(&chat_reply(&r, "twitch", "Reply")).unwrap();
        let want =
            json!({"message": "hi \"x\"", "command": "ssr", "result": "queued", "platform": "twitch", "pos": "#2"});
        assert_eq!((&req["action"]["name"], &req["args"]), (&json!("Reply"), &want));
    }

    #[test]
    fn chat() {
        // Trimmed from a real Streamer.bot 1.0.7 event.
        let v = json!({"event":{"source":"Twitch","type":"ChatMessage"},"data":{"user":{"role":4,
            "subscribed":false,"login":"pedrozamecki","name":"PedroZamecki"},"text":"!ssr teste"}});
        let m = chat_message(&v).unwrap();
        assert_eq!((m.user.as_str(), m.who), ("PedroZamecki", Who::Broadcaster));
        assert_eq!(m.text, "!ssr teste");
        assert!(chat_message(&json!({"id":"subscribe","status":"ok"})).is_none());
    }
}
