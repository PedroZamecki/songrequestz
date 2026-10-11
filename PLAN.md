# songrequestz plan

Tiny Songify replacement, built like tikstream: one small Rust exe, FLTK window + tray, ~nothing idle.
Song requests for **Pear Desktop** and **Spotify** only. It never logs into Twitch: **Streamer.bot owns
every account**; songrequestz talks to Streamer.bot, to tikstream/TikFinity, and to the player.

Drop-in: same port (65530), same JSON, same WebSocket commands, same `Songify.txt`/`cover.png`, same
default chat commands. Close Songify first (same port).

## Decisions (agreed)

| topic | decision |
|---|---|
| Twitch | Subscribe to Streamer.bot's Twitch chat events over its WebSocket; handle commands here; reply with Streamer.bot `SendMessage`. No Twitch login, no SB C# actions needed. |
| TikTok | Connect as a client to tikstream/TikFinity `ws://127.0.0.1:<port>/` (port configurable, default 21213), treat `chat` events like Twitch chat. songrequestz never replies on TikTok: it runs one configurable SB action with the result and SB decides (TTS, Twitch chat, nothing). |
| Players | Pear first (full control, no account). Spotify: the desktop app, no Web API (Spotify's API needs the dashboard app's owner to have Premium since 2026-03-09): now playing, overlay, files, play/pause/skip; no requests (no local interface queues a song). Web API later, for whoever has Premium. |
| Spotify links on Pear | Convert: title/artist from the start of the track's public web page (`<title>`; oEmbed has no artist), fetched with the OS's `curl` (Windows 10+ has it) so the exe carries no TLS; search Pear, queue top hit. YouTube links/IDs go straight in. |
| Commands | Songify defaults, each renamable, enable/disable, own permission (everyone, followers, subs, vips, mods, broadcaster), own optional SB action: `!ssr !song !next !skip !voteskip !remove !pos !queue !vol !play !pause !cmds !togglesr !bansong`. All enabled by default (Songify ships them off). Defaults: everyone requests; mods (and broadcaster) skip/remove any/play/pause/vol/togglesr/bansong; viewers remove only their own. Followers only on TikTok (SB's Twitch chat has no follow info; Twitch's followers-only chat covers it). |
| Replies | Every command always answers (errors, empty queue, nothing playing, no permission, disabled...). Every reply is an editable template with Songify's `{placeholders}` (`{user} {cmd} {title} {artist} {single_artist} {song} {req} {pos} {url} {votes} {vol}`..., `{{...}}` only for requests), so Songify texts paste in unchanged. Empty template = no chat message. |
| Overlay API | Full Songify API (see below). |
| UI | Same as tikstream: FLTK window + tray, starts hidden; Windows exe released by CI; Linux for dev. |
| Repo | Public GitHub `songrequestz`, same rules/CI as tikstream (see `CLAUDE.md`). |
| Not in v1 | Channel-point rewards, Songify cloud/premium, Twitch login, other players (VLC, foobar, Windows playback), history, polls. |

## Protocols (from Songify source)

### Pear Desktop (API Server plugin must be on)
- REST base `http://127.0.0.1:26538/api/v1/`, header `Authorization: Bearer <token>` when auth is on.
- Auth once: `POST http://127.0.0.1:26538/auth/songrequestz` → Pear shows Allow/Deny → `{"accessToken": "..."}`.
  Save in config. 401 later = ask the user to authorize again (button in Settings).
- `GET song-info` → `title artist imageSrc isPaused songDuration(s) elapsedSeconds url videoId playlistId`.
- `GET queue`, `POST queue {videoId, insertPosition: "INSERT_AT_END"|"INSERT_AFTER_CURRENT_VIDEO"}`,
  `DELETE queue/{index}`, `PATCH queue/{index} {toIndex}`.
