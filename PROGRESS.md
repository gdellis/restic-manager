# Implementation Progress

## Phase 1: Foundation

- [x] Project setup with Cargo.toml dependencies
- [x] Error types (src/errors.rs)
- [x] Config loading (src/config.rs)
- [x] Secrets loading (src/secrets.rs)
- [x] Basic CLI structure with clap

## Phase 2: Repository Operations

- [x] Repository init command
- [x] Repository check command
- [x] Repository unlock command

## Phase 3: Backup/Restore

- [x] Backup execution
- [x] Restore latest snapshot
- [x] Restore specific snapshot
- [x] Pre-backup hooks
- [x] Post-backup hooks

## Phase 4: Snapshot Management

- [x] List snapshots
- [x] Retention policies (forget)
- [x] Prune operations

## Phase 5: Scheduler

- [x] Cron parsing and validation
- [x] Daemon mode
- [x] Job queue management
- [x] Graceful shutdown

## Phase 6: Notifications

- [x] Telegram bot integration
- [x] Failure notifications
- [x] Success notifications
- [x] Rate limiting

---

## Current Focus: Open Issues Triage

*Last updated: 2026-07-17*
*Tracking: [GitHub Issues](https://github.com/gdellis/restic-manager/issues)*

### Phase 1: Critical Fixes (High Priority)

| Issue | Title | Status | Effort | PR |
|-------|-------|--------|--------|-----|
| #54 | Backup: Path handling could panic on non-UTF8 paths | 🟡 Backlog | 15 min | |
| #49 | Logging: warn on invalid RUST_LOG instead of silently falling back | 🟡 Backlog | 30 min | |
| #53 | Notifications: Unicode emoji may cause issues in non-UTF8 terminals | 🟡 Backlog | 30 min | |

**Goal**: Eliminate potential panics and improve debugging experience.
**Estimated completion**: 1-2 days

### Phase 2: Code Quality (Medium Priority)

| Issue | Title | Status | Effort | PR |
|-------|-------|--------|--------|-----|
| #55 | Documentation: Add doc comments to public structs and methods | 🟡 Backlog | 1 hour | |
| #46 | Pin the Rust toolchain so local clippy matches CI | 🟡 Backlog | 20 min | |
| #56 | Feature: Extend log_cli_output to check, prune, and forget commands | 🟡 Backlog | 1 hour | |

**Goal**: Improve code maintainability and developer experience.
**Estimated completion**: 1 day

### Phase 3: Existing Work (Ongoing)

| Issue | Title | Status | Effort | PR |
|-------|-------|--------|--------|-----|
| #50 | Shutdown integration tests: contract-shaped assertions and better diagnostics | 🟡 Backlog | 2-3 hours | |
| #45 | Feature: GUI component for managing backups | 🟢 Icebox | 1-2 weeks | |

**Goal**: Complete deferred polish and plan future features.
**Estimated completion**: Ongoing

---

### Execution Plan

#### Sprint 1 (Week 1): Stability & Polish
- Day 1: Fix #54, #49, #53 (critical fixes)
- Day 2: Complete #55, #46, #56 (code quality)

#### Sprint 2 (Week 2): Testing & Planning
- Day 3-4: Complete #50 (shutdown test improvements)
- Day 5: Review #45 (GUI design)

### Acceptance Criteria

Each issue must meet the following before closure:
- [ ] Code changes pass `cargo build`
- [ ] Code changes pass `cargo test --all-targets`
- [ ] Code changes pass `cargo clippy --all-targets -- -D warnings`
- [ ] Code changes pass `cargo fmt --check`
- [ ] Updated tests (if applicable)
- [ ] Updated documentation (if applicable)
- [ ] Commit message follows project conventions
- [ ] PR description references issue number

---

## Notes

- Start date:
- Target: Linux only initially
- Config location: ~/.config/restic-manager/
- All issues tracked on GitHub: https://github.com/gdellis/restic-manager/issues
