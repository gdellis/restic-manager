# Implementation Progress

## Completed Phases

### Phase 1: Foundation

- [x] Project setup with Cargo.toml dependencies
- [x] Error types (src/errors.rs)
- [x] Config loading (src/config.rs)
- [x] Secrets loading (src/secrets.rs)
- [x] Basic CLI structure with clap

### Phase 2: Repository Operations

- [x] Repository init command
- [x] Repository check command
- [x] Repository unlock command

### Phase 3: Backup/Restore

- [x] Backup execution
- [x] Restore latest snapshot
- [x] Restore specific snapshot
- [x] Pre-backup hooks
- [x] Post-backup hooks

### Phase 4: Snapshot Management

- [x] List snapshots
- [x] Retention policies (forget)
- [x] Prune operations

### Phase 5: Scheduler

- [x] Cron parsing and validation
- [x] Daemon mode
- [x] Job queue management
- [x] Graceful shutdown

### Phase 6: Notifications

- [x] Telegram bot integration
- [x] Failure notifications
- [x] Success notifications
- [x] Rate limiting

---

## Review Remediation Train (2026-11)

Remediation of the 2026-10 code review; the ordered plan is [FIX_PLAN.md](FIX_PLAN.md).

- [x] PR-0.1 - test safety net hardening (contract stubs, log-ordering, exclusive scratch dirs)
- [x] PR-1.1 - shared `run_restic()` runner with typed exit codes (`src/restic.rs`)
- [x] FIX_PLAN.md reconciled against the parallel GitHub PR train (coverage map + merge gate)
- [ ] PR-1.2 through PR-7.3 remain - see the FIX_PLAN remaining-train list

## Current Focus: Open Issues Triage

*Last updated: 2026-11-08*
*Tracking: [GitHub Issues](https://github.com/gdellis/restic-manager/issues)*

For acceptance criteria, see [AGENTS.md](AGENTS.md#Commands).

### High Priority — Quick Wins

(Both previous quick wins are merged: #71 via PR #76, #73 via PR #78/#84/#86/#88.)

| Issue | Title | Status | Effort | PR |
|-------|-------|--------|--------|-----|
| #74 | Support non-file secret backends without triggering false missing-key errors | Open | 2-3 hours | in flight |

**Goal**: Keep startup validation compatible with future env/keyring backends.

### Medium Priority — Architecture & Polish

| Issue | Title | Status | Effort | PR |
|-------|-------|--------|--------|-----|
| #55 | Documentation: Add doc comments to public structs and methods | Done | 1 hour | #61 |
| #46 | Pin the Rust toolchain so local clippy matches CI | Done | 20 min | #60 |
| #56 | Feature: Extend log_cli_output to check, prune, and forget commands | Done | 1 hour | #62 |
| - | October 2026 review remediation (C1, C3, H1, H7, H9-H11, ...) | In flight | varies | #102-#116 |

**Goal**: Land the review remediation PRs and keep the codebase maintainable.

### Icebox / Needs Scoping

| Issue | Title | Status | Effort | PR |
|-------|-------|--------|--------|-----|
| #45 | Feature: GUI component for managing backups | Icebox | 1-2 weeks | — |
| #50 | Shutdown integration tests: contract-shaped assertions | Done | 2-3 hours | #63 |

**Goal**: Plan larger features once core CLI behavior is solid.

---

### Suggested Execution Order

1. **#102** — CI markdown-lint glob fix (trust gate).
2. **#103 / #104 / #105 / #106 / #107** — Wave 1+2 correctness and secrets.
3. **#112** — Wave 3 process lifecycle (orphan restic).
4. **#92 + #108 + #114 + #115** — Wave 5 TUI fixes (needs workflow trigger widening).
5. **#74** — Secret-backend abstraction (depends on #106 landing).
6. **#45** — GUI design discussion once scheduler state is exposed.

---

## Notes

- Config location: `~/.config/restic-manager/`
- All issues tracked on GitHub: [gdellis/restic-manager/issues](https://github.com/gdellis/restic-manager/issues)
- See [AGENTS.md](AGENTS.md) for development commands and CI checks
