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
`songrequestz.json` next to the exe (made on the first run):

- `streamerbot_url`: Streamer.bot's WebSocket address. Empty turns it off.
- `streamerbot_password`: needed when Streamer.bot's WebSocket authentication is on.
- `reply_action`: how chat replies are sent. Pick one:
  - **Empty** (default): songrequestz sends them itself. Streamer.bot only allows that with
    authentication on: **Servers/Clients → WebSocket Server → Enable Authentication**, set a
    password, and put the same password in `streamerbot_password`.
  - **An action name**: songrequestz runs that Streamer.bot action with the reply in `%message%`.
    Make the action with one sub-action, **Twitch → Chat → Send Message to Channel**, text
    `%message%`. Works with authentication off, and you can change how replies are sent there.

With authentication off and no reply action, the log says so on connect and replies fail.
