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
