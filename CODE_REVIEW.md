# Code Review: restic-manager

Review date: 2026-07-25
Reviewer: Sisyphus (OhMyOpenCode)
Scope: Full Rust codebase (`src/`, `tests/`, CI, dependencies)

## Verification Results

| Check | Command | Result |
|---|---|---|
| Build | `cargo build --all-targets` | Pass |
| Tests | `cargo test --all-targets` | **175 passed / 0 failed** |
| Clippy | `cargo clippy --all-targets --all-features -- -D warnings` | Clean |
| Format | `cargo fmt --check` | Clean |
| Unwrap/expect audit | `cargo clippy ... -W clippy::unwrap_used -W clippy::expect_used` | 88 warnings (test-only or mostly-harmless) |

Overall the project is in good shape: tests pass, CI is clean, the architecture is clear, and there are no critical security issues.

---

## Critical (Must Fix)

None identified.

---

## Suggestions (Should Consider)

### 1. Enforce the project's own clippy lint policy in `Cargo.toml`

**Where:** `Cargo.toml`
**Why:** The global standard (`CLAUDE.md`) requires `unwrap_used = "deny"`, `expect_used = "warn"`, `panic = "deny"`, etc. Currently there is no `[lints.clippy]` section, so these rules are not enforced in CI. Adding them will immediately surface the 88 unwrap/expect warnings that are currently only visible with manual flags.
**Fix:** Add a `[lints.clippy]` table. This will require cleaning up test-only `unwrap()` calls (e.g., convert to `?` in tests or use `assert!(result.is_ok())` patterns) and adding targeted `allow` attributes where truly justified.

### 2. Avoid cloning the whole configuration for every scheduled job

**Where:** `src/scheduler.rs:223`, `src/backup.rs:99`
**Why:** `ResolvedConfig` is cloned for every backup task dispatch. For typical configs this is fine, but it scales with secret count and job count.
**Fix:** Pass `Arc<ResolvedConfig>` into `Backup::run` instead of cloning. This requires changing `Backup::run` and `Scheduler::run_backup_job` signatures but is a straightforward refactor.

### 3. `resolve_exclude_file` silently ignores a missing explicit exclude file

**Where:** `src/exclude.rs:76-93`
**Why:** When a user explicitly sets `exclude_file: /path/to/my/excludes.txt` and that file does not exist, the code silently falls back to the default excludes file. This is surprising behavior.
**Fix:** Return an error when `exclude_file` is configured but the path does not exist. The fallback chain should only apply when no explicit file is configured.

### 4. Improve log accuracy and safety for restic command args

**Where:** `src/backup.rs:200`
**Why:** `debug!("Executing: restic {}", args.join(" "))` does not quote arguments, so paths with spaces are misleading. It is safe today because the password is passed via `env`, but future refactors could introduce sensitive args.
**Fix:** Log args as structured fields or shell-quote each argument. Avoid a single joined string that could leak or misrepresent the command.

### 5. Use typed deserialization for restic snapshot JSON

**Where:** `src/snapshot.rs:58-93`
**Why:** The parser uses direct indexing with `unwrap_or("")` defaults. If restic changes its JSON shape, the code silently produces empty strings, making debugging harder.
**Fix:** Define a `#[derive(Deserialize)]` struct matching restic's snapshot JSON and deserialize into it.

### 6. Make scheduler clock-truncation failures visible

**Where:** `src/scheduler.rs:198-204`
**Why:** If `Local::now().with_second(0).with_nanosecond(0)` ever returns `None`, the tick is silently skipped. Unusual clock states should be logged.
**Fix:** Log `warn!` in the `else` branch when truncation fails.

### 7. Avoid `reqwest::Client` fallback to default client on build failure

**Where:** `src/notifications.rs:66-69`
**Why:** `Client::builder().timeout(...).build().unwrap_or_else(|_| Client::new())` silently drops the custom 10-second timeout if the builder fails.
**Fix:** Propagate the error or use `expect` with a clear message. The builder failure indicates a serious misconfiguration.

