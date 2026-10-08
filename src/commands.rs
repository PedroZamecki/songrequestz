//! Chat commands: permissions, limits, the request queue and reply templates. No I/O: main.rs feeds
//! chat in and carries out what comes back (player calls, chat replies, config changes).
//! Every command answers, whatever happens; a reply template left empty sends nothing to chat (its
//! Streamer.bot action still runs).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Who may use a command, from everyone up; a user counts as their highest rank.
#[derive(Clone, Copy, PartialEq, PartialOrd, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Who {
    Everyone,
    Followers,
    Subs,
    Vips,
    Mods,
    Broadcaster,
}

impl Who {
    /// A Twitch chatter (Streamer.bot role: 1 viewer, 2 VIP, 3 moderator, 4 broadcaster). Following
    /// can't be seen without a Twitch login, so every chatter counts as a follower: Twitch's own
    /// followers-only chat mode does that job.
    pub fn twitch(role: u8, subscribed: bool) -> Who {
        match role {
            4 => Who::Broadcaster,
            3 => Who::Mods,
            2 => Who::Vips,
            _ if subscribed => Who::Subs,
            _ => Who::Followers,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Command {
    /// Without the "!".
    pub trigger: String,
    pub enabled: bool,
    pub who: Who,
    pub reply: String,
    /// Streamer.bot action that gets this command's replies (`message`) and their parts (see
    /// `Reply`) instead of them going to chat as-is. Empty: the default action, or straight to chat.
    #[serde(default)]
    pub action: String,
}

fn cmd(trigger: &str, who: Who, reply: &str) -> Command {
    Command {
        trigger: trigger.into(),
        enabled: true,
        who,
        reply: reply.into(),
        action: String::new(),
    }
}

/// Songify's commands and default replies.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Commands {
    pub ssr: Command,
    pub song: Command,
    pub next: Command,
    pub skip: Command,
    pub voteskip: Command,
    pub remove: Command,
    pub pos: Command,
    pub queue: Command,
    /// Reply for reading and setting it.
    pub vol: Command,
    pub play: Command,
    pub pause: Command,
    pub cmds: Command,
    pub togglesr: Command,
    pub bansong: Command,
}

impl Default for Commands {
    fn default() -> Self {
        use Who::*;
        Commands {
            ssr: cmd(
                "ssr",
                Everyone,
                "{artist} - {title} requested by @{user} has been added to the queue ({pos}).",
            ),
            song: cmd(
                "song",
                Everyone,
                "@{user} {single_artist} - {title} {{requested by @{req}}}",
            ),
            next: cmd("next", Everyone, "@{user} {song}"),
            skip: cmd("skip", Mods, "@{user} skipped the current song."),
            voteskip: cmd(
                "voteskip",
                Everyone,
                "@{user} voted to skip the current song. ({votes})",
            ),
            remove: cmd(
                "remove",
                Everyone,
                "{user} your previous request ({song}) will be skipped.",
            ),
            pos: cmd("pos", Everyone, "@{user} {songs}{pos} {song}{/songs}"),
            queue: cmd("queue", Everyone, "{queue}"),
            vol: cmd("vol", Mods, "Volume at {vol}%"),
            play: cmd("play", Mods, "Playback resumed."),
            pause: cmd("pause", Mods, "Playback stopped."),
            cmds: cmd("cmds", Everyone, "Active Songify commands: {commands}"),
            togglesr: cmd("togglesr", Mods, "Song requests are now {state}"),
            bansong: cmd("bansong", Mods, "The song {song} has been added to the blocklist."),
        }
    }
}

impl Commands {
    pub fn all(&self) -> [(&'static str, &Command); 14] {
        [
            ("ssr", &self.ssr),
            ("song", &self.song),
            ("next", &self.next),
            ("skip", &self.skip),
            ("voteskip", &self.voteskip),
            ("remove", &self.remove),
            ("pos", &self.pos),
            ("queue", &self.queue),
            ("vol", &self.vol),
            ("play", &self.play),
            ("pause", &self.pause),
            ("cmds", &self.cmds),
            ("togglesr", &self.togglesr),
            ("bansong", &self.bansong),
        ]
    }
}

/// Replies for everything that isn't a command's own success (Songify's texts where it has them).
/// Besides their own placeholders, all have {user} and {cmd}.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Replies {
    pub disabled: String,
    pub no_permission: String,
    pub player_error: String,
    pub no_query: String,
    pub closed: String,
    pub blocked_user: String,
    pub cooldown: String,
    pub max_per_user: String,
    pub full: String,
    pub too_long: String,
    pub blocked_artist: String,
    pub blocked_song: String,
    pub in_queue: String,
    pub not_found: String,
    pub bad_link: String,
    pub nothing_playing: String,
    pub queue_empty: String,
    pub no_requests: String,
    pub not_in_queue: String,
    pub removed_by_mod: String,
    pub already_voted: String,
    pub vote_skipped: String,
}

impl Default for Replies {
    fn default() -> Self {
        Replies {
            disabled: "@{user} the command {cmd} is not enabled.".into(),
            no_permission: "@{user} only {userlevel} or higher can use {cmd}.".into(),
            player_error: "@{user} {cmd} didn't work: {errormsg}".into(),
            no_query: "@{user} please specify a song to add to the queue.".into(),
            closed: "@{user} song requests are closed.".into(),
            blocked_user: "@{user} you can't request songs.".into(),
            cooldown: "@{user} you have to wait {cd} before you can request a song again.".into(),
            max_per_user: "@{user} maximum number of songs in queue reached ({maxreq}).".into(),
            full: "@{user} the queue is full.".into(),
            too_long: "@{user} the song you requested exceeded the maximum song length ({maxlength}).".into(),
            blocked_artist: "@{user} Artist: {artist} has been blocked by the broadcaster.".into(),
            blocked_song: "@{user} the song: {song} has been blocked by the broadcaster.".into(),
            in_queue: "@{user} this song is already in the queue.".into(),
            not_found: "@{user} no track found for \"{query}\".".into(),
            bad_link: "@{user} that link isn't a YouTube or Spotify song.".into(),
            nothing_playing: "@{user} nothing is playing right now.".into(),
            queue_empty: "@{user} the queue is empty.".into(),
            no_requests: "@{user} you have no songs in the current queue.".into(),
            not_in_queue: "@{user} there's no request {arg} in the queue.".into(),
            removed_by_mod: "The request {song} requested by @{req} has been removed.".into(),
            already_voted: "@{user} you already voted to skip this song. ({votes})".into(),
            vote_skipped: "Skipping song by vote...".into(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Requests {
    pub open: bool,
    /// 0: no limit.
    pub max_queue: usize,
    /// 0: no limit.
    pub max_per_user: usize,
    /// Longest song, in minutes. 0: no limit.
    pub max_minutes: u32,
    /// Seconds between one user's requests.
    pub cooldown_s: u64,
    pub votes_needed: usize,
    /// Matched case-insensitively: user names, artist names, song ids or titles.
    pub blocked_users: Vec<String>,
    pub blocked_artists: Vec<String>,
    pub blocked_songs: Vec<String>,
    pub commands: Commands,
    pub replies: Replies,
}

impl Default for Requests {
    fn default() -> Self {
        Requests {
            open: true,
            max_queue: 0,
            max_per_user: 3,
            max_minutes: 10,
            cooldown_s: 0,
            votes_needed: 5,
            blocked_users: Vec::new(),
            blocked_artists: Vec::new(),
            blocked_songs: Vec::new(),
            commands: Commands::default(),
            replies: Replies::default(),
        }
    }
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Track {
    pub id: String,
    pub title: String,
    /// "A, B" for several.
    pub artist: String,
    pub seconds: u32,
    pub url: String,
}

impl Track {
    fn song(&self) -> String {
        format!("{} - {}", self.artist, self.title)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Request {
    /// "twitch" or "tiktok".
    pub platform: &'static str,
    pub user: String,
    pub track: Track,
}

/// A chat message, from either platform.
pub struct Msg<'a> {
    pub platform: &'static str,
    pub user: &'a str,
    pub who: Who,
    pub text: &'a str,
}

#[derive(Default)]
pub struct State {
    pub queue: Vec<Request>,
    /// What's playing, and who asked for it (empty: not a request).
    pub current: Option<Track>,
    pub current_by: String,
    votes: Vec<String>,
    /// Last request time per "platform:user".
    last: HashMap<String, u64>,
}

/// What main.rs has to do.
#[derive(PartialEq, Debug)]
pub enum Do {
    Reply(Reply),
    /// Look it up on the player, then `add` what it finds (`not_found`, `player_error`).
    Search(Query),
    /// Player calls: the reply goes out once it worked, `player_error` if it didn't.
    Skip(Reply),
    Play(Reply),
    Pause(Reply),
    /// Read (None) or set the volume, then `vol_reply` (`player_error`).
    Volume(Option<u8>),
    /// Take it off the player's queue too.
    Removed(Request),
    /// Open or close requests, and save.
    Open(bool),
    /// Add this to blocked_songs and save (a Skip comes too).
    Ban(String),
}

/// What a request asks for.
#[derive(PartialEq, Debug)]
pub enum Query {
    Text(String),
    /// A YouTube video id.
    Video(String),
    /// A Spotify track link.
    Spotify(String),
}

impl Query {
    /// A request's text: a YouTube or Spotify song link or id, or words to search. None: a link to
    /// something else (playlist, album, artist, another site).
    pub fn parse(arg: &str) -> Option<Query> {
        let id_ok = |id: &str| id.len() == 11 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if let Some(id) = arg.strip_prefix("spotify:track:") {
            return Some(Query::Spotify(format!("https://open.spotify.com/track/{id}")));
        }
        let Some(rest) = arg.strip_prefix("https://").or_else(|| arg.strip_prefix("http://")) else {
            // ponytail: an 11-character word with a digit, - or _ is taken for a video id; all-letter
            // ids get searched, and YouTube finds a video by its id anyway.
            let is_id = id_ok(arg) && !arg.chars().all(|c| c.is_ascii_alphabetic());
            return Some(if is_id {
                Query::Video(arg.into())
            } else {
                Query::Text(arg.into())
            });
        };
        let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
        let host = host.trim_start_matches("www.").trim_start_matches("m.");
        let video = match host {
            "open.spotify.com" if path.contains("track/") => return Some(Query::Spotify(arg.into())),
            "spotify.link" | "spotify.app.link" => return Some(Query::Spotify(arg.into())),
            "youtu.be" => path.split(['?', '/']).next(),
            "youtube.com" | "music.youtube.com" => match path.split_once('/') {
                Some(("shorts" | "live", p)) => p.split(['?', '/']).next(),
                _ => path.split(['?', '&']).find_map(|p| p.strip_prefix("v=")),
            },
            _ => None,
        };
        video.filter(|id| id_ok(id)).map(|id| Query::Video(id.into()))
    }
}

/// A chat reply, and what went into it: the args of the command's Streamer.bot action.
#[derive(PartialEq, Debug)]
pub struct Reply {
    pub command: &'static str,
    /// ok, or why not: blocked, closed, full, cooldown, toolong, duplicate, notfound, error, ...
    pub result: &'static str,
    pub text: String,
    pub args: Vec<(&'static str, String)>,
}

fn out(command: &'static str, result: &'static str, template: &str, vars: &[(&'static str, &str)]) -> Reply {
    let args = vars.iter().map(|(k, v)| (*k, v.to_string())).collect();
    Reply {
        command,
        result,
        text: fill(template, vars),
        args,
    }
}

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn key(platform: &str, user: &str) -> String {
    format!("{platform}:{}", user.to_lowercase())
}

/// Fill `{name}` placeholders. Text in `{{...}}` only shows when the song is a request ({req}).
pub fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut t = template.to_string();
    for (k, v) in vars {
        t = t.replace(&format!("{{{k}}}"), v);
    }
    let req = vars.iter().any(|(k, v)| *k == "req" && !v.is_empty());
    if let (Some(a), Some(b)) = (t.find("{{"), t.rfind("}}")) {
        if a < b {
            if req {
                t = t.replace("{{", "").replace("}}", "");
            } else {
                t.replace_range(a..b + 2, "");
            }
        }
    }
    t.trim().to_string()
}

fn first_artist(artist: &str) -> &str {
    artist.split(',').next().unwrap_or("").trim()
}

fn when(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds} seconds"),
        _ => format!("{} minutes {} seconds", seconds / 60, seconds % 60),
    }
}

fn cmd_name(c: &Command) -> String {
    format!("!{}", c.trigger.trim_start_matches('!'))
}

/// Handle one chat message. Nothing comes back for chat that isn't one of the commands.
pub fn handle(cfg: &Requests, st: &mut State, m: &Msg, now: u64) -> Vec<Do> {
    // Some chat clients add U+034F to tell repeated messages apart.
    let junk = |c: char| c.is_whitespace() || c == '\u{34f}';
    let text = m.text.trim_matches(junk);
    let (word, arg) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    let arg = arg.trim_matches(junk);
    let Some(word) = word.strip_prefix('!') else {
        return Vec::new();
    };
    let Some((name, c)) = cfg
        .commands
        .all()
        .into_iter()
        .find(|(_, c)| same(c.trigger.trim_start_matches('!'), word))
    else {
        return Vec::new();
    };
    let (r, user, cmd) = (&cfg.replies, m.user, cmd_name(c));
    let rep = |result, t: &str, vars: &[(&'static str, &str)]| {
        let mut all = vec![("user", user), ("cmd", cmd.as_str())];
        all.extend_from_slice(vars);
        out(name, result, t, &all)
    };
    let reply = |result, t: &str, vars: &[(&'static str, &str)]| vec![Do::Reply(rep(result, t, vars))];
    if !c.enabled {
        return reply("disabled", &r.disabled, &[]);
    }
    if m.who < c.who {
        let level = format!("{:?}", c.who).to_lowercase();
        return reply("blocked", &r.no_permission, &[("userlevel", &level)]);
    }
    // The playing song's placeholders.
    let current = st.current.as_ref().map(|t| {
        [
            ("artist", t.artist.clone()),
            ("single_artist", first_artist(&t.artist).to_string()),
            ("title", t.title.clone()),
            ("song", t.song()),
            ("url", t.url.clone()),
            ("req", st.current_by.clone()),
        ]
    });
    let current: Option<Vec<(&'static str, &str)>> = current
        .as_ref()
        .map(|v| v.iter().map(|(k, v)| (*k, v.as_str())).collect());
    let nothing_playing = || reply("nothing", &r.nothing_playing, &[]);
    match name {
        "ssr" => {
            if cfg.blocked_users.iter().any(|b| same(b, user)) {
                return reply("blocked", &r.blocked_user, &[]);
            }
            if !cfg.open {
                return reply("closed", &r.closed, &[]);
            }
            if arg.is_empty() {
                return reply("error", &r.no_query, &[]);
            }
            if let Some(no) = limits(cfg, st, m.platform, user, now) {
                return vec![Do::Reply(no)];
            }
            match Query::parse(arg) {
                Some(q) => vec![Do::Search(q)],
                None => reply("badlink", &r.bad_link, &[]),
            }
        }
        "song" => match &current {
            Some(vars) => reply("ok", &c.reply, vars),
            None => nothing_playing(),
        },
        "next" => match st.queue.first() {
            Some(q) => reply("ok", &c.reply, &[("song", &q.track.song()), ("req", &q.user)]),
            None => reply("empty", &r.queue_empty, &[]),
        },
        "skip" => match &current {
            Some(vars) => vec![Do::Skip(rep("ok", &c.reply, vars))],
            None => nothing_playing(),
        },
        "voteskip" => {
            if current.is_none() {
                return nothing_playing();
            }
            let need = cfg.votes_needed.max(1);
            let me = key(m.platform, user);
            if st.votes.contains(&me) {
                return reply(
                    "duplicate",
                    &r.already_voted,
                    &[("votes", &format!("{}/{need}", st.votes.len()))],
                );
            }
            st.votes.push(me);
            let votes = format!("{}/{need}", st.votes.len());
            let mut out = reply("ok", &c.reply, &[("votes", &votes)]);
            if st.votes.len() >= need {
                st.votes.clear();
                out.push(Do::Skip(rep("ok", &r.vote_skipped, &[("votes", &votes)])));
            }
            out
        }
        "remove" => {
            let by_mod = m.who >= Who::Mods && !arg.is_empty();
            let i = if !by_mod {
                st.queue
                    .iter()
                    .rposition(|q| q.platform == m.platform && same(&q.user, user))
            } else if let Ok(pos) = arg.trim_start_matches('#').parse::<usize>() {
                (1..=st.queue.len()).contains(&pos).then(|| pos - 1)
            } else {
                st.queue
                    .iter()
                    .rposition(|q| same(&q.user, arg.trim_start_matches('@')))
            };
            let Some(i) = i else {
                return match by_mod {
                    true => reply("notfound", &r.not_in_queue, &[("arg", arg)]),
                    false => reply("notfound", &r.no_requests, &[]),
                };
            };
            let q = st.queue.remove(i);
            let t = if by_mod { &r.removed_by_mod } else { &c.reply };
            let vars = [
                ("song", q.track.song()),
                ("req", q.user.clone()),
                ("pos", format!("#{}", i + 1)),
            ];
            let vars: Vec<(&'static str, &str)> = vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let done = rep("ok", t, &vars);
            vec![Do::Removed(q), Do::Reply(done)]
        }
        "pos" => {
            let mine: Vec<(usize, String)> = (st.queue.iter().enumerate())
                .filter(|(_, q)| q.platform == m.platform && same(&q.user, user))
                .map(|(i, q)| (i + 1, q.track.song()))
                .collect();
            if mine.is_empty() {
                return reply("notfound", &r.no_requests, &[]);
            }
            // "{songs}...{/songs}" repeats for each request, joined by " | ".
            let t = &c.reply;
            let (a, b) = match (t.find("{songs}"), t.find("{/songs}")) {
                (Some(a), Some(b)) if a < b => (a, b),
                _ => (t.len(), t.len()),
            };
            let each = t.get(a + 7..b).unwrap_or("{pos} {song}");
            let list: Vec<String> = mine
                .iter()
                .map(|(pos, song)| each.replace("{pos}", &format!("#{pos}")).replace("{song}", song))
                .collect();
            let t = format!("{}{}{}", &t[..a], list.join(" | "), t.get(b + 8..).unwrap_or(""));
            reply("ok", &t, &[])
        }
        "queue" => {
            if st.queue.is_empty() {
                return reply("empty", &r.queue_empty, &[]);
            }
            let list: Vec<String> = (st.queue.iter().take(5).enumerate())
                .map(|(i, q)| format!("#{} {} (@{})", i + 1, q.track.song(), q.user))
                .collect();
            reply(
                "ok",
                &c.reply,
                &[("queue", &list.join(" | ")), ("count", &st.queue.len().to_string())],
            )
        }
        "vol" => vec![Do::Volume(
            arg.trim_end_matches('%').parse::<u8>().ok().map(|v| v.min(100)),
        )],
        "play" => vec![Do::Play(rep("ok", &c.reply, &[]))],
        "pause" => vec![Do::Pause(rep("ok", &c.reply, &[]))],
        "cmds" => {
            let list: Vec<String> = (cfg.commands.all().iter())
                .filter(|(_, c)| c.enabled && m.who >= c.who)
                .map(|(_, c)| cmd_name(c))
                .collect();
            reply("ok", &c.reply, &[("commands", &list.join(", "))])
        }
        "togglesr" => {
            let state = if cfg.open { "disabled" } else { "enabled" };
            let mut out = vec![Do::Open(!cfg.open)];
            out.extend(reply("ok", &c.reply, &[("state", state)]));
            out
        }
        _ => match (&st.current, &current) {
            (Some(t), Some(vars)) => vec![Do::Ban(t.id.clone()), Do::Skip(rep("ok", &c.reply, vars))],
            _ => nothing_playing(),
        },
    }
}

/// Limits a request has to pass before and after the player lookup.
fn limits(cfg: &Requests, st: &State, platform: &str, user: &str, now: u64) -> Option<Reply> {
    let (r, u) = (&cfg.replies, ("user", user));
    if let Some(t) = st.last.get(&key(platform, user)) {
        let left = (t + cfg.cooldown_s).saturating_sub(now);
        if left > 0 {
            return Some(out("ssr", "cooldown", &r.cooldown, &[u, ("cd", &when(left))]));
        }
    }
    let mine = st
        .queue
        .iter()
        .filter(|q| q.platform == platform && same(&q.user, user))
        .count();
    if cfg.max_per_user > 0 && mine >= cfg.max_per_user {
        let max = cfg.max_per_user.to_string();
        return Some(out("ssr", "full", &r.max_per_user, &[u, ("maxreq", &max)]));
    }
    if cfg.max_queue > 0 && st.queue.len() >= cfg.max_queue {
        return Some(out("ssr", "full", &r.full, &[u]));
    }
    None
}

/// The player found `track` for a request: queue it, or say why not. Main.rs enqueues it on the
/// player when the result is "queued".
pub fn add(cfg: &Requests, st: &mut State, platform: &'static str, user: &str, track: Track, now: u64) -> Reply {
    if let Some(no) = limits(cfg, st, platform, user, now) {
        return no;
    }
    let r = &cfg.replies;
    let (song, max, pos) = (
        track.song(),
        format!("{} minutes", cfg.max_minutes),
        format!("#{}", st.queue.len() + 1),
    );
    let mut v = vec![
        ("user", user),
        ("artist", track.artist.as_str()),
        ("single_artist", first_artist(&track.artist)),
        ("title", track.title.as_str()),
        ("song", song.as_str()),
        ("url", track.url.as_str()),
    ];
    let artists: Vec<&str> = track.artist.split(',').map(str::trim).collect();
    let (result, t) = if cfg.max_minutes > 0 && track.seconds > cfg.max_minutes * 60 {
        v.push(("maxlength", &max));
        ("toolong", &r.too_long)
    } else if cfg.blocked_artists.iter().any(|b| artists.iter().any(|a| same(a, b))) {
        ("blocked", &r.blocked_artist)
    } else if cfg
        .blocked_songs
        .iter()
        .any(|b| same(b, &track.id) || same(b, &track.title) || same(b, &song))
    {
        ("blocked", &r.blocked_song)
    } else if st.queue.iter().any(|q| q.track.id == track.id) {
        ("duplicate", &r.in_queue)
    } else {
        st.last.insert(key(platform, user), now);
        st.queue.push(Request {
            platform,
            user: user.into(),
            track: track.clone(),
        });
        v.push(("pos", &pos));
        ("queued", &cfg.commands.ssr.reply)
    };
    out("ssr", result, t, &v)
}

/// The player couldn't queue what `add` just took: forget it, cooldown included.
pub fn undo_add(st: &mut State) {
    if let Some(q) = st.queue.pop() {
        st.last.remove(&key(q.platform, &q.user));
    }
}

/// The player found nothing for a request.
pub fn not_found(cfg: &Requests, user: &str, query: &str) -> Reply {
    out(
        "ssr",
        "notfound",
        &cfg.replies.not_found,
        &[("user", user), ("query", query)],
    )
}

/// A player call for `command` failed; `errormsg` says how.
pub fn player_error(cfg: &Requests, command: &'static str, user: &str, errormsg: &str) -> Reply {
    let c = cfg
        .commands
        .all()
        .into_iter()
        .find(|(n, _)| *n == command)
        .map_or(String::new(), |(_, c)| cmd_name(c));
    let vars = [("user", user), ("cmd", c.as_str()), ("errormsg", errormsg)];
    out(command, "error", &cfg.replies.player_error, &vars)
}

/// Reply for `Do::Volume`, once the player says what the volume is.
pub fn vol_reply(cfg: &Requests, user: &str, vol: u8) -> Reply {
    out(
        "vol",
        "ok",
        &cfg.commands.vol.reply,
        &[("user", user), ("vol", &vol.to_string())],
    )
}

/// A new song started (None: nothing playing). Its request leaves the queue, skip votes reset.
pub fn song_changed(st: &mut State, track: Option<Track>) {
    st.votes.clear();
    st.current_by.clear();
    if let Some(t) = &track {
        if let Some(i) = st.queue.iter().position(|q| q.track.id == t.id) {
            st.current_by = st.queue.remove(i).user;
        }
    }
    st.current = track;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, artist: &str, seconds: u32) -> Track {
        Track {
            id: id.into(),
            title: format!("Song {id}"),
            artist: artist.into(),
            seconds,
            url: String::new(),
        }
    }

    fn msg<'a>(user: &'a str, who: Who, text: &'a str) -> Msg<'a> {
        Msg {
            platform: "twitch",
            user,
            who,
            text,
        }
    }

    /// The (result, text) of every reply, wherever it is.
    fn replies(out: &[Do]) -> Vec<(&str, &str)> {
        out.iter()
            .filter_map(|d| match d {
                Do::Reply(r) | Do::Skip(r) | Do::Play(r) | Do::Pause(r) => Some((r.result, r.text.as_str())),
                _ => None,
            })
            .collect()
    }

    fn say(cfg: &Requests, st: &mut State, user: &str, who: Who, text: &str) -> Vec<(String, String)> {
        let out = handle(cfg, st, &msg(user, who, text), 0);
        replies(&out).into_iter().map(|(a, b)| (a.into(), b.into())).collect()
    }

    fn one(r: Vec<(String, String)>) -> (String, String) {
        assert_eq!(r.len(), 1, "{r:?}");
        r.into_iter().next().unwrap()
    }

    /// !ssr through to the queue, like main.rs does it.
    fn request(cfg: &Requests, st: &mut State, user: &str, t: Track, now: u64) -> (&'static str, String) {
        let out = handle(cfg, st, &msg(user, Who::Followers, "!ssr anything"), now);
        let r = match out.into_iter().next() {
            Some(Do::Search(_)) => add(cfg, st, "twitch", user, t, now),
            Some(Do::Reply(r)) => r,
            d => panic!("{d:?}"),
        };
        (r.result, r.text)
    }

    #[test]
    fn templates() {
        let t = "@{user} {single_artist} - {title} {{requested by @{req}}}";
        let v = [("user", "Ana"), ("single_artist", "A"), ("title", "T"), ("req", "Bob")];
        assert_eq!(fill(t, &v), "@Ana A - T requested by @Bob");
        let v = [("user", "Ana"), ("single_artist", "A"), ("title", "T"), ("req", "")];
        assert_eq!(fill(t, &v), "@Ana A - T");
        assert_eq!(fill("{unknown} {user}", &[("user", "x")]), "{unknown} x");
        assert_eq!(Who::twitch(4, false), Who::Broadcaster);
        assert_eq!(Who::twitch(1, true), Who::Subs);
        assert_eq!(Who::twitch(1, false), Who::Followers);
    }

    #[test]
    fn every_command_answers() {
        let mut cfg = Requests::default();
        let mut st = State::default();
        let s = &mut st;
        // Not a command, or not ours: nothing.
        assert!(handle(&cfg, s, &msg("Ana", Who::Followers, "hello !song"), 0).is_empty());
        assert!(handle(&cfg, s, &msg("Ana", Who::Followers, "!other"), 0).is_empty());
        // Every command, with an empty queue and nothing playing.
        let empty = [
            ("!ssr", "@Ana please specify a song to add to the queue."),
            ("!song", "@Ana nothing is playing right now."),
            ("!next", "@Ana the queue is empty."),
            ("!skip", "@Ana nothing is playing right now."),
            ("!voteskip", "@Ana nothing is playing right now."),
            ("!remove", "@Ana you have no songs in the current queue."),
            ("!remove 3", "@Ana there's no request 3 in the queue."),
            ("!remove @bob", "@Ana there's no request @bob in the queue."),
            ("!pos", "@Ana you have no songs in the current queue."),
            ("!queue", "@Ana the queue is empty."),
            ("!bansong", "@Ana nothing is playing right now."),
        ];
        for (text, want) in empty {
            assert_eq!(one(say(&cfg, s, "Ana", Who::Broadcaster, text)).1, want, "{text}");
        }
        // Permissions: every command says who may use it.
        let (res, t) = one(say(&cfg, s, "Ana", Who::Followers, "!skip"));
        assert_eq!(
            (res.as_str(), t.as_str()),
            ("blocked", "@Ana only mods or higher can use !skip.")
        );
        cfg.commands.ssr.who = Who::Subs;
        assert_eq!(
            one(say(&cfg, s, "Ana", Who::Followers, "!ssr x")).1,
            "@Ana only subs or higher can use !ssr."
        );
        let out = handle(&cfg, s, &msg("Vic", Who::Vips, "!ssr x"), 0);
        assert_eq!(out, vec![Do::Search(Query::Text("x".into()))]);
        // Renamed, case-insensitive, with the trailing U+034F some clients add.
        cfg.commands.ssr.trigger = "!sr".into();
        let out = handle(&cfg, s, &msg("Vic", Who::Vips, "!SR never gonna \u{34f}"), 0);
        assert_eq!(out, vec![Do::Search(Query::Text("never gonna".into()))]);
        // Disabled.
        cfg.commands.ssr.enabled = false;
        assert_eq!(
            one(say(&cfg, s, "Ana", Who::Mods, "!sr x")).1,
            "@Ana the command !sr is not enabled."
        );
        // !cmds lists what this user may use.
        assert_eq!(
            one(say(&cfg, s, "Ana", Who::Followers, "!cmds")).1,
            "Active Songify commands: !song, !next, !voteskip, !remove, !pos, !queue, !cmds"
        );
        // Player calls carry their reply, for when they worked.
        assert_eq!(
            handle(&cfg, s, &msg("M", Who::Mods, "!vol 150%"), 0),
            vec![Do::Volume(Some(100))]
        );
        assert_eq!(
            handle(&cfg, s, &msg("M", Who::Mods, "!vol loud"), 0),
            vec![Do::Volume(None)]
        );
        assert_eq!(vol_reply(&cfg, "M", 40).text, "Volume at 40%");
        assert!(
            matches!(&handle(&cfg, s, &msg("M", Who::Mods, "!pause"), 0)[..], [Do::Pause(r)] if r.text == "Playback stopped.")
        );
        let e = player_error(&cfg, "pause", "M", "no player connected");
        assert_eq!(
            (e.result, e.text.as_str()),
            ("error", "@M !pause didn't work: no player connected")
        );
        let out = handle(&cfg, s, &msg("M", Who::Broadcaster, "!togglesr"), 0);
        assert_eq!(out[0], Do::Open(false));
        assert_eq!(replies(&out), [("ok", "Song requests are now disabled")]);
    }

    #[test]
    fn request_text() {
        let v = |id: &str| Some(Query::Video(id.into()));
        assert_eq!(Query::parse("never gonna"), Some(Query::Text("never gonna".into())));
        assert_eq!(Query::parse("despacito"), Some(Query::Text("despacito".into())));
        assert_eq!(Query::parse("dQw4w9WgXcQ"), v("dQw4w9WgXcQ"));
        assert_eq!(
            Query::parse("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1"),
            v("dQw4w9WgXcQ")
        );
        assert_eq!(
            Query::parse("https://music.youtube.com/watch?list=x&v=dQw4w9WgXcQ"),
            v("dQw4w9WgXcQ")
        );
        assert_eq!(Query::parse("https://youtu.be/dQw4w9WgXcQ?si=abc"), v("dQw4w9WgXcQ"));
        assert_eq!(Query::parse("https://youtube.com/shorts/dQw4w9WgXcQ"), v("dQw4w9WgXcQ"));
        let sp = "https://open.spotify.com/intl-pt/track/4PTG3Z6ehGkBFwjybzWkR8?si=1";
        assert_eq!(Query::parse(sp), Some(Query::Spotify(sp.into())));
        let sp = "https://open.spotify.com/track/4PTG3Z6ehGkBFwjybzWkR8";
        assert_eq!(
            Query::parse("spotify:track:4PTG3Z6ehGkBFwjybzWkR8"),
            Some(Query::Spotify(sp.into()))
        );
        for bad in [
            "https://open.spotify.com/album/1",
            "https://open.spotify.com/playlist/1",
            "https://www.youtube.com/playlist?list=PL1",
            "https://soundcloud.com/a/b",
            "https://youtu.be/short",
        ] {
            assert_eq!(Query::parse(bad), None, "{bad}");
        }
        let cfg = Requests::default();
        let mut st = State::default();
        let want = "@Ana that link isn't a YouTube or Spotify song.";
        assert_eq!(
            one(say(&cfg, &mut st, "Ana", Who::Followers, "!ssr https://x.com/y")).1,
            want
        );
        // A request the player couldn't queue is forgotten, cooldown too.
        let cfg = Requests {
            cooldown_s: 60,
            ..Default::default()
        };
        assert_eq!(request(&cfg, &mut st, "Ana", track("a", "X", 1), 0).0, "queued");
        undo_add(&mut st);
        assert!(st.queue.is_empty());
        assert_eq!(request(&cfg, &mut st, "Ana", track("a", "X", 1), 1).0, "queued");
    }

    #[test]
    fn queue_and_limits() {
        let mut cfg = Requests {
            max_per_user: 2,
            max_queue: 3,
            cooldown_s: 30,
            ..Default::default()
        };
        cfg.blocked_artists.push("bad band".into());
        cfg.blocked_songs.push("Song b2".into());
        cfg.blocked_users.push("troll".into());
        let mut st = State::default();
        let s = &mut st;
        let want = "Rick Astley - Song a1 requested by @Ana has been added to the queue (#1).";
        assert_eq!(
            request(&cfg, s, "Ana", track("a1", "Rick Astley", 200), 0),
            ("queued", want.into())
        );
        // Cooldown, then the per-user limit.
        let want = "@ana you have to wait 20 seconds before you can request a song again.";
        assert_eq!(
            request(&cfg, s, "ana", track("a2", "X", 1), 10),
            ("cooldown", want.into())
        );
        assert_eq!(request(&cfg, s, "Ana", track("a2", "X", 1), 40).0, "queued");
        let want = "@Ana maximum number of songs in queue reached (2).";
        assert_eq!(request(&cfg, s, "Ana", track("a3", "X", 1), 80), ("full", want.into()));
        // Same user name on TikTok is someone else.
        assert_eq!(add(&cfg, s, "tiktok", "Ana", track("t1", "X", 1), 0).result, "queued");
        // Queue full, too long, blocked, duplicate, blocked user, closed.
        assert_eq!(
            request(&cfg, s, "Bob", track("b1", "X", 1), 0),
            ("full", "@Bob the queue is full.".into())
        );
        cfg.max_queue = 0;
        let want = "@Bob the song you requested exceeded the maximum song length (10 minutes).";
        assert_eq!(
            request(&cfg, s, "Bob", track("b1", "X", 601), 0),
            ("toolong", want.into())
        );
        let want = "@Bob Artist: Feat, Bad Band has been blocked by the broadcaster.";
        assert_eq!(
            request(&cfg, s, "Bob", track("b1", "Feat, Bad Band", 1), 0),
            ("blocked", want.into())
        );
        let want = "@Bob the song: X - Song b2 has been blocked by the broadcaster.";
        assert_eq!(
            request(&cfg, s, "Bob", track("b2", "X", 1), 0),
            ("blocked", want.into())
        );
        let want = "@Bob this song is already in the queue.";
        assert_eq!(
            request(&cfg, s, "Bob", track("a1", "X", 1), 0),
            ("duplicate", want.into())
        );
        let want = "@Troll you can't request songs.";
        assert_eq!(
            request(&cfg, s, "Troll", track("c", "X", 1), 0),
            ("blocked", want.into())
        );
        assert_eq!(not_found(&cfg, "Bob", "zzz").text, "@Bob no track found for \"zzz\".");
        cfg.open = false;
        assert_eq!(
            request(&cfg, s, "Bob", track("b3", "X", 1), 0),
            ("closed", "@Bob song requests are closed.".into())
        );

        // !pos, !queue, !next.
        let want = "@Ana #1 Rick Astley - Song a1 | #2 X - Song a2";
        assert_eq!(one(say(&cfg, s, "Ana", Who::Followers, "!pos")).1, want);
        let want = "#1 Rick Astley - Song a1 (@Ana) | #2 X - Song a2 (@Ana) | #3 X - Song t1 (@Ana)";
        assert_eq!(one(say(&cfg, s, "Bob", Who::Followers, "!queue")).1, want);
        assert_eq!(
            one(say(&cfg, s, "Bob", Who::Followers, "!next")).1,
            "@Bob Rick Astley - Song a1"
        );

        // !remove: a viewer removes their own last one; arguments are for mods only.
        let out = handle(&cfg, s, &msg("Ana", Who::Followers, "!remove 1"), 0);
        assert!(matches!(&out[0], Do::Removed(q) if q.track.id == "a2"));
        assert_eq!(
            replies(&out),
            [("ok", "Ana your previous request (X - Song a2) will be skipped.")]
        );
        let out = handle(&cfg, s, &msg("Mia", Who::Mods, "!remove #2"), 0);
        assert!(matches!(&out[0], Do::Removed(q) if q.track.id == "t1"));
        assert_eq!(
            replies(&out),
            [("ok", "The request X - Song t1 requested by @Ana has been removed.")]
        );

        // The request starts playing: off the queue, and !song names the requester.
        song_changed(s, Some(track("a1", "Rick Astley, Other", 200)));
        assert!(s.queue.is_empty());
        let want = "@Bob Rick Astley - Song a1 requested by @Ana";
        assert_eq!(one(say(&cfg, s, "Bob", Who::Followers, "!song")).1, want);
        song_changed(s, Some(track("z", "Radio", 200)));
        assert_eq!(
            one(say(&cfg, s, "Bob", Who::Followers, "!song")).1,
            "@Bob Radio - Song z"
        );
        let out = handle(&cfg, s, &msg("Mia", Who::Mods, "!bansong"), 0);
        assert_eq!(out[0], Do::Ban("z".into()));
        assert_eq!(
            replies(&out),
            [("ok", "The song Radio - Song z has been added to the blocklist.")]
        );
        let out = handle(&cfg, s, &msg("Mia", Who::Mods, "!skip"), 0);
        assert_eq!(replies(&out), [("ok", "@Mia skipped the current song.")]);
    }

    #[test]
    fn voteskip() {
        let cfg = Requests {
            votes_needed: 2,
            ..Default::default()
        };
        let mut st = State::default();
        let s = &mut st;
        song_changed(s, Some(track("s", "X", 1)));
        assert_eq!(
            one(say(&cfg, s, "Ana", Who::Followers, "!voteskip")).1,
            "@Ana voted to skip the current song. (1/2)"
        );
        // Voting twice counts once.
        let want = "@ana you already voted to skip this song. (1/2)";
        assert_eq!(
            one(say(&cfg, s, "ana", Who::Followers, "!voteskip")),
            ("duplicate".into(), want.into())
        );
        // A new song resets the votes.
        song_changed(s, Some(track("s2", "X", 1)));
        assert_eq!(
            one(say(&cfg, s, "Ana", Who::Followers, "!voteskip")).1,
            "@Ana voted to skip the current song. (1/2)"
        );
        let out = handle(&cfg, s, &msg("Bob", Who::Followers, "!voteskip"), 0);
        assert!(matches!(out[1], Do::Skip(_)));
        let want = [
            ("ok", "@Bob voted to skip the current song. (2/2)"),
            ("ok", "Skipping song by vote..."),
        ];
        assert_eq!(replies(&out), want);
    }
}
