//! Spotify's desktop app as the player, without its Web API (which needs the app owner to have
//! Premium): the playing song, play/pause and skip, as media keys would. Linux: MPRIS over D-Bus,
//! woken by its signals. Windows: the Spotify window's title ("Artist - Title" while playing).
//! Song requests can't go to Spotify this way: no local interface queues a song.

use crate::pear::Event;
use crate::{say, status, Config, Player};
use std::time::Duration;
use tokio::sync::{mpsc, watch};

pub const NO_REQUESTS: &str = "song requests need Pear Desktop (Spotify only queues songs through its Web API, \
which needs Premium)";

/// Wait for another player to be picked (returns () so wait_for's guard isn't held across awaits).
async fn unpicked(cfg: &mut watch::Receiver<Config>) {
    let _ = cfg.wait_for(|c| c.player != Player::Spotify).await;
}

/// Stay idle until Spotify is the player, then follow it until another one is picked.
pub async fn run(mut cfg: watch::Receiver<Config>, tx: mpsc::UnboundedSender<Event>) {
    loop {
        let _ = cfg.wait_for(|c| c.player == Player::Spotify).await;
        say("spotify: following the Spotify app".into());
        tokio::select! {
            _ = follow(&tx) => {}
            _ = unpicked(&mut cfg) => {}
        }
        let _ = tx.send(Event::Song(None));
        status(|s| s.player = "off".into());
    }
}

