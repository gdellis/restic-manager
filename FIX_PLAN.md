# Fix Plan — Code Review Remediation

Ordered plan for fixing the findings from the full code review (three parallel
reviewers over all of `src/` + `tests/`, with cross-site verification and a
full clippy/test/fmt run). Severity labels `[H#] [M#] [L#] [N#]` refer to the
review report (High / Medium / Low / Nit index). Existing TODO.md items are
cited where the finding is already tracked.

## Ground rules

- Each PR is independently reviewable and keeps `cargo fmt --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`, and
  `cargo test --all-targets` green; run `cargo tarpaulin` before pushing
  (AGENTS.md).
- Every behavior change lands with a regression test named
  `Regression test for #NN` (repo convention). Open one GitHub issue per fix
  and name the test after its number.
- Update README.md / DESIGN.md / TODO.md / PROGRESS.md in the same PR as the
  behavior change (AGENTS.md).
- No CLI output or exit-code change without a locking integration test.
- Effort legend: S <= 1h, M ~ half day, L ~ 1-2 days.

## Decisions to confirm (recommendations included)

| # | Decision | Recommendation |
|---|----------|----------------|
| D1 | How to scope legacy (untagged) snapshots once job tagging lands | Tag `restic-manager-job:<name>` going forward; filter `--tag` first, fall back to `--path <job paths>` so legacy snapshots stay visible |
| D2 | Auto-retention: where it runs, and whether prune is toggleable | Run `apply_retention` after successful backup in `Backup::run` (skip dry-run); add `retention.prune: true` default since DESIGN calls prune optional |
| D3 | Exit code for partial backups | `3`, mirroring restic |
| D4 | `unlock --remove-all` (removes live locks) | Plain `restic unlock` by default; `--force` opts into `--remove-all` |
| D5 | 6-field cron with non-zero seconds | Occurrence walking (`schedule.after()`) fixes the never-fires bug; also reject seconds != 0 at validation for honest minute-granularity semantics |
| D6 | Restore `--target` guardrails | Refuse `/` and CWD always; non-empty existing dir requires new `--force` flag |
| D7 | Config key for concurrency cap | `daemon: { max_concurrent_jobs: 2 }` (top-level block; additive, `deny_unknown_fields` safe) |

## Milestone 0 — Test safety net (do first)

**PR-0.1 — Test hardening [M15, L14] — M**

- Make `test_config_validate_job_refs_nonexistent_repo` call `Config::validate()`
  (env-scratch `XDG_CONFIG_HOME` + `Config::load`, like the existing
  env-mutating tests) and assert the error names the missing repository.
- Reword `daemon_drain_waits_for_in_flight_backup` to assert log ordering
  ("Draining in-flight jobs" -> job end -> "Scheduler stopped") instead of
  betting on the ~1s alive-window.
- Replace pid-only `/tmp/restic-manager-*` dirs with exclusive create
  (`tempfile::TempDir` or `create_dir` that fails if exists) in
  daemon_shutdown.rs, integration_test.rs, cli_log tests; clean up scratch dirs.
- Add contract tests locking CURRENT behavior that later PRs will change:
  exit 0 on partial (flips in PR-4.1), `restore_snapshot_id` argv shape
  (extract pure `build_restore_args` first — no behavior change),
  `init-exclude` message text.

Why first: M2-M4 change exit codes, restore argv, and prune semantics; these
tests turn each change into a deliberate, visible diff.

## Milestone 1 — Shared restic runner + error classification

**PR-1.1 — `run_restic()` extraction + exit-code mapping [L10, M12, L9, L11] — M**

- New `src/restic.rs`: `run_restic(args, password, log_path) -> Result<Output>`;
  migrate all 9 call sites (backup, restore, snapshot x3, repository x3,
  find_latest_snapshot).
- Map restic exit codes: 10 -> `NotInitialized`, 11 -> `Locked`,
  12 -> `InvalidPassword` (the currently-dead variants); `forget` exit 3
  (partial removal) must parse output and still run prune.
- When stderr is empty, the error carries exit status + stdout tail.
- Tests: unit tests for the mapper; regression test for forget-partial path.

**PR-1.2 — Watchdog + bounded capture [M14, L2, L17] — M/L**

- Deadline per restic invocation (restic `--timeout` and/or wait-with-kill),
  clear error on expiry; a wedged backend no longer hangs the daemon forever
  while holding a repo lock.
