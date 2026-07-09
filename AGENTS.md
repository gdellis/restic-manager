# AI Agent Instructions

This file contains instructions for AI coding agents working in this repository.

## Project Overview

A Rust project for managing restic backups.

## Commands

### Development

- `cargo build --all-targets` - Build the project
- `cargo run` - Run the project
- `cargo test --all-targets` - Run tests
- `cargo clippy --all-targets --all-features -- -D warnings` - Run linter (matches CI; plain
  `cargo clippy` is not sufficient)
- `cargo fmt --check` - Verify formatting (matches CI; plain `cargo fmt` rewrites files instead of
  checking them)
- `cargo check` - Check for errors
- `npx markdownlint-cli "**/*.md"` - Lint Markdown files (matches the pre-commit hook; any edited
  `.md` file must pass this, see `.markdownlint.json` for the 120-char line-length limit. CI itself
  uses `markdownlint-cli2`, which enforces the same rule config but is a different tool version)

### Git Operations

```bash
git checkout -b feature/your-feature
# Make changes
git add .
git commit -m "Description of changes"
git push -u origin feature/your-feature
```

## Rules Reference

- [Rust Rules](https://github.com/gdellis/agent-files/raw/refs/heads/main/rules/rust.md)
- [Git Rules](https://github.com/gdellis/agent-files/raw/refs/heads/main/rules/git.md)
- [Markdown Rules](https://github.com/gdellis/agent-files/raw/refs/heads/main/rules/markdown.md)

## Code Style

- Use `cargo fmt --check` to verify formatting
- Use `cargo clippy --all-targets --all-features -- -D warnings` to lint
- Use `Result<T, E>` over panics
- Write doc comments for public functions

## Key Rules

1. Never commit directly to main
2. Run clippy before committing
3. Write tests for new features
4. `.pre-commit-config.yaml` is the authoritative source of pre-submit checks (fmt, clippy,
   markdownlint, plus generic YAML/whitespace hooks). If `pre-commit` is installed, `pre-commit
   run --all-files` runs everything above in one command.
