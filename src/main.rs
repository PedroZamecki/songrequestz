#![cfg_attr(windows, windows_subsystem = "windows")]
//! Tiny Songify replacement: song requests for Pear Desktop and Spotify, from Twitch chat (through
//! Streamer.bot) and TikTok chat (through tikstream/TikFinity). See PLAN.md.

mod commands;
mod pear;
mod server;
mod spotify;
mod streamerbot;
mod tiktok;
mod tray;
mod ui;

use commands::{Do, Query, Reply};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::{mpsc, watch};

/// A chat message from Twitch (Streamer.bot) or TikTok (tikstream/TikFinity).
pub struct Chat {
    pub platform: &'static str,
    pub user: String,
    pub who: commands::Who,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Player {
    #[default]
    Pear,
    /// The Spotify desktop app (no song requests).
    Spotify,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    player: Player,
    streamerbot_url: String,
    streamerbot_password: String,
    /// Streamer.bot action for replies of commands without their own: gets the text as `message`
    /// (plus its parts). Empty: straight to chat (SendMessage), which needs Streamer.bot's WebSocket
    /// authentication on.
    reply_action: String,
    /// tikstream/TikFinity's WebSocket for TikTok chat; empty: off.
    tiktok_url: String,
    /// Your TikTok @name: its messages count as the broadcaster's (TikTok doesn't flag them).
    tiktok_user: String,
    /// Streamer.bot action for TikTok results of commands without their own action (nothing goes
    /// back to TikTok chat; the action decides: TTS, Twitch chat, nothing). Empty: only logged.
    tiktok_action: String,
    /// Pear's API token, when its API Server asks for authorization (filled in by itself).
    pear_token: String,
    /// Songify's API port (read at start).
    port: u16,
    /// Songify's API password; empty: none.
    api_password: String,
    /// Folder for Songify.txt and cover.png; empty: next to the exe.
    files_dir: String,
    /// Songify.txt: Songify's {placeholders} (artist, single_artist, title, req, url, uri).
    output: String,
    /// Songify.txt while paused: null keeps the song, "" empties it, any other text replaces it.
    /// Empty or text also blanks cover.png.
    paused_text: Option<String>,
    requests: commands::Requests,
    dark: bool,
    start_hidden: bool,
    autostart: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            player: Player::Pear,
            streamerbot_url: "ws://127.0.0.1:8080/".into(),
            streamerbot_password: String::new(),
            reply_action: String::new(),
            tiktok_url: "ws://127.0.0.1:21213/".into(),
            tiktok_user: String::new(),
            tiktok_action: String::new(),
            pear_token: String::new(),
            port: 65530,
            api_password: String::new(),
            files_dir: String::new(),
            output: "{artist} - {title}".into(),
            paused_text: None,
            requests: commands::Requests::default(),
            dark: true,
            start_hidden: true,
            autostart: false,
        }
    }
}

/// What the window and tray show. Shared by all tasks.
struct Status {
    sb: String,
    tiktok: String,
    player: String,
    /// Streamer.bot's action names, once it sent them; bumps `actions_rev` on change.
    actions: Vec<String>,
    actions_rev: u64,
    /// Now playing and the requests, one line each; bumps `queue_rev` on change.
    now: String,
    queue: Vec<String>,
    queue_rev: u64,
    /// Last LOG_LINES log lines, and how many were ever logged (so the window knows what's new).
    log: VecDeque<String>,
    logged: u64,
}

const LOG_LINES: usize = 200;

static STATUS: Mutex<Status> = Mutex::new(Status {
    sb: String::new(),
    tiktok: String::new(),
    player: String::new(),
    actions: Vec::new(),
    actions_rev: 0,
    now: String::new(),
    queue: Vec::new(),
    queue_rev: 0,
    log: VecDeque::new(),
    logged: 0,
});

fn status(f: impl FnOnce(&mut Status)) {
    if let Ok(mut s) = STATUS.lock() {
        f(&mut s);
    }
}

