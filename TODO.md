# TODO

## Active backlog

1. **#71** — Gate the OpenCode reviewer behind a PR comment command (`/opencode review`) so it does not run on every PR automatically.
2. **#73** — Collect all config validation errors and report them together instead of stopping at the first failure.
3. **#74** — Design a secret-backend abstraction so non-file secret sources (env var, keyring, etc.) do not trigger false "missing password key" errors.

## Short-term code quality

- **#55** — Add doc comments to public structs and methods.
- **#46** — Pin the Rust toolchain so local clippy matches CI (1.97.0).
- **#56** — Extend `log_cli_output` to `check`, `prune`, and `forget` commands.

## Longer term / needs scoping

- **#45** — GUI component for managing backups (evaluate whether to use [blinc](https://github.com/project-blinc/blinc) or another approach).
- **#50** — Shutdown integration tests: contract-shaped assertions and better diagnostics.
