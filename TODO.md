# TODO

## Active backlog

1. **#74** — Design a secret-backend abstraction so non-file secret sources (env var, keyring,
   etc.) do not trigger false "missing password key" errors. *(Open; see the October 2026 review
   remediation PRs #102-#116, most of which are still awaiting merge.)*

## Short-term code quality

- *(done)* **#55** — doc comments (merged: PR #61).
- *(done)* **#46** — toolchain pin to 1.97.0 (merged: PR #60).
- *(done)* **#56** — `log_cli_output` for check/prune/unlock/forget (merged: PR #62).

## Longer term / needs scoping

- **#45** — GUI component for managing backups (evaluate whether to use
  [blinc](https://github.com/project-blinc/blinc) or another approach).
  *(Icebox; depends on scheduler state exposure.)*
- **#50** — Shutdown integration tests (done: PR #63; contract-shaped assertions shipped).
- *(in flight)* October 2026 review remediation: 15 open PRs (#92, #102-#116) covering the
  Critical/High findings C1, C3, H1, H7, H9-H11, M10, M13, M14, M19, M21 plus ratatui 0.30
  and TUI in-process data loading. Awaiting maintainer approval before merge.
