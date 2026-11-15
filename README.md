# Restic Manager

<picture>
 <img alt="Logo" src="resources/logo.jpg" width=400px>
</picture>

---

<!-- License -->

![GitHub License](https://img.shields.io/github/license/gdellis/restic-manager?style=for-the-badge&logoSize=auto&link=LICENSE)

<!-- Made With Rust -->
![Rust](https://img.shields.io/badge/Made%20with-Rust-orange?logo=rust&style=for-the-badge)

<!-- Workflow -->
![Latest][Latest] ![Contributors][Contributors] ![Open PRs][Open PR]
[![Build][Build-svg]][Build-Workflow] [![Tests][Test-svg]][Test-Workflow] [![PR
Review][PR-Review-svg]][PR-Review-Agent] [![Coverage][Coverage-svg]][Coverage-Workflow]

<!-- Badge Link References -->
[Latest]: https://badgen.net/github/tag/gdellis/restic-manager
[Contributors]: https://badgen.net/github/contributors/gdellis/restic-manager
[Open PR]: https://badgen.net/github/open-prs/gdellis/restic-manager

[Build-svg]: https://github.com/gdellis/restic-manager/actions/workflows/build.yml/badge.svg
[Build-Workflow]: https://github.com/gdellis/restic-manager/actions/workflows/build.yml
[Test-svg]: https://github.com/gdellis/restic-manager/actions/workflows/test.yml/badge.svg
[Test-Workflow]: https://github.com/gdellis/restic-manager/actions/workflows/test.yml

[PR-Review-svg]: https://github.com/gdellis/restic-manager/actions/workflows/pr_agent.yml/badge.svg
[PR-Review-Agent]: https://github.com/gdellis/restic-manager/actions/workflows/pr_agent.yml

[Coverage-svg]: https://github.com/gdellis/restic-manager/actions/workflows/coverage.yml/badge.svg
[Coverage-Workflow]: https://github.com/gdellis/restic-manager/actions/workflows/coverage.yml

---

> CLI tool for managing restic backups with scheduling and notifications

`restic-manager` simplifies backup management by providing a unified interface
to configure jobs, schedule backups with cron, and receive notifications via
Telegram.

## Features

- **Job-based backup management** — Define backup jobs in YAML with paths,
  exclusions, and retention policies
- **Scheduled execution** — Run backups automatically via cron expressions
- **Telegram notifications** — Get notified on job success or failure
- **Pre/post backup hooks** — Run custom commands before or after backups
- **Retention policies** — Automatic snapshot pruning with configurable keep
  rules
- **Multiple repositories** — Manage multiple restic repositories from one
  config
- **Terminal UI dashboard** — browse jobs/repositories, view snapshots, and
  trigger actions from `restic-manager tui`

## Quick Start

### Prerequisites

- [restic](https://restic.net/) installed and in PATH
- Rust 1.75+

### Build

```bash
cargo build --release
```

### Configuration

Create `~/.config/restic-manager/config.yaml`:

```yaml
repositories:
  local:
    repo: /backup/my-repo
    password_key: restic-password

jobs:
  documents:
    repository: local
    paths:
      - /home/user/documents
    exclude_patterns:
      - "*.tmp"
      - ".cache/**"
    schedule: "0 2 * * *"  # 2 AM daily
    retention:
      keep_daily: 7
      keep_weekly: 4
      keep_monthly: 6
    notifications:
      on_failure: true
      on_success: false
    pre_backup:
      - type: Command
        command: /usr/local/bin/db-dump.sh
        args: []
        continue_on_error: false  # abort the backup if this hook fails (default)
```

Create `~/.config/restic-manager/secrets.yaml` (gitignored):

```yaml
restic-password: your-secret-password
telegram:
  bot_token: your-bot-token
  chat_id: your-chat-id
```

This file holds plaintext repository passwords and your Telegram bot token, so restrict it to your
own user (`chmod 600 ~/.config/restic-manager/secrets.yaml`). `restic-manager` warns at startup if
it detects the file is readable or writable by group or others.

### Usage

```bash
# Run a backup job
restic-manager run documents

# Restore latest backup (--target is required to avoid accidentally
# overwriting the current directory)
restic-manager restore documents --target /path/to/restore

# Restore specific snapshot
restic-manager restore documents --snapshot abc123 --target /path/to/restore

# List snapshots
restic-manager list documents

# Check repository integrity
restic-manager check documents

# Prune old snapshots
restic-manager prune documents

# Start the scheduler daemon (runs in the foreground; see
# "Running as a systemd service" below to run it in the background)
restic-manager daemon

# Open the interactive terminal dashboard
restic-manager tui

### TUI keybindings

| Key | Action |
| --- | --- |
| `↑` / `↓` or `j` / `k` | Select sidebar item |
| `n` / `k` | Move selection inside the current list (`j` also moves down) |
| `Enter` | Select highlighted job/repository and load details |
| `r` | Run selected job |
| `R` | Restore (full overlay coming in a future release) |
| `l` | List snapshots for selected job |
| `p` | Prune selected job |
| `c` | Check selected job |
| `L` | Switch to Logs view |
| `?` | Toggle help overlay |
| `q` / `Esc` / `Ctrl-C` | Quit |

# List configured jobs and repositories
restic-manager jobs
restic-manager repos

# Initialize a new repository
restic-manager init local
```

### Running as a systemd service

`restic-manager daemon` intentionally runs in the foreground and does not fork into the
background — that is the correct shape for a modern service manager, which handles
backgrounding, restarts, and logging for you. Unit files for both system-level and user-level
operation ship in [`contrib/systemd/`](contrib/systemd/).

System-level (runs as root, or uncomment `User=` in the unit):

```bash
sudo cp contrib/systemd/restic-manager.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now restic-manager
```

The unit's `ExecStart=` assumes the binary is at `/usr/local/bin/restic-manager`; if
`which restic-manager` says otherwise, adjust the path in the unit first — a wrong path fails
the service on start until the start limit stops the retries.

User-level (runs as your user, config read from `~/.config/restic-manager`):

```bash
mkdir -p ~/.config/systemd/user
cp contrib/systemd/restic-manager.user.service ~/.config/systemd/user/restic-manager.service
systemctl --user daemon-reload
systemctl --user enable --now restic-manager

# Start at boot without requiring a login session:
loginctl enable-linger $USER
```

Logs go to the journal (`journalctl -u restic-manager -f`, or `journalctl --user -u
restic-manager -f` for the user unit). Set `RUST_LOG` in the unit's `Environment=` to change
verbosity; the default `info` is right for normal operation. Avoid `debug`/`trace` on shared
hosts — those levels can write backup paths and restic command details into the journal.

If the daemon fails to start 5 times within 5 minutes (for example, a broken config), systemd
stops retrying and the service stays down — check `journalctl -u restic-manager -n 200` and
`systemctl reset-failed restic-manager` after fixing the cause.

On shutdown (`systemctl stop`, which sends SIGTERM) the daemon stops scheduling new jobs and
waits for in-flight backups to finish before exiting, so a running restic job is never killed
mid-write. The shipped units use `KillMode=mixed` and a generous `TimeoutStopSec` to preserve
that behavior — see the comments in the unit files.

## Configuration Reference

### config.yaml

| Field | Description |
| --- | --- |
| `repositories.<name>.repo` | Restic repository path |
| `repositories.<name>.password_key` | Key in secrets.yaml for repository password |
| `jobs.<name>.repository` | Repository name to use |
| `jobs.<name>.paths` | List of paths to backup |
| `jobs.<name>.exclude_patterns` | List of exclude patterns, written to a per-job exclude file |
| `jobs.<name>.exclude_file` | Path to an existing exclude file; takes precedence over `exclude_patterns` |
| `jobs.<name>.schedule` | Cron expression for scheduling |
| `jobs.<name>.retention` | `keep_daily`/`keep_weekly`/`keep_monthly`/`keep_yearly`/`keep_hourly`/`keep_last` |
| `jobs.<name>.notifications` | `on_failure`/`on_success` |
| `jobs.<name>.pre_backup` | Hooks to run before backup (skipped in `--dry-run`) |
| `jobs.<name>.post_backup` | Hooks to run after backup (skipped in `--dry-run`) |
|   ↳ hook `continue_on_error` | If `true`, a failed hook warns instead of aborting the backup |

### secrets.yaml

| Field              | Description                                          |
| ------------------ | ---------------------------------------------------- |
| `<key>`            | Arbitrary key-value pairs referenced by password_key |
| `telegram.bot_token` | Telegram bot token                                   |
| `telegram.chat_id`   | Telegram chat ID for notifications                   |

## Development

```bash
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check
cargo coverage
```

## Architecture

```mermaid
flowchart TB
    CLI["CLI Layer<br/>run · restore · prune · list · check · daemon · jobs"]

    subgraph Core[" "]
        Config[Config]
        Secrets[Secrets]
        Scheduler[Scheduler]
    end

    subgraph Executor[" "]
        Backup[Backup]
    end

    subgraph External[" "]
        Restic[Restic]
        Telegram[Telegram]
        Hooks[Hooks]
    end

    CLI --> Config
    CLI --> Secrets
    CLI --> Scheduler
    Config --> Backup
    Secrets --> Backup
    Scheduler --> Backup
    Backup --> Restic
    Backup --> Telegram
    Backup --> Hooks
```

## License

![GitHub License](https://img.shields.io/github/license/gdellis/restic-manager)

MIT