#[cfg(target_os = "linux")]
pub use linux::{follow, player};

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::commands::Track;
    use futures_util::StreamExt;
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedValue, Value};
    use zbus::{Connection, MatchRule, MessageStream};

    const NAME: &str = "org.mpris.MediaPlayer2.spotify";
    const PATH: &str = "/org/mpris/MediaPlayer2";
    const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

    type Props = HashMap<String, OwnedValue>;

    /// While playing, the position is counted here once a second (MPRIS doesn't push it).
    fn tick(playing: bool) -> Duration {
        Duration::from_secs(if playing { 1 } else { 3600 })
    }

    pub async fn follow(tx: &mpsc::UnboundedSender<Event>) {
        let conn = match Connection::session().await {
            Ok(c) => c,
            Err(e) => return say(format!("spotify: no D-Bus session ({e})")),
        };
        // Spotify starting or quitting, and any signal it sends (PropertiesChanged, Seeked).
        let owner = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")
            .and_then(|b| b.member("NameOwnerChanged"))
            .and_then(|b| b.add_arg(NAME))
            .map(|b| b.build());
        let signals = MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender(NAME)
            .and_then(|b| b.path(PATH))
            .map(|b| b.build());
        let (Ok(owner), Ok(signals)) = (owner, signals) else {
            return;
        };
        let owner = MessageStream::for_match_rule(owner, &conn, None).await;
        let signals = MessageStream::for_match_rule(signals, &conn, None).await;
        let (Ok(owner), Ok(signals)) = (owner, signals) else {
            return say("spotify: can't listen to D-Bus".into());
        };
        let mut events = futures_util::stream::select(owner, signals);
        let mut song = None::<Track>;
        let mut was_up = true;
        loop {
            let (playing, mut position) = match get_all(&conn).await {
                Some(p) => {
                    if !was_up || song.is_none() {
                        status(|s| s.player = "connected (Spotify)".into());
                    }
                    was_up = true;
                    let now = track(&p);
                    if now != song {
                        let _ = tx.send(Event::Song(now.clone()));
                        song = now;
                    }
                    let playing =
                        p.get("PlaybackStatus").and_then(|v| v.downcast_ref::<&str>().ok()) == Some("Playing");
                    let position = p.get("Position").map_or(0, |v| (int(v) / 1_000_000) as u32);
                    let _ = tx.send(Event::State(playing, position));
                    (playing, position)
                }
                None => {
                    if was_up {
                        say("spotify: the Spotify app isn't running".into());
                        status(|s| s.player = "Spotify app not running".into());
                        if song.take().is_some() {
                            let _ = tx.send(Event::Song(None));
                        }
                    }
                    was_up = false;
                    (false, 0)
                }
            };
            // Next signal, or a second of playing.
            loop {
                tokio::select! {
                    e = events.next() => match e {
                        Some(_) => break,
                        None => return,
                    },
                    _ = tokio::time::sleep(tick(playing)) => {
                        position += 1;
                        let _ = tx.send(Event::Position(position));
                    }
                }
            }
        }
    }

    async fn get_all(conn: &Connection) -> Option<Props> {
        let m = conn
            .call_method(
                Some(NAME),
                PATH,
                Some("org.freedesktop.DBus.Properties"),
                "GetAll",
                &(PLAYER,),
            )
            .await
            .ok()?;
        m.body().deserialize().ok()
    }

    /// Signed or unsigned, as players differ.
    fn int(v: &Value) -> i64 {
        match v {
            Value::I64(n) => *n,
            Value::U64(n) => *n as i64,
            Value::I32(n) => *n as i64,
            Value::U32(n) => *n as i64,
            _ => 0,
        }
    }

    fn track(p: &Props) -> Option<Track> {
        let m: Props = p.get("Metadata")?.try_clone().ok()?.try_into().ok()?;
        let s = |k: &str| {
            m.get(k)
                .and_then(|v| v.downcast_ref::<String>().ok())
                .unwrap_or_default()
        };
        let artist: Vec<String> = m
            .get("xesam:artist")
            .and_then(|v| v.try_clone().ok()?.try_into().ok())
            .unwrap_or_default();
        let title = s("xesam:title");
        if title.is_empty() {
            return None;
        }
        let id = m.get("mpris:trackid").and_then(|v| match &**v {
            Value::ObjectPath(p) => Some(p.to_string()),
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        });
        let id = id.unwrap_or_default();
        // Free accounts' ads ("/com/spotify/ad/..."): not a song.
        if !id.starts_with("/com/spotify/track/") {
            return None;
        }
        let id = id.rsplit('/').next().unwrap_or_default().to_string();
        Some(Track {
            url: format!("https://open.spotify.com/track/{id}"),
            id,
            title,
            artist: artist.join(", "),
            seconds: m.get("mpris:length").map_or(0, |v| (int(v) / 1_000_000) as u32),
            // Older clients gave a link that no longer works.
            cover: s("mpris:artUrl").replace("https://open.spotify.com/image/", "https://i.scdn.co/image/"),
        })
    }

    /// "next", "play" or "pause".
    pub async fn player(what: &str) -> Result<(), String> {
        let method = match what {
            "next" => "Next",
            "play" => "Play",
            _ => "Pause",
        };
        let conn = Connection::session().await.map_err(|e| e.to_string())?;
        conn.call_method(Some(NAME), PATH, Some(PLAYER), method, &())
            .await
            .map(drop)
            .map_err(|_| "the Spotify app isn't running".into())
    }
}

#[cfg(windows)]
pub use windows::{follow, player};

