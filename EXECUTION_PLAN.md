# Execution Plan - restic-manager remediation

Ordered sequence for clearing the October 2026 review findings. Phases 0-3 landed on main
on 2026-11-15; the remaining work is tracked as issues. When an issue closes, remove its
line here.

## Done on main

- **Phase 0 - trust gate and docs**: PRs #102, #109, #118 (PR #113 closed as superseded by
  #117's TODO rewrite).
- **Phase 1 - correctness and secrets**: PRs #103, #104, #105, #107, #106, plus the
  burst-test flake fix #119. Covers C1, C3, H9, H10, H11, M13, M14, M21.
- **Phase 2 - process lifecycle**: PR #112 - the restic child is terminated on shutdown
  (H7), one-shot and daemon force-exit paths alike.
- **Phase 3 - TUI stream**: PRs #92, #108, #111, #114, #115, #120, #121, #122. Covers H1,
  H4, H5, H6, and dissolves H2/H3 (in-process data loading).

## Phase 3 leftovers - TUI findings

- #123 (M4) - typed snapshot JSON parsing in SnapshotManager.
- #126 (M11, M12) - TUI resize handling and periodic data refresh.

## Phase 4 - scheduler and notification hygiene

- #124 (H12, M17, M3, M18) - lifetime-scoped NotificationManager with per-job rate
  limiting, JoinSet reaping, poison-tolerant mutexes.
- #125 (M5, M6) - Arc<ResolvedConfig> per dispatch and a documented concurrency semaphore.

## Phase 5 - hardening

- #127 (H13, M1) - exclude-directory traversal guard and missing exclude_file handling.
- #128 (M2, M8, M9) - file paths in errors, reqwest timeout preservation, 0700 config dir.
- #129 (M20) - RetentionPolicy serde defaults vs the dead Default impl (M19 done in #104).
- #130 - low-severity bundle (thread names, pure kill, doc drift, micro-cleanups).

## Phase 6 - issue-driven work

- #74 - secret-backend abstraction (file backend keeps strict validation; env/keyring
  backends bypass the missing-key check). The secret-handling hardening it depended on
  (#106) is merged.
- #45 - read-only GUI status view. Icebox until the scheduler exposes job state over an
  IPC or HTTP surface.

## Guardrails

- One logical change per branch; never commit to main.
- Run the four CI commands on every PR (build, test, clippy -D warnings, fmt).
- Markdown changes must pass markdownlint-cli2 (120-char limit).
- Do not merge without explicit maintainer approval.

## Reference material

The October 2026 review, the July 2026 review, and the original implementation plan live
in `docs/reviews/` (moved from the repo root during the docs consolidation).
