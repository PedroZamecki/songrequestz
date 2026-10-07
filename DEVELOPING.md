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
