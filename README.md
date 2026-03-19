# Restic Manager

A Rust project for managing restic backups with scheduling and notifications.

## Getting Started

```bash
cargo build --release
cargo run --release
```

## Commands

```bash
cargo run -- run <job>       # Run a backup job
cargo run -- restore <job>   # Restore from a backup
cargo run -- prune <job>     # Prune old snapshots
cargo run -- list <job>      # List snapshots
cargo run -- check <job>     # Check repository integrity
cargo run -- daemon          # Run the scheduler daemon
cargo run -- jobs            # List all jobs
cargo run -- repos           # List all repositories
```

## Development

```bash
cargo build
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt
```

## Pre-commit Hooks

Install pre-commit hooks:

```bash
pip install pre-commit
pre-commit install
pre-commit run --all-files
```

## GitHub Actions

- **Test**: Runs on push to main/develop and PRs - tests, clippy, fmt
- **Build**: Runs on tags (`v*`) and PRs - cross-platform builds
- **PR Review**: AI-powered PR review using OpenCode

## License

MIT