### 8. Harden mutex usage in notification rate limiter

**Where:** `src/notifications.rs:167,178`
**Why:** `lock().unwrap()` can panic if the mutex is poisoned. A panic in a notification task would poison the mutex and crash future notifications.
**Fix:** Use `lock().unwrap_or_else(|poisoned| poisoned.into_inner())` or switch to `parking_lot::Mutex`.

### 9. Bound or document scheduler concurrency

**Where:** `src/scheduler.rs:295`
**Why:** Backups run in `spawn_blocking`. Multiple different jobs can run concurrently, potentially exhausting the blocking thread pool or system resources.
**Fix:** Consider a `tokio::sync::Semaphore` to bound concurrent backups, and document the maximum concurrency in the README.

### 10. Improve TUI child termination

**Where:** `src/tui.rs:164-170`
**Why:** The TUI sends `SIGTERM` to children by spawning `kill -TERM <pid>`. This relies on the external `kill` binary, ignores failures, and could race with PID reuse.
**Fix:** Use `nix::signal::kill()` (or equivalent) directly instead of spawning a shell command.

### 11. Manage TUI background threads more explicitly

**Where:** `src/tui.rs:217-221,310,361,697,731,743`
**Why:** Multiple `std::thread::spawn` calls are unbounded and un-named.
**Fix:** Name threads with `.name(...)` and consider a bounded pool or `tokio::task::spawn_blocking` for background work.

---

## Nits (Optional)

- **`src/cli.rs:240-327`**: The `Show` command repeats `serde_json::to_string_pretty(...).unwrap_or_else(|e| format!(...))` four times. Extract a small helper.
- **`src/cli.rs:240-327`**: Human-readable formatting for jobs/repos is duplicated between `Show` and the list commands. Consider a shared rendering helper.
- **`src/backup.rs:42-54`**: `format_bytes` uses floating-point arithmetic; consider a bit-shift loop or the `humansize` crate for more predictable output.
- **`src/config.rs:17-27`**: `join_errors` allocates an intermediate `Vec<String>` purely to join. It can be expressed as a direct iterator chain, though the current form is readable.
- **`src/scheduler.rs:355-368`**: `list_scheduled_jobs` returns `Vec<(&str, String)>`. A small named struct would be clearer.
- **`README.md`**: The architecture diagram shows "Backup → Telegram" directly, but notifications are actually routed through `NotificationManager` inside the scheduler.

---

## What's Good

- **Clean error taxonomy:** `thiserror`-based enums with `#[from]` conversions keep call sites readable.
- **Signal handling:** The scheduler correctly installs SIGTERM and Ctrl-C handlers, drains in-flight backups, and force-exits on a second signal. Integration tests verify this.
- **Password handling:** Repository passwords are passed via `RESTIC_PASSWORD` environment variable, never logged or shell-interpolated.
- **Partial backup handling:** Restic exit code 3 is treated as partial success and surfaced in notifications.
- **Secrets permission warnings:** Unix permission checks on `secrets.yaml` and its parent directory are present and tested.
- **Config validation:** Errors are aggregated before failing, ordering is deterministic via `BTreeMap`, and job-to-repository references are validated.
- **Test coverage:** 175 tests including end-to-end daemon signal tests and edge cases such as non-UTF8 paths, notification rate-limiting, and dry-run hook skipping.
- **TUI child cleanup:** Child PIDs are tracked and sent `SIGTERM` on TUI exit.

---

## Bottom Line

The codebase is solid, well-tested, and CI-clean. The highest-impact improvements are:

1. Enforce `[lints.clippy]` in `Cargo.toml` and clean up the resulting unwrap/expect warnings.
2. Switch `Backup::run` and the scheduler to use `Arc<ResolvedConfig>` instead of cloning.
3. Make `resolve_exclude_file` error when an explicit exclude file is missing.

None of these are blockers, but addressing them will bring the project fully in line with its own stated standards.
