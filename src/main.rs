//! Tiny Songify replacement: song requests for Pear Desktop and Spotify, from Twitch chat (through
//! Streamer.bot) and TikTok chat (through tikstream/TikFinity). See PLAN.md.

mod commands;
mod pear;
mod streamerbot;

use commands::{Do, Query, Reply};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;
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
    /// Pear's API token, when its API Server asks for authorization (filled in by itself).
    pear_token: String,
    requests: commands::Requests,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            streamerbot_url: "ws://127.0.0.1:8080/".into(),
            streamerbot_password: String::new(),
            reply_action: String::new(),
            pear_token: String::new(),
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

/// The request queue and what the chat commands need, owned by the main loop.
struct App {
    cfg: watch::Sender<Config>,
    sb: mpsc::UnboundedSender<String>,
    st: commands::State,
    start: Instant,
}

impl App {
    /// Send a reply: the command's own action, else the default one, else straight to chat.
    fn send(&self, platform: &str, r: Reply) {
        let cfg = self.cfg.borrow();
        let all = cfg.requests.commands.all();
        let own = all.iter().find(|(n, _)| *n == r.command).map_or("", |(_, c)| &c.action);
        let action = if own.is_empty() { &cfg.reply_action } else { own };
        say(format!("reply ({} {}): {}", r.command, r.result, r.text));
        // An empty template means no chat message; an action still runs.
        if !r.text.is_empty() || !action.is_empty() {
            let _ = self.sb.send(streamerbot::chat_reply(&r, platform, action));
        }
    }

    fn change(&self, f: impl FnOnce(&mut Config)) {
        self.cfg.send_modify(f);
        save(&self.cfg.borrow());
    }

    /// One chat message: run the command, carry out what it needs. ponytail: one at a time, so a
    /// slow Pear search holds up the next command; chat is slow enough for that.
    async fn chat(&mut self, platform: &'static str, user: &str, who: commands::Who, text: &str) {
        let m = commands::Msg {
            platform,
            user,
            who,
            text,
        };
        let now = self.start.elapsed().as_secs();
        let out = commands::handle(&self.cfg.borrow().requests, &mut self.st, &m, now);
        if !out.is_empty() {
            say(format!("{platform}: {user} ({who:?}): {text}"));
        }
        let token = self.cfg.borrow().pear_token.clone();
        let failed = |app: &Self, command, e: String| {
            say(format!("pear: {command} failed: {e}"));
            commands::player_error(&app.cfg.borrow().requests, command, user, &e)
        };
        for d in out {
            match d {
                Do::Reply(r) => self.send(platform, r),
                Do::Open(open) => self.change(|c| c.requests.open = open),
                Do::Ban(id) => self.change(|c| c.requests.blocked_songs.push(id)),
                Do::Search(q) => {
                    let r = self.request(platform, user, q, &token, now).await;
                    let r = r.unwrap_or_else(|e| failed(self, "ssr", e));
                    self.send(platform, r);
                }
                Do::Skip(r) | Do::Play(r) | Do::Pause(r) => {
                    let what = match r.command {
                        "play" => "play",
                        "pause" => "pause",
                        _ => "next",
                    };
                    let r = match pear::player(&token, what).await {
                        Ok(()) => r,
                        Err(e) => failed(self, r.command, e),
                    };
                    self.send(platform, r);
                }
                Do::Volume(set) => {
                    let r = match pear::volume(&token, set).await {
                        Ok(v) => commands::vol_reply(&self.cfg.borrow().requests, user, v),
                        Err(e) => failed(self, "vol", e),
                    };
                    self.send(platform, r);
                }
                Do::Removed(q) => {
                    if let Err(e) = pear::remove(&token, &q.track.id).await {
                        say(format!("pear: couldn't take {} off its queue: {e}", q.track.id));
                    }
                }
            }
        }
    }

    /// Find a request on Pear and queue it there.
    async fn request(
        &mut self,
        platform: &'static str,
        user: &str,
        q: Query,
        token: &str,
        now: u64,
    ) -> Result<Reply, String> {
        let (found, asked) = match q {
            Query::Video(id) => (pear::video(token, &id).await?, id),
            Query::Text(t) => (pear::search(token, &t).await?, t),
            Query::Spotify(url) => {
                let t = pear::spotify_query(&url).await?;
                (pear::search(token, &t).await?, t)
            }
        };
        let cfg = self.cfg.borrow().requests.clone();
        let Some(track) = found else {
            return Ok(commands::not_found(&cfg, user, &asked));
        };
        let id = track.id.clone();
        let r = commands::add(&cfg, &mut self.st, platform, user, track, now);
        if r.result == "queued" {
            let earlier: Vec<String> = self.st.queue.iter().rev().skip(1).map(|q| q.track.id.clone()).collect();
            if let Err(e) = pear::enqueue(token, &id, &earlier).await {
                commands::undo_add(&mut self.st);
                return Err(e);
            }
        }
        Ok(r)
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
    let (pear_tx, mut pear_rx) = mpsc::unbounded_channel();
    // ponytail: one runtime thread is plenty for a few sockets, and lighter than one per core.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.spawn(streamerbot::run(cfg_tx.subscribe(), sb_rx, chat_tx));
    rt.spawn(pear::run(cfg_tx.subscribe(), pear_tx));
    let mut app = App {
        cfg: cfg_tx,
        sb: sb_tx,
        st: commands::State::default(),
        start: Instant::now(),
    };
    rt.block_on(async {
        loop {
            tokio::select! {
                Some(c) = chat_rx.recv() => {
                    let who = commands::Who::twitch(c.role, c.subscribed);
                    app.chat("twitch", &c.user, who, &c.text).await;
                }
                Some(e) = pear_rx.recv() => match e {
                    pear::Event::Song(t) => {
                        // Pear repeats the same song now and then: only a new one counts.
                        if t.as_ref().map(|t| &t.id) != app.st.current.as_ref().map(|c| &c.id) {
                            if let Some(t) = &t {
                                say(format!("pear: playing {} - {} ({})", t.artist, t.title, t.id));
                            }
                            commands::song_changed(&mut app.st, t);
                        }
                    }
                    pear::Event::Token(t) => app.change(|c| c.pear_token = t),
                },
                else => break,
            }
        }
    });
}
