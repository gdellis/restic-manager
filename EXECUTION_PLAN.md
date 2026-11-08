# Execution Plan - restic-manager remediation

Ordered sequence for clearing the open issues and pull requests. Depends on PR #117
docs reconcile having landed (done). Merging must wait for explicit maintainer approval.

## Phase 0 - Trust gate and docs

- (1) PR #102 - chore/ci-markdownlint-glob: make the Markdown lint job actually lint files.
   Blocks trust in every later doc change.
- (2) PR #109 - review/audit-2026-10: land the review report and its implementation plan as
   reference material. Docs only, low risk.
- (3) PR #113 - chore/todo-markdown-line-length: cosmetic TODO reflow.

## Phase 1 - Correctness and secrets (Waves 1-2)

- (4) PR #103 - fix/backup-summary-parse: correct restic summary counters (C1).
- (5) PR #104 - fix/retention-after-backup: apply retention after a backup (C3).
- (6) PR #105 - fix/cron-seconds-field: reject unsatisfiable 6-field cron (H11).
- (7) PR #107 - fix/scheduler-channel-deadlock: prevent wedge at 101+ due jobs (H10).
- (8) PR #106 - fix/secret-handling-hardening: token scrubbing plus credential redaction
   (H9, M21, M13, M14).
   Verify each with a real end-to-end run per .agents/workflow.md.

## Phase 2 - Process lifecycle (Wave 3)

- (9) PR #112 - fix/orphan-restic-on-sigterm: kill the restic child on SIGTERM (H7).
   Add the integration test asserting no surviving restic process.

## Phase 3 - TUI stream (Wave 5)

- (10) Widen CI triggers so PRs based on feat/tui actually run checks (PR #111 currently
    targets feat/tui). Fold this in before steps 11-14.
- (11) PR #92 - feat/tui: merge the interactive dashboard base.
- (12) PR #108 - fix/password-key-mask-panic: char-safe masking (H1, based on feat/tui).
- (13) PR #114 - chore/ratatui-0.30-upgrade: dependency bump before more TUI work.
- (14) PR #115 - refactor/tui-in-process-data: library calls instead of subprocesses. This
    dissolves H2 (broken config hiding all jobs) and H3 (child stderr corruption).
- (15) New PRs still needed for remaining TUI findings: H4 daemon detection, H5 keybinding
    docs, H6 stale async responses, M4 typed snapshot parsing, M11 resize handling,
    M12 data refresh. One logical PR each.

## Phase 4 - Scheduler and notification hygiene (Wave 4)

- (16) New PR: lifetime-scoped NotificationManager, per-job rate-limit keying, and JoinSet
    reaping (H12, M17).
- (17) New PR: poison-tolerant mutexes in scheduler and notifications, plus Arc<ResolvedConfig>
    and a concurrency semaphore (M3, M5, M6, M18).

## Phase 5 - Remaining hardening (Wave 6)

- (18) New PR: exclude-directory traversal guard and missing-file handling (H13, M1).
- (19) New PR: errors carrying the file path, reqwest timeout preservation, 0700 config dir
    (M2, M8, M9).
- (20) New PR: reject all-null retention policies and fix RetentionPolicy::default (M19, M20).
- (21) New PR: low-severity bundle (unnamed threads, float format_bytes, join_errors
    allocation, duplicated Show formatting).

## Phase 6 - Issue-driven work

- (22) Issue #74: design and implement the secret-backend abstraction (file backend keeps strict
    validation; env/keyring backends bypass the missing-key check). Depends on Phase 1 item 8.
- (23) Issue #45: scope a read-only GUI status view backed by daemon state. Icebox until the
    scheduler exposes job state over an IPC or HTTP surface.

## Guardrails

- One logical change per branch; never commit to main.
- Run the four CI commands on every PR.
- Markdown changes must pass markdownlint-cli2 (120-char limit).
- Do not merge without explicit maintainer approval.
