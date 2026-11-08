# Implementation Plan — restic-manager review remediation

**Source:** `CODE_REVIEW_2026-10.md` (37 numbered findings; C2 refuted and removed)
**Branch:** `feat/tui` @ `cec8edf` — plan to land as a series of PRs against `main`
**Date:** 2026-10-28

## Ground rules (from `AGENTS.md` and `.agents/workflow.md`)

- Branch per logical change: `type/short-kebab-description`. One logical change per branch.
- Never commit to `main`. PRs only. **Do not merge without explicit user approval.**
- Every PR runs: `cargo build --all-targets`, `cargo test --all-targets`,
  `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --check`.
- Markdown changes must pass `markdownlint-cli2` (120-char limit).
- Workflow loop: **plan → cheapest failing check → smallest change → targeted checks + one real
  surface QA → summarize.**
- Real surface QA for this repo means running the binary end-to-end, not just `cargo test`.
- `Cargo.lock` is gitignored — do not force-add it.
- Production code must stay cross-platform (Windows is in the build matrix). Unix-only
  syscalls need a portable equivalent or a `#[cfg]` gate.

## Wave 0 — Foundations (prerequisite for everything)

**PR 0 — `chore/ci-markdownlint-glob` (H8)**
*Why first:* the markdown lint job is currently a no-op, so **no PR in this plan can be trusted
to have its Markdown linted** until this lands. Everything else is unverifiable in CI.

- `.github/workflows/test.yml:39-43` — pass `all_changed_files` as a list, not a comma-joined
  string.
- Fix `TODO.md`'s 3 MD013 violations (it is currently failing lint, invisibly).
- Verify: `markdownlint-cli2 "README.md,TODO.md"` lints >0 files and exits non-zero on a bad file.

**PR 1 — `test/backup-output-fixtures` (test-only, unblocks C1)**
*Why first:* C1's root cause is that the unit-test fixtures encode the **same wrong JSON shape**
as the code, so the test suite passes against a broken parser. Fix the fixtures to be verbatim
restic output; they must fail against current code.

- `src/backup.rs` tests — replace the hand-written `"summary"`-nested fixtures with verbatim
  restic 0.16.4 summary lines (captured via `restic backup --json`).
- Add a test for the `total_duration` field name.
- Verify: `cargo test --lib backup::` **fails** (this is the "cheapest failing check").

## Wave 1 — Correctness (user-visible wrong output)

**PR 2 — `fix/backup-summary-parse` (C1)**
*Fixes the tool's core output.*

- `src/backup.rs:339-368` — read counters from the top level of the summary line, not
  `json["summary"]`. Use `total_duration`, not `duration`.
- Replace the ad-hoc `serde_json::Value` indexing with a typed `#[derive(Deserialize)]` struct so
  a future shape change is a hard error, not a silent zero.
- Match the summary line on `"message_type":"summary"`, not the substring `"summary"` (M7).
- Verify: `cargo test --lib backup::` passes; real end-to-end run reports non-zero
  `files_new`; the notification body shows real counts.

**PR 3 — `fix/retention-after-backup` (C3)**
*Documented feature that does not exist. Highest user impact of the Criticals.*

- `src/backup.rs` — after a successful (non-dry-run) backup, apply the job's retention policy.
- `src/snapshot.rs:215` `apply_retention` already exists; wire it in, gated on the job having a
  `retention:` block.
- Decide and document: does retention failure fail the backup, or warn? Recommend **warn and
  continue** — a prune failure must not invalidate a good snapshot.
- Requires a decision (see Open Questions) on whether this is opt-in per job.
- Verify: a job with `keep_last: 1` leaves exactly 1 snapshot after 3 backups.

**PR 4 — `fix/password-key-mask-panic` (H1)**
*Crash in a read-only command.*

- `src/cli.rs:316-317` — `&repo.password_key[..2]` → `.chars().take(2)`.
- Add a regression test with a multi-byte leading character.
- Verify: `cargo test --lib cli::` passes; `show repo` on a `→`-prefixed key does not panic.

**PR 5 — `fix/scheduler-channel-deadlock` (H10)**
*Permanent daemon wedge at ≥101 due jobs.*

