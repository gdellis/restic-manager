# AI Agent Instructions

This file contains instructions for AI coding agents working in this repository.

## Project Overview

A Rust project for managing restic backups.

## Commands

### Development

- `cargo build` - Build the project
- `cargo run` - Run the project
- `cargo test` - Run tests
- `cargo clippy` - Run linter
- `cargo fmt` - Format code
- `cargo check` - Check for errors

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

- Use `cargo fmt` for formatting
- Use `cargo clippy` for linting
- Use `Result<T, E>` over panics
- Write doc comments for public functions

## Key Rules

1. Never commit directly to main
2. Run clippy before committing
3. Write tests for new features
