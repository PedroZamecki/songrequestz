# songrequestz

Tiny Songify replacement (one small exe, nothing running while idle): song requests from Twitch chat
(through Streamer.bot) and TikTok chat (through tikstream/TikFinity) for **Pear Desktop**, and now
playing from the **Spotify** app. Drop-in for Songify: same port (65530), JSON, WebSocket commands, `Songify.txt`,
`cover.png` and default chat commands. Close Songify first.

Work in progress: see `PLAN.md`.

## Get it

Download `songrequestz.exe` from the latest GitHub release.
Releases are automatic: merging a `feat` or `fix` pull request publishes the next version.

Build it yourself: install Rust from https://rustup.rs, then `cargo build --release`.

It starts hidden in the tray (click the icon, or Show/Hide in its menu, for the window). The window
has the status and connections, the request queue (play/pause, skip, remove a request), every
command and reply, the settings and the log. **Save** (Ctrl+S) applies the settings at once, except
the API port (restart). Settings are kept in `songrequestz.json` next to the exe. Closing the
window hides it; Quit is in the window and the tray menu.

## Streamer.bot

songrequestz reads Twitch chat from Streamer.bot (its WebSocket server, on by default at
`ws://127.0.0.1:8080/`) and answers through it, so it never logs into Twitch. Settings (Settings
tab, or `songrequestz.json`):

- `streamerbot_url`: Streamer.bot's WebSocket address. Empty turns it off.
- `streamerbot_password`: needed when Streamer.bot's WebSocket authentication is on.
- `reply_action`: the Streamer.bot action for replies of commands without their own `action`.

How a reply is sent, per command:

- **No action** (default): songrequestz sends it to chat itself. Streamer.bot only allows that with
  authentication on: **Servers/Clients → WebSocket Server → Enable Authentication**, set a password,
  and put the same password in `streamerbot_password`. With authentication off and no actions, the
  log says so on connect.
- **An action** (`requests.commands.<command>.action`, else `reply_action`): songrequestz runs it
  instead, and it decides what happens (chat, TTS, OBS, nothing). It gets these arguments:
  - `%message%`: the reply text. For plain chat replies, one sub-action **Twitch → Chat → Send
    Message to Channel** with text `%message%` is enough. Works with authentication off.
  - `%command%`: `ssr`, `song`, `skip`, ... (its name, even when you renamed the trigger).
  - `%result%`: `ok`, or why not: `blocked`, `closed`, `full`, `cooldown`, `toolong`, `duplicate`,
    `notfound`, `nothing`, `empty`, `disabled`, `error`, `queued` (a request went in).
  - `%platform%`: `twitch` (or `tiktok`), and the reply's placeholders as arguments too
    (`%user%`, `%title%`, `%artist%`, `%song%`, `%pos%`, `%votes%`, ...).

## TikTok