- Stop retaining every stdout line unless `log_cli_output` needs them; handle
  line-read `Err` explicitly (kills the misleading "No summary in backup
  output").

## Milestone 2 — Data-loss guards (highest stakes)

**PR-2.1 — Retention validation + prune semantics [H3, M12, TODO #?-new] — S/M**

- `Job::validate`: reject any `keep_*: 0` (0 = keep none -> wipes the group)
  and policies with no effective keep flag; error text spells out the risk.
- `Commands::Prune`: run `restic prune` unconditionally (forget only when a
  policy exists); `retention.prune` toggle per D2.
- Tests: `keep_last: 0` rejected; all-zeros rejected; prune works on a job
  without retention.

**PR-2.2 — Job scoping for restore / list / forget [H4] — L**

- `Backup::run` adds `--tag restic-manager-job:<name>` to every snapshot.
- `find_latest_snapshot`, `SnapshotManager::list`, `forget` filter `--tag`
  with `--path` fallback for legacy untagged snapshots (D1); `restore_latest`
  selects max `time`, not `.first()`.
- Tests: arg-builder unit tests for all three commands; parser picks newest;
  document a real-restic e2e smoke (init -> backup x2 jobs -> list -> restore
  -> forget) since the sandbox has no restic binary.

## Milestone 3 — Backup run flow

**PR-3.1 — Hooks lifecycle [H2, M1, L13, TODO #76] — M**

- Capture `execute_backup`'s result; run post-backup hooks whenever pre-hooks
  succeeded (failure path included); return the backup error first if any.
- A post-hook-only failure keeps the `BackupResult` (snapshot id!) and sends a
  degraded notification naming the snapshot — never `[FAILED]` for a good
  backup.
- Set `result.job_name` [N6]; hook errors include the command and a human exit
  status ("exit 1" / "killed by signal", not `exit Some(1)`).
- Tests: post-hooks-run-on-backup-failure (stub restic script); notification
  message content.

**PR-3.2 — Auto-retention after backup [H1, D2] — M**

- After a successful backup in `Backup::run` (never in dry-run): run
  `SnapshotManager::apply_retention` when `job.retention` is set; retention
  failure warns + degraded-notifies but does not fail the backup.
- Tests: dry-run never forgets; retention failure does not flip backup result.

## Milestone 4 — CLI contract & input safety

**PR-4.1 — Exit codes [H5, D3] — S**

- Partial `run` exits 3 after the summary; integration tests lock `run` and
  `check` exit-code contracts; README documents the codes.

**PR-4.2 — Argv hardening [M2, L1] — M**

- Insert `--` before positionals in backup and restore; validate `snapshot_id`
  against `^(latest|[0-9a-fA-F]{6,64})$`; reject paths starting with `-` at
  config validation; build all args as `OsString` end-to-end (fixes non-UTF8
  path mangling [L1]).
- Tests: `build_restore_args` / backup-args unit tests incl. injection cases.

**PR-4.3 — Restore `--target` guardrails [M3, TODO #75, D6] — M**

- Canonicalize target; refuse `/` and the CWD; existing non-empty dir requires
  `--force`; fix the help text that currently promises protection it does not
  provide.
- Tests: each refusal plus the `--force` path on a pure validation function.

## Milestone 5 — Scheduler robustness

**PR-5.1 — Deadlock + concurrency cap [M6, TODO #66, D7] — L**

- The tick handler must never await the channel consumer: `try_send` /
  unbounded internal channel / dedicated dispatcher task (101+ simultaneous
  triggers must not wedge the daemon and block SIGTERM).
- `tokio::sync::Semaphore` capped by `daemon.max_concurrent_jobs`; optionally
  serialize per repository to avoid restic lock contention.
- Test: 101+ triggers in one tick, then SIGTERM still shuts down gracefully.

**PR-5.2 — Catch-up + cron correctness [M7, M8, N8, TODO #68] — L**

- Replace `includes(now)`-on-truncated-time sampling with occurrence walking
  (`schedule.after(&last_tick)` up to now): runs catch up after suspend/stall,
  and skipped/missed windows notify per the job's notification config instead
  of vanishing with a `warn!`.
- Reject non-zero seconds in 6-field expressions at validation (D5); one shared
  `config::parse_schedule()` used by validation and the scheduler.
- Tests: clock-advance suspend simulation; seconds-field rejection.

**PR-5.3 — Daemon hygiene [L4, L5] — S**

- Reap finished tasks (`try_join_next`) in tick/recv arms; poison-proof every
  `lock().unwrap()` (`unwrap_or_else(|p| p.into_inner())`) in
  `InFlightGuard::drop`, dispatch, and the notifications limiter.

## Milestone 6 — Secrets, notifications & log hygiene

**PR-6.1 — Credential redaction [H6, M10] — M**

- `reqwest` failures must not echo the bot-token URL: `e.without_url()` or a
  generic `SendFailed("network error")`.
- `redact_url()` helper strips `user:****@` userinfo from repo strings before
  any log line, error, or notification (backup.rs `info!`/`debug!`,
  `CommandFailed` payloads -> Telegram).
- Tests: redact_url table; regression test asserting error strings never
  contain the configured password/userinfo substring.

**PR-6.2 — Notification correctness [H7, N2, L6, N1] — M**

- One rate limiter per job created at `Scheduler::new` (per-run limiter is
  non-functional); make check-and-record atomic under one lock.
- Keep the 10s client timeout on the builder-failure fallback; truncate by
  chars, not bytes, at Telegram's 4096 limit.
- Tests: limiter persists across two job runs; multibyte truncation.

**PR-6.3 — Secrets & log file [M11, L7, L8, N3-N5] — M**

- Coerce YAML scalars to strings (numeric `chat_id` must load); redacted
  `Debug` for `Secrets`/`TelegramConfig`; drop `Serialize` if unused; factor
  `from_path` for `load`/`load_optional`; align README wording with the 0600
  check; doc note that a secret named `telegram` is reserved.
- `cli_log`: create files 0600 (`OpenOptionsExt::mode`); `list` and `init`
  also write the configured CLI output log.
- Tests: numeric chat_id loads; 0600 asserted after create.

## Milestone 7 — Exclude, bootstrap & cleanup train

**PR-7.1 — Exclude hardening [M4, M5] — M**

- `resolve_exclude_file` returns `Result<Option<PathBuf>>`: a configured but
  missing `job.exclude_file` fails the run (or warns and skips the job) instead
  of silently backing up excluded data; stop swallowing `write_job_excludes`
  and `exclude_file_path` errors.
- Reject job names containing `/`, `\`, or `..` in `Job::validate`
  (kills the `excludes.d` path traversal).
- Tests: missing exclude_file errors; traversal job name rejected.

**PR-7.2 — Bootstrap & validation scope [M9, L3, L15, TODO #74] — M**

- Dispatch `InitExclude` (and `--help`) before `ResolvedConfig::load`; it must
  work on a fresh machine with no config.yaml or secrets.yaml.
- `init-exclude` tells the truth ("created" vs "already exists") and gains
  `--reset`/`--force` to match its documented reset semantics.
- Move the missing-secret check out of the load-time global sweep into
  `resolve_repo` (per-job resolution) — one bad repo key stops blocking every
  command.
- Tests: init-exclude with empty env-scratch config dir; message accuracy.

**PR-7.3 — Cleanup + docs reconciliation [L11, L12, L16, N7, N9, N10] — M**

- Remove (or wire up) dead `ResticError`/`ConfigError`/`SecretsError` variants
  and the unused `Result` alias.
- Progress output: gate on `IsTerminal`, route file paths through `debug!`
  (no raw ANSI/paths in journald at default level).
- `format_progress` shows bytes in both branches; `List` prints paths via
  `display()`, not `Debug`; config-dir failures get their own error wording.
- DESIGN.md: annotate "restore to original path / stdin" as not yet
  implemented; verify and close/clarify TODO #77 ("Restore runs post_backup
  hooks" — no such call site found; likely stale).

## Suggested issue mapping

- Existing TODO items covered here: #66 (PR-5.1), #67 (PR-4.2), #68 (PR-5.2),
  #74 (PR-7.2), #75 (PR-4.3), #76 (PR-3.1); verify #77 in PR-7.3.
- Open new issues for the rest (H1, H3-H8, M2, M10-M15, etc.) and name the
  regression tests after the new numbers.

## Minimum viable safety set

If time is constrained, ship these four PRs first — they cover every
data-loss and secret-leak exposure:

1. PR-2.1 + PR-2.2 (retention validation, job scoping)
2. PR-3.1 (post-hooks on the failure path)
3. PR-4.1 (partial exit code)
4. PR-6.1 (credential redaction)

## Final verification (before declaring done)

- Full AGENTS.md matrix green: fmt, clippy `-D warnings`, tests, tarpaulin.
- Manual e2e with a real restic binary: init -> backup (2 jobs, 1 repo) ->
  list -> restore --target -> forget/prune -> daemon SIGTERM drain.
- README/DESIGN examples re-validated by the integration tests
  (`test_readme_quick_start_*` style).
- PROGRESS.md entry summarizing the remediation.