/// Print and keep for the window's log.
fn say(line: String) {
    println!("{line}");
    status(|s| {
        if s.log.len() == LOG_LINES {
            s.log.pop_front();
        }
        s.log.push_back(line);
        s.logged += 1;
    });
}

/// The tray tooltip.
fn summary() -> String {
    let mut t = String::new();
    status(|s| {
        let now = if s.now.is_empty() { "nothing" } else { &s.now };
        t = format!(
            "songrequestz\n{now}\nStreamer.bot: {}\nTikTok: {}\nPlayer: {}",
            s.sb, s.tiktok, s.player
        );
    });
    t
}

fn config_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent()
        .map_or_else(|| "songrequestz.json".into(), |d| d.join("songrequestz.json"))
}

fn save(c: &Config) -> std::io::Result<()> {
    let r = std::fs::write(config_path(), serde_json::to_string_pretty(c).unwrap_or_default());
    if let Err(e) = &r {
        say(format!("can't save songrequestz.json ({e})"));
    }
    r
}

/// Start with Windows: a value under the user's Run registry key.
#[cfg(windows)]
fn autostart(on: bool) -> std::io::Result<()> {
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let w = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (key, name) = (w(r"Software\Microsoft\Windows\CurrentVersion\Run"), w("songrequestz"));
    let exe = w(&format!("\"{}\"", std::env::current_exe()?.display()));
    // SAFETY: NUL-terminated UTF-16 strings that outlive the calls; size is in bytes.
    let err = unsafe {
        if on {
            let size = (exe.len() * 2) as u32;
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name.as_ptr(),
                REG_SZ,
                exe.as_ptr().cast(),
                size,
            )
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr())
        }
    };
    // 2 = not found: already off.
    match err {
        0 | 2 => Ok(()),
        e => Err(std::io::Error::from_raw_os_error(e as i32)),
    }
}

/// Start at login: an XDG autostart entry.
#[cfg(not(windows))]
fn autostart(on: bool) -> std::io::Result<()> {
    use std::io::ErrorKind::NotFound;
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or(NotFound)?;
    let file = config.join("autostart/songrequestz.desktop");
    if !on {
        return match std::fs::remove_file(file) {
            Err(e) if e.kind() == NotFound => Ok(()),
            r => r,
        };
    }
    std::fs::create_dir_all(config.join("autostart"))?;
    let exe = std::env::current_exe()?;
    std::fs::write(
        file,
        format!(
            "[Desktop Entry]\nType=Application\nName=songrequestz\nExec=\"{}\"\n",
            exe.display()
        ),
    )
}

/// The config, or the defaults. Written back, so new settings show up in the file; an invalid file
/// is left alone.
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
    let _ = save(&c);
    c
}

/// A transparent 1x1 PNG: cover.png when there's no cover (Songify writes an empty image too).
const BLANK_PNG: [u8; 68] = [
    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 31, 21, 196,
    137, 0, 0, 0, 11, 73, 68, 65, 84, 120, 156, 99, 96, 0, 2, 0, 0, 5, 0, 1, 122, 94, 171, 63, 0, 0, 0, 0, 73, 69, 78,
    68, 174, 66, 96, 130,
];

/// The request queue and what the chat commands need, owned by the main loop.
struct App {
    cfg: Arc<watch::Sender<Config>>,
    sb: mpsc::UnboundedSender<String>,
    st: commands::State,
    start: Instant,
    /// Pear's play state and position (seconds) in the song.
    playing: bool,
    position: u32,
    /// Songify's JSON, for the API (empty: nothing played yet).
    payload: watch::Sender<Arc<str>>,
    /// What Songify.txt and cover.png hold now, so they're written only when that changes.
    text: Option<String>,
    cover: Option<String>,
}