songrequestz reads TikTok chat from [tikstream](https://github.com/PedroZamecki/tikstream) or
TikFinity (their event WebSocket, `ws://127.0.0.1:21213/`), so it never logs into TikTok either.
Nothing goes back to TikTok chat: a command's result runs its own `action`, else `tiktok_action`,
with the same arguments as above and `%platform%` = `tiktok` (`%user%` is the sender's @name). The
action decides: TTS, a Twitch chat message, an overlay, nothing. With no action it's only logged.

- `tiktok_url`: the WebSocket address. Empty turns it off.
- `tiktok_user`: your TikTok @name. TikTok doesn't mark the streamer's own messages, so this is how
  yours count as the broadcaster's. Moderators, subscribers and followers come from TikTok itself;
  a `who` of `followers` keeps out viewers who don't follow you (TikTok only: Twitch chatters all
  count as followers, use Twitch's followers-only chat for that).
- `tiktok_action`: the Streamer.bot action for results of commands without their own `action`.

## Overlays and Songify's API

songrequestz answers Songify's local API on `http://127.0.0.1:65530/` (port in Settings,
applied on restart), so Songify overlays and tools work unchanged:

- `GET /`: the now-playing JSON (same shape as Songify's), `/ws/data`: the same, pushed on every
  change. Songify's widgets (https://songify.rocks/widgets) need `?ws=65530` on their URL.
- WebSocket commands on any other path, e.g. `{"action":"skip"}`: `queue_add` (`data.track`,
  `data.requester`), `skip`/`next`, `play`, `pause`, `play_pause`, `vol_set` (`data.value`),
  `vol_up`, `vol_down`, `send_to_chat`, `sr_enable`/`sr_open`, `sr_disable`/`sr_close`,
  `block_artist`, `block_all_artists`, `block_song`, `block_user`. Answers start with
  `Command executed: `.
- `api_password`: off when empty. With one, pass it as `?password=`, header `X-Songify-Password`,
  on each command (`"password"`), or once with `{"action":"auth","data":{"password":"..."}}`.

Files, written only when they change, in `files_dir` (empty: next to the exe):

- `Songify.txt`: `output` (default `{artist} - {title}`; Songify's placeholders, `{{...}}` only for
  requests, e.g. `{artist} - {title} {{(requested by {req})}}`). While paused: `paused_text` if set
  (`""` empties it).
- `cover.png`: the song's cover (blank while paused when `paused_text` is set).

## Pear Desktop

Install [Pear Desktop](https://github.com/pear-devs/pear-desktop) and turn on its **API Server**
plugin (default port 26538). songrequestz connects by itself, and reconnects whenever Pear starts.
If the API Server asks for authorization, Pear shows a prompt the first time: click **Allow**; the
token is saved in `songrequestz.json` (`pear_token`; empty it to authorize again).

The window's Queue tab removes a request from Pear's queue too (also on the API:
`{"action":"queue_remove","data":{"index":0}}`, 0 = first request; not a Songify command).

Requests go into Pear's queue in request order, after the playing song and ahead of Pear's own
autoplay. `!ssr` takes words to search, a YouTube or YouTube Music link or video id, or a Spotify
track link (looked up by name on YouTube Music; needs `curl`, which Windows 10 and later have).



Songify's chat commands, all on by default. Each one in `requests.commands` has `trigger` (rename
it, without the `!`), `enabled`, `who` (`everyone`, `followers`, `subs`, `vips`, `mods`,
`broadcaster`: that rank or higher), `reply` and `action`.

| command | who | does |
|---|---|---|
| `!ssr <song, link>` | everyone | request a song |
| `!song` | everyone | what's playing, and who asked for it |
| `!next` | everyone | the next request |
| `!pos` | everyone | your requests' places in the queue |
| `!queue` | everyone | the next 5 requests |
| `!remove` | everyone | remove your last request; mods: `!remove 3` (place) or `!remove @user` |
| `!voteskip` | everyone | skip once `votes_needed` people voted |
| `!cmds` | everyone | the commands you can use |
| `!skip` `!play` `!pause` | mods | the player |
| `!vol [0-100]` | mods | show or set the volume |
| `!togglesr` | mods | open or close requests |
| `!bansong` | mods | block the current song and skip it |

Every command answers, also when it can't do anything (nothing playing, empty queue, no
permission, ...). All texts are in `requests.commands.<command>.reply` and `requests.replies`, with
Songify's placeholders (`{user}`, `{cmd}`, `{artist}`, `{title}`, `{song}`, `{req}`, `{pos}`, ...;
text in `{{...}}` only shows when the song was requested), so Songify texts can be pasted in. An
empty text sends nothing to chat (the action still runs).

Limits in `requests`: `open`, `max_queue` and `max_per_user` (0: no limit), `max_minutes` (song
length), `cooldown_s` (between one user's requests), `votes_needed`, and `blocked_users`,
`blocked_artists`, `blocked_songs` (case doesn't matter).

## Spotify

Pick **Spotify app** under Settings → Player to follow the Spotify desktop app instead of Pear.
It needs no account setup and works on Spotify Free: now playing (overlays, `Songify.txt`,
`cover.png`, `!song`), play/pause and skip. Song requests and volume can't work this way: nothing
on the computer can add a song to Spotify's queue, so `!ssr` answers that requests need Pear.
On Windows the song is read from Spotify's window title, so there's no cover or song length.

Spotify's Web API could queue songs, but since March 2026 it only works when the developer app's
owner has Spotify Premium (and controlling playback needs the listener's Premium too); that's a
later phase.
