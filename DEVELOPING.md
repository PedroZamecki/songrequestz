# Developing songrequestz

Rules for commits, branches and releases are in `CLAUDE.md` (also `AGENTS.md`); the plan is `PLAN.md`.

## Setup

You need Rust (https://rustup.rs) and Node (for commitlint in the git hooks). Then, once per clone:

```
git config core.hooksPath .githooks
```

## Build and run

```
cargo build --release
./target/release/songrequestz
```

## Test without Streamer.bot: the mock

```
uv run dev/sbmock.py                    # ws://127.0.0.1:8081/, no password
uv run dev/sbmock.py --password secret  # authentication on
```

Set `streamerbot_url` in `target/release/songrequestz.json` to `ws://127.0.0.1:8081/` (and the
password). Type chat lines into the mock: `!song` (viewer Ana), `Bob: !ssr never gonna`,
`Mia(mod+sub): !skip`. It prints every reply (`SendMessage`) and action (`DoAction`) it gets. Like the
real one, `SendMessage` needs authentication on. The real Streamer.bot (`ws://127.0.0.1:8080/`) works
the same way.

## Pear Desktop

Pear has a Linux build: run it with the API Server plugin on (try both with and without its
authorization). Its API is plain HTTP on `http://127.0.0.1:26538/api/v1/` (`song-info`, `queue`,
`search`, ...), handy to compare with what songrequestz does, e.g.
`curl -s http://127.0.0.1:26538/api/v1/queue` (add `-H "Authorization: Bearer <pear_token>"` when
authorization is on).

## Checks (same as the hooks and CI)

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Windows

CI builds the exe on every pull request (artifact `songrequestz-windows` on the run page) and
releases it on merge. To catch Windows-only compile errors from Linux, see tikstream's
`DEVELOPING.md` ("Windows"), then:

```
RUSTC=/tmp/rt/bin/rustc cargo check --target x86_64-pc-windows-msvc --target-dir /tmp/wincheck
```