- `POST search {query}` → top song result (Songify's `YTHCHSearchParser` shows where it is in the JSON).
- `POST next|previous|play|pause`, `GET/POST volume {volume}`, `POST seek-to {seconds}`.
- Push, no polling: `ws://127.0.0.1:26538/api/v1/ws?token=<token>`, messages `{"type": ...}`:
  `VIDEO_CHANGED` (new song), `PLAYER_STATE_CHANGED` (play/pause), `POSITION_CHANGED` (ignore unless needed).

### Spotify desktop app (no account, no Premium)
- Linux: MPRIS on D-Bus (`org.mpris.MediaPlayer2.spotify`): `GetAll` for Metadata (`mpris:trackid`
  `/com/spotify/track/<id>`, ads `/com/spotify/ad/...`), PlaybackStatus, Position; woken by its
  signals (PropertiesChanged, Seeked, NameOwnerChanged for start/quit), position counted once a
  second only while playing. `Next`, `Play`, `Pause`. zbus is already in for the tray.
- Windows: Spotify.exe's Chromium window title, "Artist - Title" while playing ("Spotify",
  "Spotify Free/Premium" paused or idle; no cover, length or id), polled every second like Songify
  does; `WM_APPCOMMAND` media keys (next, play, pause) work on Free.
- Volume and requests answer that the Spotify app can't do them.

### Spotify Web API (later: needs Premium; PKCE, user's own dashboard app)
- Since 2026-03-09 every Development Mode app needs its owner to have Premium (all calls, reads too),
  5 allowlisted users; player writes need the listener's Premium too. Extended quota: companies only.
- User pastes their Client ID in Settings. Redirect URI `http://127.0.0.1:4002/auth` (loopback IP, not
  `localhost`; same as Songify). PKCE in the browser, tiny one-shot listener on 4002, refresh token saved.
- Scopes, minimal: `user-read-currently-playing user-read-playback-state user-modify-playback-state`.
- Read (works without Premium): `GET /v1/me/player/currently-playing`, `GET /v1/search?type=track&limit=1`.
- Write (Premium): `POST /v1/me/player/queue?uri=`, `POST next`, `PUT pause|play`, `PUT volume`.
- Spotify has no push, so poll only while Spotify is the selected player: sleep until the current track
  should end (+1 s), re-check at most every 10 s while playing and 30 s while paused/idle.
- Can't remove from Spotify's queue: removed requests are skipped when they come up (Songify does this).
- Verify the current dashboard rules (dev-mode user allowlist, Premium requirements) when we get there.

### Streamer.bot (WebSocket, default `ws://127.0.0.1:8080/`, optional password)
- Reuse tikstream's `src/streamerbot.rs` (auth, reconnect, `GetActions`, `DoAction`).
- Add: `{"request":"Subscribe","id":"sub","events":{"Twitch":["ChatMessage"]}}` and read
  `{"event":{"source":"Twitch","type":"ChatMessage"},"data":{...}}` (user name, message, roles).
- Replies: `{"request":"SendMessage","id":"msg","platform":"twitch","bot":false,"message":"..."}`.
  Needs SB WebSocket authentication on. Or an SB action: a command's own `action`, else `reply_action`
  (config), run with `message` plus `command result platform` and the reply's parts (`user title pos`...),
  for users who want auth off or their own handling. All documented in README.
- **Check real field names against the running Streamer.bot** (Wine, port 8080) before coding the parser:
  subscribe, type in Twitch chat, dump what arrives. Same for `SendMessage`.
- TikTok results: `DoAction` of the configured action with args
  `platform user result message title artist url pos` (`result`: queued, blocked, full, notfound, error...).

