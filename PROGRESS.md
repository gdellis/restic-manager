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

## Current Focus: Open Issues Triage

*Last updated: 2026-07-18*
*Tracking: [GitHub Issues](https://github.com/gdellis/restic-manager/issues)*

For acceptance criteria, see [AGENTS.md](AGENTS.md#Commands).

### High Priority — Quick Wins

| Issue | Title | Status | Effort | PR |
|-------|-------|--------|--------|-----|
| #71 | Opencode reviewer should only run when explicitly requested in PR comments | Open | 1 hour | — |
| #73 | Collect and report all config validation errors at once | Open | 1-2 hours | — |

**Goal**: Reduce CI spend and improve first-run config debugging.

### Medium Priority — Architecture & Polish

| Issue | Title | Status | Effort | PR |
|-------|-------|--------|--------|-----|
| #74 | Support non-file secret backends without triggering false missing-key errors | Open | 2-3 hours | — |
| #55 | Documentation: Add doc comments to public structs and methods | Backlog | 1 hour | — |
| #46 | Pin the Rust toolchain so local clippy matches CI | Backlog | 20 min | — |
| #56 | Feature: Extend log_cli_output to check, prune, and forget commands | Backlog | 1 hour | — |

**Goal**: Keep the codebase maintainable and prepare for future secret backends.

### Icebox / Needs Scoping

| Issue | Title | Status | Effort | PR |
|-------|-------|--------|--------|-----|
| #45 | Feature: GUI component for managing backups | Icebox | 1-2 weeks | — |
| #50 | Shutdown integration tests: contract-shaped assertions and better diagnostics | Backlog | 2-3 hours | — |

**Goal**: Plan larger features once core CLI behavior is solid.

---

### Suggested Execution Order

1. **#71** — Gate OpenCode reviewer behind a PR comment command.
2. **#73** — Collect all config validation errors in one pass.
3. **#74** — Design secret-backend abstraction so non-file sources don't false-fail.
4. **#55 / #46 / #56** — Pick up remaining code-quality items.
5. **#45** — GUI design discussion and milestone planning.

---

## Notes

- Config location: `~/.config/restic-manager/`
- All issues tracked on GitHub: [gdellis/restic-manager/issues](https://github.com/gdellis/restic-manager/issues)
- See [AGENTS.md](AGENTS.md) for development commands and CI checks
