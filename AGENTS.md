# AGENTS

Single-crate Rust CLI that wraps `restic` (job-based backups, cron scheduling, Telegram
notifications, retention/keep policies, pre/post hooks). `restic` must be on `PATH` to run
anything beyond build/test.

## Layout

- `src/cli.rs` — clap subcommand dispatch (Run, Restore, Prune, List, Check, Unlock, Daemon,
  Jobs, Repos, Init, InitExclude). The Daemon subcommand is the long-running foreground
  process systemd wraps. `src/main.rs` calls into `cli_run` (re-exported from
  `src/lib.rs:14`).
- `src/lib.rs` — module list: `backup`, `cli`, `cli_log`, `config`, `errors`, `exclude`,
  `notifications`, `repository`, `restore`, `scheduler`, `secrets`, `snapshot`. Read this first
  when tracing a command's flow.
- `tests/daemon_shutdown.rs` — Linux-only (`#![cfg(target_os = "linux")]`) end-to-end tests
  of the SIGTERM/SIGINT drain contract. Uses `XDG_CONFIG_HOME` to point at scratch config.
- `tests/integration_test.rs` — the rest of the integration suite.
- `contrib/systemd/` — system + user unit files that ship with the repo. `ExecStart=` assumes
  the binary at `/usr/local/bin/restic-manager`; if `which restic-manager` says otherwise, fix
  the path before enabling or systemd will hit the start limit.
- `DESIGN.md` — design notes (mermaid is out of date vs. the current 12-module layout; trust
  the source).

## Commands (CI runs all four)

```bash
cargo build --all-targets
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
```

CI also lints Markdown with `markdownlint-cli2` (config: `.markdownlint.json`, 120-char line
limit) on any `*.md` file changed by a PR. The pre-commit hook runs the same checks plus
yaml/shellcheck — install with `pre-commit install`; the day-to-day driver is
`pre-commit run --all-files` (or `npx markdownlint-cli2` for markdown only).

Run a single test by name:
`cargo test --test daemon_shutdown daemon_drain_waits_for_in_flight_backup`. Run a single lib
test: `cargo test --lib cli::tests::`.

Coverage: `cargo tarpaulin --all-features --workspace --timeout 120 --out html`. The
`RUST_LOG`-reading tests in `src/cli.rs` are tarpaulin-flaky (tarpaulin runs tests in a single
process with its own thread orchestration, which races on the process-global `RUST_LOG`); the
tests pass in plain `cargo test` and the race is serialized by a `Mutex` in that file. Don't
refactor away the `Mutex` thinking it's dead code.

## Repo quirks

- **`Cargo.lock` is gitignored** (line 2 of `.gitignore`). Deps float; CI resolves fresh.
  Don't `git add -f Cargo.lock`.
- **Toolchain pinned to 1.97.0** in `rust-toolchain.toml` with `clippy` and `rustfmt`
  components. `dtolnay/rust-toolchain@stable` in CI picks this up automatically.
- **`RESTIC_MANAGER_TEST_TICK_SECS` env var** in `src/scheduler.rs:165` is test-only and
  unstable — exists only so integration tests don't have to wait for a real minute boundary.
- **No `cargo new`-style sub-crates.** Single binary, single `Cargo.toml`, all modules live
  under `src/`.
- **First-party actions in `.github/workflows/` are SHA-pinned** (`actions/checkout`
  v5.0.0, `actions/cache` v5.0.0, `actions/upload-artifact` v6.0.0). When bumping, verify the
  new release's `action.yml` actually declares `using: node24` — the v5.0.0 release of
  `actions/upload-artifact` claimed Node 24 but its `action.yml` was not updated, so it needed
  a bump to v6.0.0 to actually fix the deprecation warning.
- **`coverage.yml` and `opencode.yml`** still use floating `@v5`/`@v6.0.1` tags on
  first-party actions. They're v5+ so the deprecation won't fire, but they're an inconsistency
  with the rest. Issue #68 tracks pinning them to SHAs.

## Build matrix

`build.yml` builds on `ubuntu-latest`, `macos-latest`, `windows-latest`. Production code
lives in `src/scheduler.rs` (the daemon) and runs on all three — that's why no
`#[cfg(target_os = "linux")]` gate is needed there. The integration tests in
`tests/daemon_shutdown.rs` *do* need it (and already have one) because they exercise
Linux-specific signal semantics. Don't add Linux-only syscalls in production code without
finding a portable equivalent first.

## Working agreement

- Work on a feature branch; do not commit, merge, or rebase on `main` directly (even with
  `--no-ff`). `main` accepts only PR merges.
- One logical change per branch; smaller PRs review and revert cleanly.
- Branch name: `type/short-kebab-description` (e.g. `fix/log-tick-env`,
  `feat/restic-unlock`).
- Don't update files outside the project folder (no edits to `~/.claude/`,
  `~/.config/opencode/`, etc. — those are personal dotfiles and drift independently of
  this repo).
- Local tool state (`.omo/`, `.opencode/`) is in `.gitignore`; never `git add -f` it.
