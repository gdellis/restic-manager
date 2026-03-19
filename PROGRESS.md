# Implementation Progress

## Phase 1: Foundation
- [ ] Project setup with Cargo.toml dependencies
- [ ] Error types (src/errors.rs)
- [ ] Config loading (src/config.rs)
- [ ] Secrets loading (src/secrets.rs)
- [ ] Basic CLI structure with clap

## Phase 2: Repository Operations
- [ ] Repository init command
- [ ] Repository check command
- [ ] Repository unlock command

## Phase 3: Backup/Restore
- [ ] Backup execution
- [ ] Restore latest snapshot
- [ ] Restore specific snapshot
- [ ] Pre-backup hooks
- [ ] Post-backup hooks

## Phase 4: Snapshot Management
- [ ] List snapshots
- [ ] Retention policies (forget)
- [ ] Prune operations

## Phase 5: Scheduler
- [ ] Cron parsing and validation
- [ ] Daemon mode
- [ ] Job queue management
- [ ] Graceful shutdown

## Phase 6: Notifications
- [ ] Telegram bot integration
- [ ] Failure notifications
- [ ] Success notifications
- [ ] Rate limiting

---

## Notes

- Start date: 
- Target: Linux only initially
- Config location: ~/.config/restic-manager/