#[cfg(windows)]
mod windows {
    use super::*;
    use crate::commands::Track;
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, MAX_PATH};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, IsWindow, SendMessageW, WM_APPCOMMAND,
    };

    // ponytail: polls the window title every second (Songify does too); a WinEvent hook on its
    // name changes would need its own message-loop thread.
    pub async fn follow(tx: &mpsc::UnboundedSender<Event>) {
        let (mut song, mut playing, mut position) = (None::<Track>, false, 0u32);
        let mut was_up = true;
        loop {
            match window().map(|w| title(w)) {
                Some(t) => {
                    if !was_up || song.is_none() {
                        status(|s| s.player = "connected (Spotify)".into());
                    }
                    was_up = true;
                    // Paused or idle, the title is just "Spotify", "Spotify Free" or "Spotify Premium".
                    let now = t.split_once(" - ").map(|(artist, title)| Track {
                        id: t.clone(),
                        title: title.into(),
                        artist: artist.into(),
                        ..Default::default()
                    });
                    let now_playing = now.is_some();
                    if now.is_some() && now != song {
                        let _ = tx.send(Event::Song(now.clone()));
                        song = now;
                        position = 0;
                    } else if now_playing {
                        position += 1;
                        let _ = tx.send(Event::Position(position));
                    }
                    if now_playing != playing {
                        playing = now_playing;
                        let _ = tx.send(Event::State(playing, position));
                    }
                }
                None => {
                    if was_up {
                        say("spotify: the Spotify app isn't running".into());
                        status(|s| s.player = "Spotify app not running".into());
                        if song.take().is_some() {
                            let _ = tx.send(Event::Song(None));
                        }
                    }
                    was_up = false;
                    playing = false;
                }
            }
            tokio::time::sleep(Duration::from_secs(if was_up { 1 } else { 5 })).await;
        }
    }

    fn title(w: HWND) -> String {
        let mut buf = [0u16; 512];
        // SAFETY: the buffer outlives the call and its length is passed.
        let n = unsafe { GetWindowTextW(w, buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    /// Spotify's main window: a Chromium-class window of Spotify.exe with a title.
    fn window() -> Option<HWND> {
        unsafe extern "system" fn each(w: HWND, found: LPARAM) -> i32 {
            let mut class = [0u16; 64];
            // SAFETY: buffers outlive the calls, their lengths are passed; `found` is the
            // pointer passed to EnumWindows below, alive for the whole enumeration.
            unsafe {
                let n = GetClassNameW(w, class.as_mut_ptr(), class.len() as i32);
                if !String::from_utf16_lossy(&class[..n.max(0) as usize]).starts_with("Chrome_WidgetWin")
                    || title(w).is_empty()
                {
                    return 1;
                }
                let mut pid = 0;
                GetWindowThreadProcessId(w, &mut pid);
                let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if p.is_null() {
                    return 1;
                }
                let mut path = [0u16; MAX_PATH as usize];
                let mut len = path.len() as u32;
                let ok = QueryFullProcessImageNameW(p, 0, path.as_mut_ptr(), &mut len) != 0;
                CloseHandle(p);
                let exe = String::from_utf16_lossy(&path[..len as usize]).to_ascii_lowercase();
                if ok && exe.ends_with("\\spotify.exe") {
                    *(found as *mut HWND) = w;
                    return 0;
                }
                1
            }
        }
        let mut found: HWND = std::ptr::null_mut();
        // SAFETY: `each` only writes through the pointer to `found`, which outlives the call.
        unsafe {
            EnumWindows(Some(each), &mut found as *mut HWND as LPARAM);
            (!found.is_null() && IsWindow(found) != 0).then_some(found)
        }
    }

    /// "next", "play" or "pause", as the keyboard's media keys.
    pub async fn player(what: &str) -> Result<(), String> {
        // APPCOMMAND_MEDIA_NEXTTRACK, APPCOMMAND_MEDIA_PLAY, APPCOMMAND_MEDIA_PAUSE.
        let cmd: isize = match what {
            "next" => 11,
            "play" => 46,
            _ => 47,
        };
        let w = window().ok_or("the Spotify app isn't running")?;
        // SAFETY: a plain message to a window handle that was just found.
        unsafe { SendMessageW(w, WM_APPCOMMAND, w as usize, cmd << 16) };
        Ok(())
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
pub async fn follow(_tx: &mpsc::UnboundedSender<Event>) {
    say("spotify: the Spotify app can't be followed on this system".into());
    std::future::pending::<()>().await
}

#[cfg(not(any(target_os = "linux", windows)))]
pub async fn player(_what: &str) -> Result<(), String> {
    Err("not supported on this system".into())
}
