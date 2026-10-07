//! Tiny Songify replacement: song requests for Pear Desktop and Spotify, from Twitch chat (through
//! Streamer.bot) and TikTok chat (through tikstream/TikFinity). See PLAN.md.

mod streamerbot;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::sync::{mpsc, watch};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    streamerbot_url: String,
    streamerbot_password: String,
    /// Streamer.bot action that sends its `message` arg to Twitch chat. Empty: SendMessage, which
    /// needs Streamer.bot's WebSocket authentication on.
    reply_action: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            streamerbot_url: "ws://127.0.0.1:8080/".into(),
            streamerbot_password: String::new(),
            reply_action: String::new(),
        }
    }
}

fn say(line: String) {
    println!("{line}");
}

fn config_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent()
        .map_or_else(|| "songrequestz.json".into(), |d| d.join("songrequestz.json"))
}

/// The config, or the defaults (written out, so there's a file to edit until the window exists).
fn load() -> Config {
    match std::fs::read_to_string(config_path()) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            say(format!("songrequestz.json is invalid ({e}), using defaults"));
            Config::default()
        }),
        Err(_) => {
            let c = Config::default();
            let _ = std::fs::write(config_path(), serde_json::to_string_pretty(&c).unwrap_or_default());
            c
        }
    }
}

fn main() {
    // Set by the release build from the git tag; local builds say "dev".
    say(format!(
        "songrequestz {}",
        option_env!("SONGREQUESTZ_VERSION").unwrap_or("dev")
    ));
    let cfg_tx = watch::channel(load()).0;
    let (sb_tx, sb_rx) = mpsc::unbounded_channel();
    let (chat_tx, mut chat_rx) = mpsc::unbounded_channel();
    // ponytail: one runtime thread is plenty for a few sockets, and lighter than one per core.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.spawn(streamerbot::run(cfg_tx.subscribe(), sb_rx, chat_tx));
    rt.block_on(async {
        while let Some(c) = chat_rx.recv().await {
            let sub = if c.subscribed { ", sub" } else { "" };
            say(format!(
                "twitch: {} ({}, role {}{sub}): {}",
                c.user, c.login, c.role, c.text
            ));
            // ponytail: echo test until commands.rs (phase 2) takes over.
            if c.text.split_whitespace().next() == Some("!song") {
                let reply = format!("@{} songrequestz hears you", c.user);
                let _ = sb_tx.send(streamerbot::chat_reply(&reply, &cfg_tx.borrow().reply_action));
            }
        }
    });
}
