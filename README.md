# songrequestz

Tiny Songify replacement (one small exe, nothing running while idle): song requests from Twitch chat
(through Streamer.bot) and TikTok chat (through tikstream/TikFinity) for **Pear Desktop** and
**Spotify**. Drop-in for Songify: same port (65530), JSON, WebSocket commands, `Songify.txt`,
`cover.png` and default chat commands. Close Songify first.

Work in progress: see `PLAN.md`.

## Get it

Download `songrequestz.exe` from the latest GitHub release.
Releases are automatic: merging a `feat` or `fix` pull request publishes the next version.

Build it yourself: install Rust from https://rustup.rs, then `cargo build --release`.

## Streamer.bot

songrequestz reads Twitch chat from Streamer.bot (its WebSocket server, on by default at
`ws://127.0.0.1:8080/`) and answers through it, so it never logs into Twitch. Settings are in
`songrequestz.json` next to the exe (made on the first run, new settings added on each start; it's
read when songrequestz starts):

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

## Commands

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