impl App {
    /// Send a reply: the command's own action, else the platform's default one, else straight to
    /// Twitch chat (TikTok: nowhere).
    fn send(&self, platform: &str, r: Reply) {
        let cfg = self.cfg.borrow();
        let all = cfg.requests.commands.all();
        let own = all.iter().find(|(n, _)| *n == r.command).map_or("", |(_, c)| &c.action);
        let tiktok = platform == "tiktok";
        let default = if tiktok { &cfg.tiktok_action } else { &cfg.reply_action };
        let action = if own.is_empty() { default } else { own };
        say(format!("reply ({platform} {} {}): {}", r.command, r.result, r.text));
        // An empty template means no chat message; an action still runs.
        if (!r.text.is_empty() && !tiktok) || !action.is_empty() {
            let _ = self.sb.send(streamerbot::chat_reply(&r, platform, action));
        }
    }

    fn spotify(&self) -> bool {
        self.cfg.borrow().player == Player::Spotify
    }

    /// "next", "play" or "pause" on the selected player.
    async fn control(&self, what: &str) -> Result<(), String> {
        if self.spotify() {
            return spotify::player(what).await;
        }
        let token = self.cfg.borrow().pear_token.clone();
        pear::player(&token, what).await
    }

    /// Read (None) or set the volume, 0-100.
    async fn volume(&self, set: Option<u8>) -> Result<u8, String> {
        if self.spotify() {
            return Err("the Spotify app's volume can't be read or set from here".into());
        }
        let token = self.cfg.borrow().pear_token.clone();
        pear::volume(&token, set).await
    }

