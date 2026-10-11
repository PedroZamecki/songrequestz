//! Pear Desktop (its API Server plugin): REST calls over plain HTTP on 127.0.0.1:26538, and its
//! WebSocket for song changes (push, no polling). Spotify links become a Pear search here too.

use crate::commands::Track;
use crate::{say, status, Config, Player};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

const HOST: &str = "127.0.0.1:26538";
const DOWN: &str = "Pear Desktop isn't running, or its API Server plugin is off";
const UNAUTHORIZED: &str = "Pear needs songrequestz authorized (allow it in Pear's prompt)";

/// What the WebSocket task tells main.rs.
pub enum Event {
    /// The song now playing (None: Pear went away).
    Song(Option<Track>),
    /// Pear authorized songrequestz: save this token.
    Token(String),
    /// Playing or paused, and where in the song (seconds).
    State(bool, u32),
    /// Where in the song (seconds).
    Position(u32),
}

/// One request to Pear's API; the body of a 2xx answer, or why not (for chat).
async fn http(method: &str, path: &str, body: Option<Value>, token: &str) -> Result<String, String> {
    let call = async {
        let mut s = TcpStream::connect(HOST).await.map_err(|_| DOWN.to_string())?;
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        let auth = if token.is_empty() {
            String::new()
        } else {
            format!("Authorization: Bearer {token}\r\n")
        };
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {HOST}\r\n{auth}Content-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let mut buf = Vec::new();
        s.write_all(req.as_bytes()).await.map_err(|e| e.to_string())?;
        s.read_to_end(&mut buf).await.map_err(|e| e.to_string())?;
        Ok::<_, String>(buf)
    };
    // Auth waits for a click in Pear's prompt; everything else answers quickly.
    let wait = Duration::from_secs(if path.starts_with("/auth") { 120 } else { 10 });
    let buf = tokio::time::timeout(wait, call)
        .await
        .map_err(|_| "Pear didn't answer".to_string())??;
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    match head.split(' ').nth(1).and_then(|c| c.parse::<u16>().ok()) {
        Some(200..=299) => Ok(body.to_string()),
        Some(401) => Err(UNAUTHORIZED.into()),
        Some(403) if path.starts_with("/auth") => Err("authorization was denied in Pear".into()),
        Some(c) => Err(format!("Pear answered {c}")),
        None => Err("Pear's answer made no sense".into()),
    }
}

async fn api(method: &str, path: &str, body: Option<Value>, token: &str) -> Result<String, String> {
    http(method, &format!("/api/v1/{path}"), body, token).await
}

/// The first song Pear's search finds.
pub async fn search(token: &str, query: &str) -> Result<Option<Track>, String> {
    let body = api("POST", "search", Some(json!({ "query": query })), token).await?;
    Ok(songs(&body)?.into_iter().next())
}

/// A YouTube video by id. ponytail: Pear can't look a video up, so search for its id; if that
/// doesn't find it, it goes in without a title or length (no length limit then).
pub async fn video(token: &str, id: &str) -> Result<Option<Track>, String> {
    let body = api("POST", "search", Some(json!({ "query": id })), token).await?;
    let found = songs(&body)?.into_iter().find(|t| t.id == id);
    Ok(Some(found.unwrap_or_else(|| Track {
        id: id.into(),
        title: id.into(),
        url: watch_url(id),
        ..Default::default()
    })))
}

fn watch_url(id: &str) -> String {
    format!("https://music.youtube.com/watch?v={id}")
}