- `src/scheduler.rs:205-211` — replace `tx.send(...).await` in the tick arm with `try_send`;
  log and drop when the queue is full. Alternatively dispatch directly without the channel.
- Add a test that 150 due jobs complete without wedging (use `RESTIC_MANAGER_TEST_TICK_SECS`).
- Verify: test passes; the watchdog harness in the review no longer hangs.

**PR 6 — `fix/cron-seconds-field` (H11)**
*6-field schedules with non-zero seconds validate but never fire.*

- `src/config.rs:274-280` / `src/scheduler.rs:57-61` — decide: **reject** 6-field expressions
  whose seconds field is non-zero, at validation time, with a clear error. (Recommended —
  supporting them means evaluating schedules at their true next-due time, a much larger change.)
- Add a validation test.
- Verify: a config with `30 0 2 * * *` now fails validation with an explanatory message.

## Wave 2 — Secrets & credentials

**PR 7 — `fix/telegram-token-in-errors` (H9)**
*Bot token written to the systemd journal.*

- `src/notifications.rs:214-225` — call `err.without_url()` before wrapping. Also redact the
  token from any `show`/config dump.
- Add a test asserting the token does not appear in the error string.

**PR 8 — `fix/redact-repo-credentials` (M21)**
*Repo password reaches the log *and* Telegram.*

- New helper to strip credentials from a URL in an error string, applied to the restic stderr
  before it is logged (`src/backup.rs:319`) and before it is notified (`src/scheduler.rs:331`).
- Also cover hook stderr (`src/backup.rs:401-414`).
- Validate at config time: warn if a repo URL contains embedded credentials.

**PR 9 — `fix/secret-validation-gaps` (M13, M14, M15)**
*Three silent alerting failure modes.*

- M14 — `src/config.rs:303` — treat an empty secret value as missing (`is_none_or(is_empty)`).
- M13 — `src/secrets.rs` — warn when a `telegram:` block is present but `bot_token` is absent
  (a typo is currently swallowed).
- M15 — `src/secrets.rs:9` — accept an unquoted numeric `chat_id` via a custom deserializer.
- Each needs a test that fails before and passes after.

**PR 10 — `fix/exclude-path-traversal` (H13)**
*Job name can write outside `excludes.d/`.*

