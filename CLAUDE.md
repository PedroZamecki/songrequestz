# songrequestz project rules

- Commits: conventional commits, one line, max 72 chars: `type(scope)?: subject`.
  Scope is optional. No body, no footer, no `Co-Authored-By` or other trailers. Enforced by
  commitlint (`.githooks/commit-msg`, `commitlint.config.mjs`) and in CI.
- `main` is protected: no direct pushes. Work on a branch, open a pull request; it's squash-merged,
  so the PR title becomes the commit on `main` and must follow the same commit rule.
  No approvals required.
- One PR at a time, never stacked: merge the open PR before branching the next one from `main`.
- Branch names (suggested, not enforced): `type/short-description`, e.g. `feat/pear-queue`,
  `fix/tray-close`, using the same types as commits.
- Before committing, `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` must pass
  (`.githooks/pre-commit`, and the `lint` workflow on every PR).
- Releases are automatic: each merge to `main` bumps the version from the commit types
  (`type!:` major, `feat` minor, `fix`/`perf` patch) and publishes the Windows exe. Don't tag by hand,
  and don't bump the version in `Cargo.toml` (it's unused).
- Language: English everywhere (code, comments, commits, PRs, docs).
- Push as rarely as possible: every push runs CI. Commit locally, test locally, and push once when the
  work is done (not after every commit or fix).
- No mention of Claude or any other AI agent anywhere: commits, PR titles and descriptions, comments
  (no "Generated with", no "Co-Authored-By").
- Performance is a hard rule: least RAM and CPU possible. If a change can make it even 1% lighter or
  faster, do it; never add a dependency, thread, timer or poll when the OS or existing code covers it.
  Measure hidden RSS (tikstream is ~20 MB; aim lower) and idle CPU before and after UI or runtime changes.
- Windows-only code can be type-checked locally before pushing (official rustc with the Windows std,
  e.g. installed to /tmp/rt): `RUSTC=/tmp/rt/bin/rustc cargo check --target x86_64-pc-windows-msvc
  --features fltk/fltk-bundled --target-dir /tmp/wincheck`.
- `PLAN.md` is the spec: follow it, tick its phases off, and update it when a decision changes.
  Songify (https://github.com/songify-rocks/Songify) is the reference for protocols and formats;
  tikstream (`../tikstream`) is the reference for code style, structure, CI and the Streamer.bot client.
- Never commit secrets: Spotify tokens, Pear token and Streamer.bot password live only in
  `songrequestz.json` (gitignored).
