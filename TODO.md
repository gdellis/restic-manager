# TODO

## Active backlog

1. **#74** — Design a secret-backend abstraction so non-file secret sources (env var, keyring,
   etc.) do not trigger false "missing password key" errors. *(Open; the remediation it depends
   on landed 2026-11-15.)*

## Short-term code quality

- *(done)* **#55** — doc comments (merged: PR #61).
- *(done)* **#46** — toolchain pin to 1.97.0 (merged: PR #60).
- *(done)* **#56** — `log_cli_output` for check/prune/unlock/forget (merged: PR #62).

## Longer term / needs scoping

- **#45** — GUI component for managing backups (evaluate whether to use
  [blinc](https://github.com/project-blinc/blinc) or another approach).
  *(Icebox; depends on scheduler state exposure.)*
- **#50** — Shutdown integration tests (done: PR #63; contract-shaped assertions shipped).
- *(done on main)* October 2026 review remediation, phases 0-3: 18 PRs merged covering C1, C3,
  H1, H4-H7, H9-H11, M13, M14, M21, ratatui 0.30, and TUI in-process data loading (dissolving
  H2/H3). Follow-up work is tracked as issues #123-#130; sequencing lives in
  [EXECUTION_PLAN.md](EXECUTION_PLAN.md).