    fn change(&self, f: impl FnOnce(&mut Config)) {
        self.cfg.send_modify(f);
        let _ = save(&self.cfg.borrow());
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
            say(format!("player: {command} failed: {e}"));
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
                    let r = match self.control(what).await {
                        Ok(()) => r,
                        Err(e) => failed(self, r.command, e),
                    };
                    self.send(platform, r);
                }
                Do::Volume(set) => {
                    let r = match self.volume(set).await {
                        Ok(v) => commands::vol_reply(&self.cfg.borrow().requests, user, v),
                        Err(e) => failed(self, "vol", e),
                    };
                    self.send(platform, r);
                }
                Do::Removed(q) if self.spotify() => drop(q),
                Do::Removed(q) => {
                    if let Err(e) = pear::remove(&token, &q.track.id).await {
                        say(format!("pear: couldn't take {} off its queue: {e}", q.track.id));
                    }
                }
            }
        }
        self.publish();
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
        if self.spotify() {
            return Err(spotify::NO_REQUESTS.into());
        }
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

    /// A Songify API WebSocket command; the answer goes back to the client.
    async fn api(&mut self, action: &str, data: &Value) -> String {
        let token = self.cfg.borrow().pear_token.clone();
        let current = self.st.current.clone();
        let answer = match action {
            // Not Songify's: the window's Remove button.
            "queue_remove" => {
                let i = data["index"].as_u64().unwrap_or(u64::MAX) as usize;
                if i >= self.st.queue.len() {
                    return "No such request.".into();
                }
                let q = self.st.queue.remove(i);
                match pear::remove(&token, &q.track.id).await {
                    Ok(()) => format!("Removed {} - {} ({}).", q.track.artist, q.track.title, q.user),
                    Err(e) => format!("Removed, but Pear still has it: {e}"),
                }
            }
            "queue_add" => {
                let track = data["track"].as_str().unwrap_or("").trim();
                let user = data["requester"].as_str().unwrap_or("").to_string();
                let now = self.start.elapsed().as_secs();
                match Query::parse(track) {
                    _ if track.is_empty() => "No track provided.".into(),
                    None => "That link isn't a YouTube or Spotify song.".into(),
                    Some(q) => match self.request("twitch", &user, q, &token, now).await {
                        Ok(r) => r.text,
                        Err(e) => e,
                    },
                }
            }
            "skip" | "next" => self
                .control("next")
                .await
                .map_or_else(|e| e, |_| "Song skipped.".into()),
            "play" | "pause" | "play_pause" => {
                let pause = action == "pause" || (action == "play_pause" && self.playing);
                let (what, done) = if pause {
                    ("pause", "Playback paused.")
                } else {
                    ("play", "Playback resumed.")
                };
                match self.control(what).await {
                    // The player says so too, but maybe not before the next command.
                    Ok(()) => {
                        self.playing = !pause;
                        done.into()
                    }
                    Err(e) => e,
                }
            }
            "vol_set" | "vol_up" | "vol_down" => {
                let set = match action {
                    "vol_set" => Ok(data["value"].as_f64().unwrap_or(0.0)),
                    // ponytail: Pear reads a new volume back ~0.3 s late, so a step right after a set
                    // starts from the old one.
                    _ => self
                        .volume(None)
                        .await
                        .map(|v| v as f64 + if action == "vol_up" { 5.0 } else { -5.0 }),
                };
                match set {
                    Ok(v) => match self.volume(Some(v.clamp(0.0, 100.0) as u8)).await {
                        Ok(v) => format!("Volume set to {v}%"),
                        Err(e) => e,
                    },
                    Err(e) => e,
                }
            }
            "send_to_chat" => match commands::now_playing(&self.cfg.borrow().requests, &self.st) {
                Some(r) => {
                    self.send("twitch", r);
                    "Current song sent to chat.".into()
                }
                None => "Nothing is playing.".into(),
            },
            "sr_enable" | "sr_open" | "sr_disable" | "sr_close" => {
                let open = action == "sr_enable" || action == "sr_open";
                self.change(|c| c.requests.open = open);
                if open {
                    "Song requests enabled."
                } else {
                    "Song requests disabled."
                }
                .into()
            }
            "block_artist" | "block_all_artists" => match &current {
                Some(t) => {
                    let all: Vec<String> = t.artist.split(',').map(|a| a.trim().to_string()).collect();
                    let artists = if action == "block_artist" {
                        all[..1].to_vec()
                    } else {
                        all
                    };
                    self.change(|c| c.requests.blocked_artists.extend(artists));
                    if action == "block_artist" {
                        "Artist blocked."
                    } else {
                        "All artists blocked."
                    }
                    .into()
                }
                None => "Nothing is playing.".into(),
            },
            "block_song" => match &current {
                Some(t) => {
                    self.change(|c| c.requests.blocked_songs.push(t.id.clone()));
                    let _ = self.control("next").await;
                    "Song blocked.".into()
                }
                None => "Nothing is playing.".into(),
            },
            "block_user" => match self.st.current_by.clone() {
                u if u.is_empty() => "No user to block.".into(),
                u => {
                    self.change(|c| c.requests.blocked_users.push(u.clone()));
                    format!("User {u} blocked")
                }
            },
            // Songify's YouTube browser companion feed: nothing to do with Pear.
            "youtube" => String::new(),
            "play_playlist" => "Not supported: playlists are Spotify only.".into(),
            "stop_sr_reward" => "Not supported: songrequestz has no channel point rewards.".into(),
            _ => format!("Unknown action: {action}"),
        };
        say(format!("api: {action}: {answer}"));
        self.publish();
        answer
    }

    /// Rebuild Songify's JSON and hand it to the API (pushed to /ws/data only when it changed).
    fn publish(&self) {
        let line = |t: &commands::Track| format!("{} - {}", t.artist, t.title);
        let now = self.st.current.as_ref().map(line).unwrap_or_default();
        let now = match self.st.current_by.as_str() {
            "" => now,
            by => format!("{now} (requested by {by})"),
        };
        let queue: Vec<String> = (self.st.queue.iter().enumerate())
            .map(|(i, q)| format!("#{} {} ({})", i + 1, line(&q.track), q.user))
            .collect();
        status(|s| {
            if s.now != now || s.queue != queue {
                (s.now, s.queue) = (now, queue);
                s.queue_rev += 1;
            }
        });
        let Some(json) = self.payload_json() else { return };
        self.payload.send_if_modified(|p| {
            let new = **p != *json;
            if new {
                *p = json.into();
            }
            new
        });
    }

    /// Songify's payload, same names and shape; what Pear doesn't have is empty.
    fn payload_json(&self) -> Option<String> {
        let t = self.st.current.as_ref()?;
        let cfg = self.cfg.borrow();
        let ms = t.seconds as u64 * 1000;
        let progress = self.position as u64 * 1000;
        let percent = (progress * 100).checked_div(ms).unwrap_or(0).min(100);
        let artist = json!({"ExternalUrls": {}, "Href": "", "Id": "", "Name": t.artist, "Type": "", "Uri": ""});
        let requests: Vec<Value> = (self.st.queue.iter().enumerate())
            .map(|(i, q)| {
                json!({
                    "queueid": i + 1, "uuid": "", "trackid": q.track.id, "artist": q.track.artist,
                    "title": q.track.title, "length": format!("{}:{:02}", q.track.seconds / 60, q.track.seconds % 60),
                    "requester": q.user, "albumcover": q.track.cover, "playerType": "YouTube", "streamId": "",
                    "IsLiked": false, "FullRequester": null,
                })
            })
            .collect();
        let chat = cfg.requests.open && cfg.requests.commands.ssr.enabled;
        let v = json!({
            "UserInfo": {
                "TwitchUser": {"Id": "", "Login": "", "BroadcasterType": ""},
                "SpotifyUser": {"Id": "", "DisplayName": "", "Product": ""},
            },
            "SongifyInfo": {"Version": option_env!("SONGREQUESTZ_VERSION").unwrap_or("dev"), "Beta": false},
            "Track": {
                "Data": {
                    "Artists": t.artist, "Title": t.title,
                    "Albums": [{"Url": t.cover, "Width": 0, "Height": 0}],
                    "SongId": t.id, "DurationMs": ms, "IsPlaying": self.playing, "Url": t.url,
                    "DurationPercentage": percent, "DurationTotal": ms, "Progress": progress,
                    "Playlist": {"Name": null, "Id": null, "Owner": null, "Url": "", "Image": null},
                    "FullArtists": [artist],
                },
                "CanvasUrl": "",
                "IsInLikedPlaylist": false,
                "Requester": {"Name": self.st.current_by, "ProfilePicture": ""},
            },
            // ponytail: Tracks is Songify's view of the player's whole upcoming queue; here it's the
            // requests, since reading Pear's queue (~500 KB) on every change isn't worth it.
            "Queue": {"Count": requests.len(), "Requests": requests, "Tracks": requests, "songRequests": {"chat": chat, "reward": false}},
        });
        Some(serde_json::to_string_pretty(&v).unwrap_or_default())
    }

    /// Songify.txt and cover.png, written only when what they should hold changes.
    async fn files(&mut self) {
        let Some(t) = &self.st.current else { return };
        let cfg = self.cfg.borrow().clone();
        let dir = match cfg.files_dir.trim() {
            "" => config_path().with_file_name(""),
            d => PathBuf::from(d),
        };
        let paused = (!self.playing).then_some(cfg.paused_text.as_ref()).flatten();
        let song = || {
            let vars = [
                ("artist", t.artist.as_str()),
                ("single_artist", t.artist.split(',').next().unwrap_or("").trim()),
                ("title", &t.title),
                ("req", &self.st.current_by),
                ("url", &t.url),
                ("uri", &t.id),
                ("extra", ""),
            ];
            commands::fill(&cfg.output, &vars)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        let text = paused.cloned().unwrap_or_else(song);
        if self.text.as_ref() != Some(&text) {
            if let Err(e) = std::fs::write(dir.join("Songify.txt"), &text) {
                say(format!("can't write Songify.txt ({e})"));
            }
            self.text = Some(text);
        }
        let cover = if paused.is_some() {
            String::new()
        } else {
            t.cover.clone()
        };
        if self.cover.as_ref() != Some(&cover) {
            // ponytail: awaited here, so chat waits for the download (~0.2 s) once per song.
            if let Err(e) = write_cover(&dir, &cover).await {
                say(format!("can't write cover.png ({e})"));
            }
            self.cover = Some(cover);
        }
    }
}