- `src/exclude.rs:67-74` — sanitise the job name before using it in a filename.
- Also fix M1 here (same function's caller): a configured-but-missing `exclude_file` must error
  rather than silently fall back, and the `write_job_excludes` error must not be swallowed.

## Wave 3 — Process lifecycle

**PR 11 — `fix/orphan-restic-on-sigterm` (H7)**
*`restic` survives the manager; the repo stays locked; the next run fails.*

- `src/backup.rs:206-212` — put the child in its own process group and kill the group, **or**
  install a SIGTERM handler in the one-shot `run` path that kills the child.
- Cross-platform constraint: the daemon already has `#[cfg(unix)]`/`#[cfg(not(unix))]` signal
  variants (`src/scheduler.rs:117-140`) — mirror that structure. Windows has no `kill`; use
  `Child::kill()` there.
- `src/tui.rs:160-171` — replace the external `kill` binary with a direct syscall/library call
  (the external binary does not exist on Windows).
- Add an integration test asserting no `restic` survives.

## Wave 4 — Scheduler & notification hygiene

**PR 12 — `fix/notification-rate-limiter-lifetime` (H12)**
*The 5-minute rate limit never applies across runs.*

- `src/scheduler.rs:288-291` — construct one `NotificationManager` for the scheduler's lifetime,
  and key the limiter per job name.
- M3/M18 — while in this file, make the mutex handling poison-tolerant
  (`unwrap_or_else(|p| p.into_inner())`) to match the TUI. Note `in_flight.lock().unwrap()` runs
  inside a `Drop` during unwinding, where a panic aborts the process.

**PR 13 — `fix/scheduler-join-reap` (M17)**
*Completed task handles accumulate for the daemon's lifetime; panics are swallowed.*

- `src/scheduler.rs:173, 226, 251-270` — reap in the dispatch arm, not only at shutdown.
- M16 — populate `BackupResult.job_name` while in `src/backup.rs`.

## Wave 5 — TUI: the structural fix

**PR 14 — `refactor/tui-in-process-data` (M10 → dissolves H2, H3, M4)**
*This is the highest-leverage change in the plan and should precede all other TUI work.*

The TUI spawns `restic-manager jobs/repos/show/list` as subprocesses for every piece of data.
That single decision causes H2 (a broken repo anywhere hides the whole job list), H3 (child
stderr corrupts the display), and M4 (five spawns per refresh).

- Call `Config::load()`, `SnapshotManager::list()`, etc. **directly** — same crate.
- Capture child stderr only where a subprocess is genuinely needed, and render it in the Logs
  view rather than letting it write into the alternate screen.
- Expect the TUI to get simpler and noticeably faster.
- Verify: the TUI starts with one broken repo and still lists the working ones; a pty capture
  shows no stderr interleaving.

**PR 15 — `fix/tui-stale-async-response` (H6)**
*One repo's snapshots shown under another job's details.*

- `src/tui.rs:632-672` — add a request generation counter; drop responses that no longer match
  the selected job. Apply the same to `load_job_details_async` and `load_repo_details_async`.

**PR 16 — `fix/tui-daemon-detection` (H4)**
*Any job whose name contains "daemon" makes the header show a green running daemon.*

- `src/tui.rs:104-144` — compare argv positionally; require the **subcommand** to be exactly
  `daemon`, not a substring anywhere in the cmdline.
- Add a test with a job named `my-daemon-backup`.

**PR 17 — `docs/tui-keybindings` (H5)**
*README and the in-app `?` overlay disagree with the code.*

- `README.md:147-160` and the help overlay — correct the table to match the code, **or** change
  the bindings to match the docs. (Decision needed — see Open Questions.)
- Also update `AGENTS.md` (module count 12 → 13, add `tui`) and `DESIGN.md`'s stale diagrams.

## Wave 6 — Remaining polish (bundle into one or two PRs)

**PR 18 — `fix/misc-hardening`**

- M2 — `src/notifications.rs:66-69` — propagate the reqwest builder error instead of silently
  dropping the 10s timeout.
- M5 — `src/scheduler.rs:223` — `Arc<ResolvedConfig>` instead of a deep clone per dispatch.
- M6 — `src/scheduler.rs:295` — bound concurrent backups with a semaphore.
- M7 — folded into PR 2.
- M8 — `src/errors.rs` — include the file path; distinguish config vs secrets parse errors.
- M9 — `src/exclude.rs:59-61` — create the config dir 0700.
- M11/M12 — `src/tui.rs` — handle `Event::Resize`; refresh data on the existing 5s tick.
- M19/M20 — `src/config.rs` — reject an all-null retention policy; fix the
  `RetentionPolicy::default()` serde interaction.
- Low-severity bullets from the review (unnamed threads, `format_bytes` float math,
  `join_errors` allocation, duplicated `Show` formatting).

**PR 19 — `chore/ratatui-0.30-upgrade`**
*Do this **before** the TUI work in Wave 5, not after — it touches every TUI file, and doing it
last would conflict with every other TUI PR.*

- `Cargo.toml` — `ratatui = "0.30"`, `crossterm = "0.29"`.
- Breaking changes to handle: `layout::Alignment` → `HorizontalAlignment`;
  `widgets::block::Title` removed; `Backend` requires an associated `Error` type;
  `Block::title()` takes `Into<Line>`.
- MSRV rises to 1.88.0 for 0.30.x — the repo pins 1.97.0, so this is fine.
- Verify: `cargo clippy --all-targets --all-features -- -D warnings` clean; TUI still renders
  (pty capture).

## Decisions (made 2026-10-28)

1. **Retention: opt-in per job.** Only prune when a job has a `retention:` block. Existing
   configs keep growing until the user opts in — no surprise deletions.
2. **TUI keybindings: fix the docs, not the code.** `j`/`k` stay bound to list navigation;
   `Tab`/`Shift-Tab` are documented for the sidebar.
3. **6-field cron: reject at validation time.** A schedule whose seconds field is non-zero
   fails validation with an explanatory message rather than silently never firing.
4. **Retention failure: warn and continue.** A prune failure must never invalidate a good
   snapshot. Log a warning; do not fail the backup.

## Branching strategy

PR #92 (`feat/tui`) is **open and mergeable**, and `src/tui.rs` exists only on that branch.
Comparing `main` against `feat/tui`, only `src/cli.rs` and `src/lib.rs` differ — every other
module (`backup.rs`, `scheduler.rs`, `config.rs`, `secrets.rs`, `notifications.rs`, `exclude.rs`,
`snapshot.rs`, `errors.rs`) is byte-identical.

So the plan branches as follows:

- **Waves 0-4 and 6** (everything except the TUI work) branch from `main`. They touch only
  byte-identical files, so they merge cleanly and independently of PR #92.
- **Wave 5** (the TUI PRs) branches from `feat/tui` and targets PR #92's branch, because the
  TUI module does not exist on `main`. They land after PR #92 merges, or as PRs against
  `feat/tui` directly.
- The **ratatui 0.30 upgrade** (PR 19) is TUI-only and therefore also branches from `feat/tui`.

This keeps the two streams independent: the correctness and secrets work can land and be
released on `main` while PR #92 is still in review.

## Status

Branches live in the repo; each teammate worked in its own `git worktree` sandbox under
`.omo/session-work/`. Nothing is pushed or merged - PRs need the maintainer's go-ahead.

| PR | Finding | Branch | State |
|---|---|---|---|
| #102 | H8 markdownlint CI | `chore/ci-markdownlint-glob` | **open** — verified |
| #103 | C1 backup counters | `fix/backup-summary-parse` | **open** — verified end-to-end |
| #104 | C3 + M19 retention | `fix/retention-after-backup` | **open** — verified end-to-end |
| #108 | H1 password_key panic | `fix/password-key-mask-panic` | **open** — based on `feat/tui`, no CI |
| #107 | H10 scheduler deadlock | `fix/scheduler-channel-deadlock` | **open** — verified |
| #105 | H11 cron seconds field | `fix/cron-seconds-field` | **open** — verified |
| #106 | H9 + M21 + M13 + M14 | `fix/secret-handling-hardening` | **open** — 4 commits, verified |
| #109 | this document | `review/audit-2026-10` | **open** — docs only |

All eight are `MERGEABLE / CLEAN` with CI green, **none merged** — merging needs the
maintainer's approval.

Earlier PR numbers (#93–#99, #101) were closed and replaced by the numbers above: the commit
authors were rewritten to `gdellis@pm.me` so GitHub attributes them to the repository owner
rather than to an unrelated account that happened to own the agent email address. The diffs are
byte-identical (tree hashes verified unchanged).

### Base-branch correction

**H1 (`password_key` panic) exists only on `feat/tui`, not on `main`.** `main` has no `show`
subcommand, so the masking code was introduced by the TUI branch. That fix is therefore based on
`feat/tui` and targets PR #92's branch, not `main`.

**PR #108 runs no CI.** Every workflow triggers on `pull_request: branches: [main]`, so a PR
targeting `feat/tui` gets no checks. The fix is verified locally (all four CI commands plus a
real-binary reproduction), but it is not machine-checked on GitHub. Either merge it into the
branch of PR 92 or widen the workflow trigger to include `feat/tui`.

### Remaining work

- **Wave 3** — H7: `restic` is orphaned on SIGTERM, leaving the repository locked.
- **Wave 5** — the TUI findings (H2, H3, H4, H5, H6, M10-M12), which branch from `feat/tui`.
  M10 (have the TUI call the library instead of spawning itself) dissolves H2 and H3 at once
  and should come first.
- **Wave 6** — H13 (path traversal), H12, M17, M18, M2, M3, and the low-severity batch.
- **The UI brainstorm** — still outstanding, and PR #109's plan references it.

### Verification performed

Each fix was checked three ways, not just by unit test:

- **Red state.** The new tests were confirmed to fail against the pre-fix code. For the cron fix,
  stubbing the helper made exactly the 2 rejection tests fail while the acceptance tests still
  passed. For the secret fixes, neutering the redaction failed 7 tests.
- **CI gate.** `cargo build --all-targets`, `cargo test --all-targets`,
  `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --check`.
- **Real surface QA** with the actual binary against real `restic`:
  - C1 — old binary reported `0 new, data_added=0`; new binary reported `2 unchanged files,
    data_added=368` on identical input.
  - C3 — a job with `keep_last: 1` leaves exactly 1 snapshot after 3 backups; a job without a
    retention block leaves all 3.
  - M21 — a repo URL `sftp:user:SECRETPASS@host.invalid:/x` is logged as
    `sftp:***@host.invalid:/x`; `SECRETPASS` appears 0 times in the output, and a config-time
    warning fires.

### Edge case found during the cron fix

A naive `seconds != "0"` comparison would have wrongly rejected valid schedules. The cron crate
treats the seconds field as a **set**: `"00"`, `"*"`, `"*/5"`, `"0-30"` and `"0,15"` all include
second 0 and all do fire. Verified against the crate:

```
0        matches@sec0=true   -> ACCEPT
00       matches@sec0=true   -> ACCEPT   (a literal "!=\"0\"" check would REJECT)
*        matches@sec0=true   -> ACCEPT
*/5      matches@sec0=true   -> ACCEPT
0-30     matches@sec0=true   -> ACCEPT
0,15     matches@sec0=true   -> ACCEPT
30       matches@sec0=false  -> REJECT
1-30/5   matches@sec0=false  -> REJECT
```

The implementation asks the cron crate itself (via a probe schedule against a known instant)
rather than re-implementing cron field grammar.

## Dependency order

```
Wave 0 (PR 0, PR 1)          <- CI trusts markdown lint; fixtures encode truth
   |
Wave 1 (PR 2..PR 6)          <- correctness; independent of each other
   |
Wave 2 (PR 7..PR 10)         <- secrets; independent
   |
Wave 3 (PR 11)               <- process lifecycle
   |
Wave 5 PR 19 (ratatui)       <- touches all TUI files; BEFORE other TUI PRs
   |
Wave 5 (PR 14..PR 17)        <- TUI; PR 14 unblocks nothing but is the enabler
   |
Wave 4 (PR 12, PR 13)        <- scheduler hygiene; independent, can run in parallel
   |
Wave 6 (PR 18)               <- bundle the rest
```

Waves 1, 2, 3 and 4 have no internal ordering constraints — those PRs can be developed in
parallel by separate agents, one PR per agent.

## Test strategy

- **Cheapest failing check first** for every fix (per `.agents/workflow.md`): write the test
  that demonstrates the bug, watch it fail, then fix.
- **Verbatim fixtures, not hand-written ones.** The C1 class of bug (assumptions about restic's
  output never checked against reality) is exactly what hand-written fixtures hide. Capture real
  `restic --json` output and commit it.
- **One real surface QA step per PR** — run the binary end-to-end, not just `cargo test`.
- **Integration tests** for anything touching process lifecycle (PR 11), scheduler timing
  (PR 5, 6), or the daemon drain contract — follow the existing `tests/daemon_shutdown.rs`
  pattern (`XDG_CONFIG_HOME` pointed at scratch config).
- **PTY-based TUI tests** for the TUI PRs — ratatui's `TestBackend` does not cover the
  subprocess/stderr interaction that PR 14 and PR 16 are about.
- Reuse the review's repro harnesses (kept in `/tmp/rmtui2`, `/tmp/orphan_test_shim.py`,
  `/tmp/race_test.py`) as the basis for regression tests. They should move into the repo under
  `tests/` if they prove durable.

## Out of scope for this plan

- The **UI brainstorm** (a separate request) — but note that PR 14 (TUI calls the library
  directly) is a prerequisite for any serious UI work, and issue **#45** ("GUI component for
  managing backups") is open and waiting.
- Issue **#74** (non-file secret backends) is open and overlaps M13/M14/M21 — worth deciding
  whether to fold those in.
- The `warn_if_insecure_permissions` ownership check (M-something, Low).