### tikstream / TikFinity (client)
- `ws://127.0.0.1:<port>/`, messages `{"event":"chat","data":{"uniqueId","nickname","comment",
  "isModerator","isSubscriber","followRole",...}}`. Ignore other events. Reconnect every 5 s when down,
  quietly (like tikstream's Streamer.bot client). Empty URL = off.
- Follow-up in tikstream: make its port configurable too (now fixed 21213), separate PR there.

### Songify-compatible server (default port 65530, configurable; `127.0.0.1` only)
- `GET /` → current JSON (503 `{"error":"Payload not available yet."}` before the first song).
  Shape (PascalCase, as Songify): `UserInfo{TwitchUser{Id,Login,BroadcasterType},SpotifyUser{Id,DisplayName,Product}}`,
  `SongifyInfo{Version,Beta}`, `Track{Data{Artists,Title,Albums[{Url,Width,Height}],SongId,DurationMs,
  IsPlaying,Url,DurationPercentage,DurationTotal,Progress,Playlist},CanvasUrl,IsInLikedPlaylist,
  Requester{Name,ProfilePicture}}`, `Queue{Count,Requests,Tracks,songRequests{chat,reward}}`.
  Fill what we know, empty strings/false for the rest. Serialize once per change, share the bytes.
- WebSocket `/ws/data`: push that JSON on every change.
- WebSocket commands (any other path): `{"action": "...", "data": {...}, "password"?}`, answer
  `Command executed: ...`: `auth queue_add{track,requester} skip|next play_pause|play|pause send_to_chat
  sr_enable|sr_open sr_disable|sr_close vol_set vol_up vol_down block_artist block_all_artists block_song
  block_user youtube play_playlist stop_sr_reward` (last three: accept, no-op or Spotify-only).
  Optional password (`?password=`, `X-Songify-Password`, or the `auth` action), off by default.
- Blocklists exist only because the API needs them: three plain string lists in config, matched
  case-insensitively, editable as text in Settings.
- Files next to the exe (folder configurable): `Songify.txt` (template, default `{artist} - {title}`,
  `paused_text` while paused: unset keeps the song, "" empties it, text replaces it) and `cover.png`
  (the image as downloaded, via the OS's `curl`; a blank PNG when paused/none). Write only on change.

## Queue rules
- One list of requests: `{platform, user, track id, title, artist, duration, url}`.
- Limits (config): max queue length, max per user, max song length, cooldown per user, requests open/closed.
- `!ssr` accepts search text, Spotify link/URI, YouTube link/ID. Reply with position.
- `!voteskip`: N distinct voters (config), reset on song change.
- `!pos` = requester's positions; `!queue` = next few titles; `!song` = now playing (+ requester).
- Request is dropped from the list when its song starts playing or is skipped.

## Code layout (like tikstream, fewest files)

| file | what |
|---|---|
| `src/main.rs` | config (`songrequestz.json` next to the exe), shared status, wiring, single-thread tokio |
| `src/commands.rs` | chat → command, permissions, limits, queue, reply templates. Pure, unit-tested |
| `src/pear.rs` / `src/spotify.rs` | the two players; main.rs picks with `Player` (`control`, `volume`) |
| `src/streamerbot.rs` | from tikstream + Subscribe + SendMessage |
| `src/tiktok.rs` | tikstream/TikFinity client |
| `src/server.rs` | port 65530: JSON, `/ws/data`, WS commands, files |
| `src/ui.rs` / `src/tray.rs` | from tikstream: tabs Status, Queue, Commands, Settings, Log |
| `dev/` | `sbmock.py` (tikstream's + fake Twitch chat events + prints SendMessage); TikTok: tikstream's dev mode |

Dependencies: tikstream's minus TikTok/wry/tao/sha2-only-if-needed; add a tiny HTTP client only if
needed. Plain HTTP to Pear can be hand-written over `tokio::net::TcpStream`; Spotify needs TLS,
pick the smallest option (e.g. `ureq` with rustls on a blocking task vs `reqwest`), measure RSS.

## Phases (each = one PR)
- [x] 0. Scaffold: `cargo init`, copy CI from tikstream (`build.yml`, `lint.yml`; drop WebKitGTK and
      login bits), README + DEVELOPING stubs, `git config core.hooksPath .githooks`,
      `gh repo create songrequestz --public`, protect `main` (squash only, no direct push).
- [x] 1. Streamer.bot: connect, subscribe, log real Twitch chat events; `!song`-style echo test via SendMessage.
      Found on SB 1.0.7: `SendMessage` needs SB authentication on (else "Authentication required"), so
      `reply_action` (DoAction with `message`) is the alternative; wrong password = close 4009. Chat:
      `data.user.{name,login,role(1 viewer,2 VIP,3 mod,4 broadcaster),subscribed}`, `data.text`.
      For phase 2: our own replies come back as broadcaster chat (never start a reply with a command),
      and chat clients may append U+034F to repeated messages (trim it).
- [x] 2. Commands + queue in `commands.rs` with unit tests (no I/O). Wired to Twitch chat; player calls
      answer "no player connected yet" until phase 3, which calls `add`/`not_found`/`player_error`/
      `vol_reply`/`song_changed` and removes `Do::Removed` requests from the player queue.
- [x] 3. Pear: auth, song-info, WS push, search, enqueue, skip, remove, volume, Spotify-link conversion.
      Found on Pear: an insert shows in `GET queue` only ~3 s later (it fetches the song first), so a
      request is moved behind the earlier ones once it appears; `GET volume` reads a loudness curve,
      not the slider (slider 40 reads 13), converted with a measured table; search and queue answers
      are 260-500 KB, parsed into typed structs with only the fields used (a `Value` tree peaked at
      ~7 MB). Search results in list form carry no length, so the length limit misses those.
      The request queue lives in memory only (lost on restart).
- [x] 4. Server: Songify JSON, `/ws/data`, WS commands, `Songify.txt`/`cover.png`.
      Checked with Songify's own free widgets (pill-player: title, artist, time, progress live). Widget
      links need `?ws=65530` (their default is 22345; Songify's gallery adds it). Chrome asks to allow
      local network access for widgets loaded from songify.rocks (same as with Songify). `Tracks` =
      the requests (not Pear's whole queue). Payload pushed every second while playing (position).
- [x] 5. TikTok: client, SB result action. (tikstream port-config PR: still to do, after tikstream's
      open PR #2 is merged, since one PR at a time.)
      Tested on a real live through tikstream. TikTok doesn't flag the streamer, so `tiktok_user`
      (their @name) counts as broadcaster. Results run the command's own action, else `tiktok_action`
      (none: only logged). `user` is the sender's @name (`uniqueId`), what TikTok mentions use.
- [x] 6. UI + tray + autostart, polish; measure RSS/CPU. Bring back tikstream's FLTK bits
      in CI (apt packages in `lint.yml`, static C runtime step in `build.yml`), icon + `build.rs`.
      Tabs Status, Queue (play/pause, skip, remove: the window uses the API command channel, plus
      a `queue_remove` action), Commands (all 14 and every reply), Settings, Log. Action fields
      are typed or picked from Streamer.bot's `GetActions` (asked on connect). Chat and API
      changes (open, blocklists) show in the window. Linux, hidden: 11.5 MB RSS but 4.3 MB PSS
      (the rest is shared system libraries), ~2 ticks/min; it was 3.5 MB without the UI.
- [x] 7. Spotify desktop app as a player (no Web API: it needs Premium, see above). Checked on Linux
      with a Free account: now playing, cover, progress, play/pause/skip, quit and restart, switching
      players at runtime; requests and volume answer why not. Windows: type-checked only.
      Known: after its player goes away, the overlay JSON keeps the last song (also with Pear).
- [ ] 8. Spotify Web API, when someone with Premium can test it: PKCE, requests into Spotify's queue,
      skip removed requests when they come up.

## Testing
- Real Streamer.bot is running locally (Wine, `ws://127.0.0.1:8080/`): use it for phase 1 and keep
  checking with it. `dev/sbmock.py` for CI-free repeatable runs.
- Pear Desktop has a Linux build: install it, enable API Server, test for real.
- TikTok: run tikstream with `TIKSTREAM_DEV=1` + `uv run dev/fake.py chat "!ssr never gonna"` (tikstream's).
- Overlays: any existing Songify overlay pointed at `http://127.0.0.1:65530/` must work unchanged.