// Just the parts of Pear's search answer (YouTube Music's, ~260 KB) that songs() reads: serde skips
// the rest without allocating, where a full serde_json::Value tree kept ~5 MB per search.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Search {
    contents: Option<Tabbed>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tabbed {
    tabbed_search_results_renderer: Option<Tabs>,
}
#[derive(Deserialize)]
struct Tabs {
    #[serde(default)]
    tabs: Vec<Tab>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Tab {
    tab_renderer: Option<TabContent>,
}
#[derive(Deserialize)]
struct TabContent {
    content: Option<SectionList>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SectionList {
    section_list_renderer: Option<Sections>,
}
#[derive(Deserialize)]
struct Sections {
    #[serde(default)]
    contents: Vec<Section>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Section {
    music_card_shelf_renderer: Option<Card>,
    music_shelf_renderer: Option<Items>,
    item_section_renderer: Option<Items>,
}
#[derive(Deserialize)]
struct Card {
    title: Option<Runs>,
    subtitle: Option<Runs>,
    thumbnail: Option<Thumbnail>,
}
#[derive(Deserialize)]
struct Items {
    #[serde(default)]
    contents: Vec<Item>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    music_responsive_list_item_renderer: Option<ListItem>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListItem {
    playlist_item_data: Option<VideoId>,
    thumbnail: Option<Thumbnail>,
    #[serde(default)]
    flex_columns: Vec<Column>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Thumbnail {
    music_thumbnail_renderer: Option<ThumbnailRenderer>,
}
#[derive(Deserialize)]
struct ThumbnailRenderer {
    thumbnail: Option<Thumbnails>,
}
#[derive(Deserialize)]
struct Thumbnails {
    #[serde(default)]
    thumbnails: Vec<Url>,
}
#[derive(Deserialize)]
struct Url {
    url: String,
}

impl Thumbnail {
    /// The biggest one (they come smallest first).
    fn url(t: &Option<Thumbnail>) -> &str {
        let t = t
            .as_ref()
            .and_then(|t| t.music_thumbnail_renderer.as_ref()?.thumbnail.as_ref());
        t.and_then(|t| t.thumbnails.last()).map_or("", |u| &u.url)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Column {
    music_responsive_list_item_flex_column_renderer: Option<ColumnText>,
}
#[derive(Deserialize)]
struct ColumnText {
    text: Option<Runs>,
}
#[derive(Deserialize, Default)]
struct Runs {
    #[serde(default)]
    runs: Vec<Run>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Run {
    #[serde(default)]
    text: String,
    navigation_endpoint: Option<Nav>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Nav {
    watch_endpoint: Option<VideoId>,
    browse_endpoint: Option<Browse>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoId {
    video_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Browse {
    browse_endpoint_context_supported_configs: Option<BrowseConfigs>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowseConfigs {
    browse_endpoint_context_music_config: Option<PageType>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageType {
    page_type: Option<String>,
}

impl Run {
    fn video(&self) -> Option<&str> {
        self.navigation_endpoint
            .as_ref()?
            .watch_endpoint
            .as_ref()?
            .video_id
            .as_deref()
    }
    fn page(&self) -> Option<&str> {
        let b = self.navigation_endpoint.as_ref()?.browse_endpoint.as_ref()?;
        b.browse_endpoint_context_supported_configs
            .as_ref()?
            .browse_endpoint_context_music_config
            .as_ref()?
            .page_type
            .as_deref()
    }
}

/// Songs in a search answer, best first: the top card, then the result lists. Language-independent
/// (Pear answers in the user's language): artists are the runs linking to an artist or channel,
/// the length is the run shaped like 3:34.
fn songs(body: &str) -> Result<Vec<Track>, String> {
    let s: Search = serde_json::from_str(body).map_err(|e| format!("Pear's search answer: {e}"))?;
    let sections = (s.contents.and_then(|c| c.tabbed_search_results_renderer))
        .and_then(|t| t.tabs.into_iter().next())
        .and_then(|t| t.tab_renderer?.content?.section_list_renderer)
        .map_or_else(Vec::new, |s| s.contents);
    let mut out = Vec::new();
    for c in sections.iter().filter_map(|s| s.music_card_shelf_renderer.as_ref()) {
        let title = c.title.as_ref().and_then(|t| t.runs.first());
        if let Some(id) = title.and_then(Run::video) {
            let sub = c.subtitle.as_ref().map_or(&[][..], |s| &s.runs);
            out.push(track(
                id,
                title.map_or("", |t| &t.text),
                sub.iter(),
                Thumbnail::url(&c.thumbnail),
            ));
        }
    }
    let lists = sections
        .iter()
        .filter_map(|s| s.music_shelf_renderer.as_ref().or(s.item_section_renderer.as_ref()));
    for i in lists
        .flat_map(|l| &l.contents)
        .filter_map(|i| i.music_responsive_list_item_renderer.as_ref())
    {
        // Albums, artists, playlists have no video.
        let Some(id) = i.playlist_item_data.as_ref().and_then(|p| p.video_id.as_deref()) else {
            continue;
        };
        let title = i
            .flex_columns
            .first()
            .and_then(|c| runs(c).first())
            .map_or("", |r| &r.text);
        let runs = i.flex_columns.iter().skip(1).flat_map(runs);
        out.push(track(id, title, runs, Thumbnail::url(&i.thumbnail)));
    }
    Ok(out)
}

fn runs(c: &Column) -> &[Run] {
    let t = c
        .music_responsive_list_item_flex_column_renderer
        .as_ref()
        .and_then(|r| r.text.as_ref());
    t.map_or(&[], |t| &t.runs)
}

fn track<'a>(id: &str, title: &str, runs: impl Iterator<Item = &'a Run> + Clone, cover: &str) -> Track {
    let artists: Vec<&str> = (runs.clone())
        .filter(|r| {
            matches!(
                r.page(),
                Some("MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL")
            )
        })
        .map(|r| r.text.as_str())
        .collect();
    Track {
        id: id.into(),
        title: if title.is_empty() { id.into() } else { title.into() },
        artist: artists.join(", "),
        seconds: runs.filter_map(|r| seconds(&r.text)).next().unwrap_or(0),
        url: watch_url(id),
        cover: cover.into(),
    }
}

/// "3:34" or "1:02:03".
fn seconds(t: &str) -> Option<u32> {
    if !t.contains(':') {
        return None;
    }
    t.split(':')
        .try_fold(0, |acc, p| Some(acc * 60 + p.parse::<u32>().ok()?))
}

// Pear's queue answer, just what queue() reads (~9 KB a row, 500 KB for 60 rows: a full
// serde_json::Value tree of it peaked at ~7 MB).
#[derive(Deserialize)]
struct Queue {
    #[serde(default)]
    items: Vec<QueueItem>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct QueueItem {
    playlist_panel_video_renderer: Option<Video>,
    playlist_panel_video_wrapper_renderer: Option<Wrapper>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wrapper {
    primary_renderer: Option<Primary>,
    #[serde(default)]
    counterpart: Vec<Counterpart>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Primary {
    playlist_panel_video_renderer: Option<Video>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Counterpart {
    counterpart_renderer: Option<Primary>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Video {
    #[serde(default)]
    video_id: String,
    #[serde(default)]
    selected: bool,
}

/// Video ids in Pear's queue, in its index order (blank for rows that aren't videos), and where the
/// playing one is.
async fn queue(token: &str) -> Result<(Vec<String>, Option<usize>), String> {
    queue_ids(&api("GET", "queue", None, token).await?)
}

fn queue_ids(body: &str) -> Result<(Vec<String>, Option<usize>), String> {
    let q: Queue = serde_json::from_str(body).map_err(|e| format!("Pear's queue answer: {e}"))?;
    let mut playing = None;
    let ids = (q.items.into_iter().enumerate())
        .map(|(i, item)| {
            let v = match (
                item.playlist_panel_video_renderer,
                item.playlist_panel_video_wrapper_renderer,
            ) {
                (Some(v), _) => Some(v),
                (None, Some(w)) => (w.primary_renderer.and_then(|p| p.playlist_panel_video_renderer)).or_else(|| {
                    w.counterpart
                        .into_iter()
                        .next()?
                        .counterpart_renderer?
                        .playlist_panel_video_renderer
                }),
                _ => None,
            };
            let v = v.unwrap_or(Video {
                video_id: String::new(),
                selected: false,
            });
            if v.selected {
                playing = Some(i);
            }
            v.video_id
        })
        .collect();
    Ok((ids, playing))
}

/// Queue a request after the songs requested before it (`earlier`), ahead of Pear's own autoplay.
pub async fn enqueue(token: &str, id: &str, earlier: &[String]) -> Result<(), String> {
    let copies = |ids: &[String], from: usize| ids.iter().skip(from).filter(|i| *i == id).count();
    let (ids, playing) = queue(token).await?;
    let before = copies(&ids, playing.map_or(0, |p| p + 1));
    let body = json!({ "videoId": id, "insertPosition": "INSERT_AFTER_CURRENT_VIDEO" });
    api("POST", "queue", Some(body), token).await?;
    if earlier.is_empty() {
        return Ok(());
    }
    // Pear fetches the song before it shows up in the queue (about 3 s), and says nothing when it
    // does: look until it's there, then move it behind the last earlier request.
    // ponytail: bounded look (10 s), only while a request is going in.
    for _ in 0..33 {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let (ids, playing) = queue(token).await?;
        let from = playing.map_or(0, |p| p + 1);
        if copies(&ids, from) <= before {
            continue;
        }
        let Some(new) = ids.iter().skip(from).position(|i| i == id).map(|n| from + n) else {
            continue;
        };
        // Each earlier request's first copy after the new one: later copies are Pear's own.
        let last = (earlier.iter())
            .filter_map(|e| ids.iter().skip(new + 1).position(|i| i == e))
            .max()
            .map(|n| new + 1 + n);
        return match last {
            Some(to) => api("PATCH", &format!("queue/{new}"), Some(json!({ "toIndex": to })), token)
                .await
                .map(drop),
            None => Ok(()),
        };
    }
    Err("Pear didn't add the song in time".into())
}

/// Take a request off Pear's queue: its first copy after the playing song.
pub async fn remove(token: &str, id: &str) -> Result<(), String> {
    let (ids, playing) = queue(token).await?;
    let from = playing.map_or(0, |p| p + 1);
    match ids.iter().skip(from).position(|i| i == id) {
        Some(n) => api("DELETE", &format!("queue/{}", from + n), None, token)
            .await
            .map(drop),
        None => Ok(()), // already gone from Pear's queue
    }
}

/// "next", "play" or "pause".
pub async fn player(token: &str, what: &str) -> Result<(), String> {
    api("POST", what, None, token).await.map(drop)
}

/// Set the volume (Some), or read it: Pear's slider value, 0-100.
pub async fn volume(token: &str, set: Option<u8>) -> Result<u8, String> {
    if let Some(v) = set {
        api("POST", "volume", Some(json!({ "volume": v })), token).await?;
        return Ok(v);
    }
    let v: Value = serde_json::from_str(&api("GET", "volume", None, token).await?).map_err(|e| e.to_string())?;
    Ok(if v["isMuted"] == true {
        0
    } else {
        slider(v["state"].as_f64().unwrap_or(0.0))
    })
}

/// Pear reads the volume back on a loudness curve, not the slider's scale (slider 40 reads 13).
/// What it reads for slider 0, 5, 10, ... 100, measured on Pear.
const READ: [f64; 21] = [
    0., 1., 2., 3., 5., 6., 8., 11., 13., 16., 20., 24., 29., 34., 40., 47., 55., 64., 74., 86., 100.,
];

/// The slider value for a read one, between the measured points.
fn slider(read: f64) -> u8 {
    let read = read.clamp(0.0, 100.0);
    let i = READ.iter().position(|&r| r >= read).unwrap_or(20).max(1);
    let (lo, hi) = (READ[i - 1], READ[i]);
    (5.0 * (i - 1) as f64 + 5.0 * (read - lo) / (hi - lo)).round() as u8
}

/// "Title Artist" of a Spotify track link, to search Pear with. From the start of the track's web
/// page (its <title>: "Song - song and lyrics by Artist | Spotify"), fetched with the OS's curl
/// (Windows 10+ and Linux have it), so songrequestz carries no TLS code for it.
pub async fn spotify_query(url: &str) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("curl");
    cmd.args(["-sL", "-m", "10", "-r", "0-40000", "-A", "Mozilla/5.0", url]);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = cmd.output().await.map_err(|e| format!("can't run curl ({e})"))?;
    let page = String::from_utf8_lossy(&out.stdout);
    spotify_title(&page).ok_or_else(|| "Spotify didn't say what that song is".into())
}

fn spotify_title(page: &str) -> Option<String> {
    let t = page.split_once("<title>")?.1.split_once("</title>")?.0;
    let t = t.trim_end_matches(" | Spotify");
    let (title, artist) = t
        .split_once(" - song and lyrics by ")
        .or_else(|| t.rsplit_once(" - "))?;
    let unescape = |s: &str| s.replace("&amp;", "&").replace("&#x27;", "'").replace("&quot;", "\"");
    Some(format!("{} {}", unescape(title), unescape(artist)))
}

/// Pear's WebSocket: the playing song as it changes. Reconnects every 5 s while Pear is down, and
/// asks Pear to authorize songrequestz when it wants that.
pub async fn run(mut cfg: watch::Receiver<Config>, tx: mpsc::UnboundedSender<Event>) {
    let mut was_up = true;
    loop {
        if cfg.borrow_and_update().player != Player::Pear {
            let _ = cfg.wait_for(|c| c.player == Player::Pear).await;
            was_up = true;
            continue;
        }
        let token = cfg.borrow_and_update().pear_token.clone();
        let q = if token.is_empty() {
            String::new()
        } else {
            format!("?token={token}")
        };
        let mut unauthorized = false;
        match tokio_tungstenite::connect_async(format!("ws://{HOST}/api/v1/ws{q}")).await {
            Ok((mut ws, _)) => {
                status(|s| s.player = "connected".into());
                loop {
                    tokio::select! {
                        m = ws.next() => match m {
                            Some(Ok(Message::Text(t))) => {
                                    if let Some(e) = message(&t) {
                                        for e in e {
                                            let _ = tx.send(e);
                                        }
                                    }
                                }
                            // Pear closes with 1008 when it wants authorization.
                            Some(Ok(Message::Close(Some(f)))) => unauthorized = u16::from(f.code) == 1008,
                            Some(Ok(_)) => {}
                            _ => break,
                        },
                        _ = changed(&mut cfg, &token) => break,
                    }
                }
                if !unauthorized {
                    say("pear: disconnected".into());
                    status(|s| s.player = "disconnected".into());
                    was_up = true;
                    let _ = tx.send(Event::Song(None));
                }
            }
            Err(_) => {
                if was_up {
                    say(format!("pear: {DOWN}, retrying every 5s"));
                    status(|s| s.player = "not running? (needs its API Server plugin on)".into());
                }
                was_up = false;
            }
        }
        if unauthorized {
            say("pear: asking Pear to authorize songrequestz: click Allow in Pear".into());
            status(|s| s.player = "click Allow in Pear".into());
            match http("POST", "/auth/songrequestz", None, "").await {
                Ok(body) => {
                    let v: Value = serde_json::from_str(&body).unwrap_or_default();
                    if let Some(t) = v["accessToken"].as_str() {
                        say("pear: authorized".into());
                        let _ = tx.send(Event::Token(t.into()));
                        changed(&mut cfg, &token).await;
                        continue;
                    }
                    say("pear: Pear didn't give a token".into());
                    status(|s| s.player = "not authorized (restart to ask again)".into());
                }
                Err(e) => {
                    say(format!("pear: {e}"));
                    status(|s| s.player = "not authorized (restart to ask again)".into());
                }
            }
            // Denied or no answer: don't ask again until the token changes (or a restart).
            changed(&mut cfg, &token).await;
            continue;
        }
        let retry = tokio::time::sleep(Duration::from_secs(5));
        tokio::select! {
            _ = retry => {}
            _ = changed(&mut cfg, &token) => {}
        }
    }
}

/// Wait for a new Pear token or another player. Returns () so wait_for's read guard isn't held
/// across awaits.
async fn changed(cfg: &mut watch::Receiver<Config>, token: &str) {
    let _ = cfg
        .wait_for(|c| c.pear_token != token || c.player != Player::Pear)
        .await;
}

/// The song in a PLAYER_INFO / VIDEO_CHANGED message (same shape as GET song-info).
fn now_playing(s: &Value) -> Option<Track> {
    let id = s["videoId"].as_str().filter(|i| !i.is_empty())?;
    Some(Track {
        id: id.into(),
        title: s["title"].as_str().unwrap_or("").into(),
        artist: s["artist"].as_str().unwrap_or("").into(),
        seconds: s["songDuration"].as_u64().unwrap_or(0) as u32,
        url: s["url"].as_str().map_or_else(|| watch_url(id), String::from),
        cover: s["imageSrc"].as_str().unwrap_or("").into(),
    })
}

/// What a Pear WebSocket message says: the song (PLAYER_INFO when connecting, VIDEO_CHANGED), and
/// play state and position (PLAYER_STATE_CHANGED, POSITION_CHANGED every second while playing).
fn message(t: &str) -> Option<Vec<Event>> {
    let v: Value = serde_json::from_str(t).ok()?;
    let pos = |p: &Value| p.as_f64().unwrap_or(0.0) as u32;
    Some(match v["type"].as_str()? {
        "PLAYER_INFO" | "VIDEO_CHANGED" => {
            let s = &v["song"];
            let state = Event::State(s["isPaused"] != true, pos(&s["elapsedSeconds"]));
            vec![Event::Song(Some(now_playing(s)?)), state]
        }
        "PLAYER_STATE_CHANGED" => vec![Event::State(v["isPlaying"] == true, pos(&v["position"]))],
        "POSITION_CHANGED" => vec![Event::Position(pos(&v["position"]))],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_answers() {
        // Trimmed from a real (Portuguese) Pear answer: top card, then a list with an album row.
        let artist = |name: &str, page: &str| {
            json!({"text": name, "navigationEndpoint": {"browseEndpoint": {"browseEndpointContextSupportedConfigs":
                {"browseEndpointContextMusicConfig": {"pageType": page}}}}})
        };
        let col = |runs: Value| json!({"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": runs}}});
        let v = json!({"contents": {"tabbedSearchResultsRenderer": {"tabs": [{"tabRenderer": {"content":
            {"sectionListRenderer": {"contents": [
            {"musicCardShelfRenderer": {
                "title": {"runs": [{"text": "Never Gonna Give You Up",
                    "navigationEndpoint": {"watchEndpoint": {"videoId": "dQw4w9WgXcQ"}}}]},
                "subtitle": {"runs": [{"text": "Vídeo"}, {"text": " • "}, artist("Rick Astley", "MUSIC_PAGE_TYPE_ARTIST"),
                    {"text": " • "}, {"text": "1,8 bi de visualizações"}, {"text": " • "}, {"text": "3:34"}]}}},
            {"itemSectionRenderer": {"contents": [
                {"musicResponsiveListItemRenderer": {"flexColumns": [col(json!([{"text": "Album"}])),
                    col(json!([{"text": "Single"}, artist("X", "MUSIC_PAGE_TYPE_ARTIST")]))]}},
                {"musicResponsiveListItemRenderer": {"playlistItemData": {"videoId": "GtL1huin9EE"},
                    "flexColumns": [col(json!([{"text": "Ad"}])),
                    col(json!([{"text": "Vídeo"}, artist("CSAA", "MUSIC_PAGE_TYPE_USER_CHANNEL")]))]}}]}}
        ]}}}}]}}});
        let s = songs(&v.to_string()).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(
            (
                s[0].id.as_str(),
                s[0].title.as_str(),
                s[0].artist.as_str(),
                s[0].seconds
            ),
            ("dQw4w9WgXcQ", "Never Gonna Give You Up", "Rick Astley", 214)
        );
        assert_eq!(
            (s[1].id.as_str(), s[1].artist.as_str(), s[1].seconds),
            ("GtL1huin9EE", "CSAA", 0)
        );
        assert!(songs("{}").unwrap().is_empty());
        assert!(songs("not json").is_err());
        assert_eq!(seconds("1:02:03"), Some(3723));
        assert_eq!(seconds("1,8 bi"), None);
        // Measured on Pear: slider 0, 30, 50, 70, 90, 100 read back as these.
        let read = [0.0, 8.0, 13.0, 20.0, 40.0, 74.0, 80.0, 100.0].map(slider);
        assert_eq!(read, [0, 30, 40, 50, 70, 90, 93, 100]);
    }

    #[test]
    fn queue_answer() {
        let v = |id: &str, sel: bool| json!({"videoId": id, "selected": sel, "title": {"runs": [{"text": "x"}]}});
        let body = json!({"items": [
            {"playlistPanelVideoRenderer": v("a", false)},
            {"playlistPanelVideoWrapperRenderer": {"primaryRenderer": {"playlistPanelVideoRenderer": v("b", true)}}},
            {"playlistPanelVideoWrapperRenderer": {"counterpart": [{"counterpartRenderer": {"playlistPanelVideoRenderer": v("c", false)}}]}},
            {"automixPreviewVideoRenderer": {}},
        ]});
        let (ids, playing) = queue_ids(&body.to_string()).unwrap();
        assert_eq!(
            (ids, playing),
            (vec!["a".into(), "b".into(), "c".into(), String::new()], Some(1))
        );
    }

    #[test]
    fn spotify_page() {
        let page =
            "<html><head><title>Don&#x27;t Stop Me Now - Remastered 2011 - song and lyrics by Queen | Spotify</title>";
        assert_eq!(
            spotify_title(page).as_deref(),
            Some("Don't Stop Me Now - Remastered 2011 Queen")
        );
        assert_eq!(spotify_title("<title>Spotify</title>"), None);
    }
}