/// cover.png: the image at `url` as it is (OBS reads it whatever its format), or a blank one. Fetched
/// with the OS's curl, like Spotify pages, then renamed in so OBS never sees half a file.
async fn write_cover(dir: &std::path::Path, url: &str) -> Result<(), String> {
    let (tmp, file) = (dir.join("cover.tmp"), dir.join("cover.png"));
    if url.is_empty() {
        std::fs::write(&tmp, BLANK_PNG).map_err(|e| e.to_string())?;
    } else {
        let mut cmd = tokio::process::Command::new("curl");
        cmd.args(["-sfL", "-m", "10", "-o"]).arg(&tmp).arg(url);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let ok = cmd
            .status()
            .await
            .map_err(|e| format!("can't run curl ({e})"))?
            .success();
        if !ok {
            return Err(format!("couldn't download {url}"));
        }
    }
    std::fs::rename(&tmp, &file).map_err(|e| e.to_string())
}

fn main() {
    // Set by the release build from the git tag; local builds say "dev".
    let version = format!("songrequestz {}", option_env!("SONGREQUESTZ_VERSION").unwrap_or("dev"));
    say(version.clone());
    let cfg = load();
    let (port, show) = (cfg.port, !cfg.start_hidden);
    let cfg_tx = Arc::new(watch::channel(cfg).0);
    let app = ui::app();
    let (sb_tx, sb_rx) = mpsc::unbounded_channel();
    let (chat_tx, mut chat_rx) = mpsc::unbounded_channel();
    let (pear_tx, mut pear_rx) = mpsc::unbounded_channel();
    let (api_tx, mut api_rx) = mpsc::unbounded_channel::<server::Command>();
    let payload = watch::channel(Arc::<str>::from("")).0;
    // ponytail: one runtime thread is plenty for a few sockets, and lighter than one per core.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let listener = match rt.block_on(server::bind(port)) {
        Ok(l) => l,
        Err(e) => ui::fatal(&e),
    };
    say(format!("config: {}", config_path().display()));
    rt.spawn(server::run(
        listener,
        payload.subscribe(),
        cfg_tx.subscribe(),
        api_tx.clone(),
    ));
    rt.spawn(streamerbot::run(cfg_tx.subscribe(), sb_rx, chat_tx.clone()));
    rt.spawn(tiktok::run(cfg_tx.subscribe(), chat_tx));
    rt.spawn(pear::run(cfg_tx.subscribe(), pear_tx.clone()));
    rt.spawn(spotify::run(cfg_tx.subscribe(), pear_tx));
    let (ui_tx, ui_rx) = fltk::app::channel();
    rt.spawn(tray::run(ui_tx));
    let mut a = App {
        cfg: cfg_tx.clone(),
        sb: sb_tx,
        st: commands::State::default(),
        start: Instant::now(),
        playing: false,
        position: 0,
        payload,
        text: None,
        cover: None,
    };
    let app_loop = async move {
        let app = &mut a;
        loop {
            tokio::select! {
                Some(c) = chat_rx.recv() => {
                    app.chat(c.platform, &c.user, c.who, &c.text).await;
                }
                Some((action, data, answer)) = api_rx.recv() => {
                    let a = app.api(&action, &data).await;
                    // Nobody waiting (the window's buttons): the log gets it.
                    if let Err(a) = answer.send(a) {
                        say(a);
                    }
                }
                Some(e) = pear_rx.recv() => {
                    match e {
                        pear::Event::Song(t) => {
                            // Pear repeats the same song now and then: only a new one counts.
                            if t.as_ref().map(|t| &t.id) != app.st.current.as_ref().map(|c| &c.id) {
                                if let Some(t) = &t {
                                    say(format!("playing {} - {} ({})", t.artist, t.title, t.id));
                                }
                                commands::song_changed(&mut app.st, t);
                                app.position = 0;
                            }
                        }
                        pear::Event::State(playing, position) => (app.playing, app.position) = (playing, position),
                        pear::Event::Position(position) => app.position = position,
                        pear::Event::Token(t) => app.change(|c| c.pear_token = t),
                    }
                    app.files().await;
                    app.publish();
                }
                else => break,
            }
        }
    };
    std::thread::spawn(move || rt.block_on(app_loop));
    ui::run(app, &version, cfg_tx, ui_rx, api_tx, show);
}
