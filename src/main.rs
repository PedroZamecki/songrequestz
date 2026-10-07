//! Tiny Songify replacement: song requests for Pear Desktop and Spotify, from Twitch chat (through
//! Streamer.bot) and TikTok chat (through tikstream/TikFinity). See PLAN.md.

mod commands;
mod streamerbot;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::sync::{mpsc, watch};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    streamerbot_url: String,
    streamerbot_password: String,
    /// Streamer.bot action for replies of commands without their own: gets the text as `message`
    /// (plus its parts). Empty: straight to chat (SendMessage), which needs Streamer.bot's WebSocket
    /// authentication on.
    reply_action: String,
    requests: commands::Requests,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            streamerbot_url: "ws://127.0.0.1:8080/".into(),
            streamerbot_password: String::new(),
            reply_action: String::new(),
            requests: commands::Requests::default(),
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

fn save(c: &Config) {
    if let Err(e) = std::fs::write(config_path(), serde_json::to_string_pretty(c).unwrap_or_default()) {
        say(format!("can't save songrequestz.json ({e})"));
    }
}

/// The config, or the defaults. Written back, so new settings show up in the file to edit (until the
/// window exists); an invalid file is left alone.
fn load() -> Config {
    let c = match std::fs::read_to_string(config_path()) {
        Ok(s) => match serde_json::from_str(&s) {
            Ok(c) => c,
            Err(e) => {
                say(format!("songrequestz.json is invalid ({e}), using defaults"));
                return Config::default();
            }
        },
        Err(_) => Config::default(),
    };
    save(&c);
    c
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
    let start = std::time::Instant::now();
    let mut st = commands::State::default();
    rt.block_on(async {
        while let Some(c) = chat_rx.recv().await {
            let who = commands::Who::twitch(c.role, c.subscribed);
            let m = commands::Msg {
                platform: "twitch",
                user: &c.user,
                who,
                text: &c.text,
            };
            let out = commands::handle(&cfg_tx.borrow().requests, &mut st, &m, start.elapsed().as_secs());
            if !out.is_empty() {
                say(format!("twitch: {} ({}, {who:?}): {}", c.user, c.login, c.text));
            }
            let send = |r: commands::Reply| {
                let cfg = cfg_tx.borrow();
                let all = cfg.requests.commands.all();
                let own = all.iter().find(|(n, _)| *n == r.command).map_or("", |(_, c)| &c.action);
                let action = if own.is_empty() { &cfg.reply_action } else { own };
                say(format!("reply ({} {}): {}", r.command, r.result, r.text));
                // An empty template means no chat message; an action still runs.
                if !r.text.is_empty() || !action.is_empty() {
                    let _ = sb_tx.send(streamerbot::chat_reply(&r, "twitch", action));
                }
            };
            // ponytail: no player until phase 3 (Pear), so every player call fails.
            let no_player = |command| {
                commands::player_error(&cfg_tx.borrow().requests, command, &c.user, "no player connected yet")
            };
            for d in out {
                match d {
                    commands::Do::Reply(r) => send(r),
                    commands::Do::Open(open) => {
                        cfg_tx.send_modify(|c| c.requests.open = open);
                        save(&cfg_tx.borrow());
                    }
                    commands::Do::Ban(id) => {
                        cfg_tx.send_modify(|c| c.requests.blocked_songs.push(id));
                        save(&cfg_tx.borrow());
                    }
                    commands::Do::Search(_) => send(no_player("ssr")),
                    commands::Do::Volume(_) => send(no_player("vol")),
                    commands::Do::Skip(r) | commands::Do::Play(r) | commands::Do::Pause(r) => {
                        send(no_player(r.command))
                    }
                    commands::Do::Removed(_) => {}
                }
            }
        }
    });
}
