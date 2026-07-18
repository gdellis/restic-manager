# AI Agent Instructions

This file contains instructions for AI coding agents working in this repository.

## Project Overview

A Rust project for managing restic backups.

## Branch Workflow

`main` is protected. All work happens on a feature branch; `main` only accepts PR merges.

- One logical change per branch. Smaller PRs review and revert cleanly.
- Branch name: `type/short-kebab-description` (e.g. `fix/log-tick-env`, `feat/restic-unlock`).
- Push the branch and open a PR. Wait for CI to pass and a review (if any) before merging.
- Do not commit, merge, or rebase directly on `main`, even with `--no-ff`. If a commit lands there by mistake, follow
  the recovery procedure below before doing anything else.

### Recovery: a commit landed on `main` by mistake

Use this when feature work was merged locally instead of via PR. The reset is local; `origin/main` is unchanged, so no
force-push is needed. Do steps 0 and 1 in this order before touching anything else.

0. **Halt the AI/coding session** so no further commits are created while you recover. The current Claude turn, any
   subagents, and any in-flight shell commands should be stopped or finished before touching `main`.
1. **Save the feature-commit SHAs before they become unreachable.** After a `git reset --hard origin/main` the feature
   commits survive only as long as the reflog retains them (default 90 days, or until `git gc --prune=now`). Run
   `git fsck --no-progress --unreachable --dangling` and write down the listed SHAs. If the commit you need is not
   listed, this clone cannot recover it. Also note the previous main tip from `git reflog` for safety.
2. `git fetch origin` and `git reset --hard origin/main` to roll main back to its remote state.
3. For each feature commit (in priority order), `git branch <branch> <commit-sha>` from the single commit, not the
   merge commit. The merge commits are discarded - the PRs end up as one clean commit each.
4. `git push -u origin <branch>` and `gh pr create --base main --head <branch>` for each one.
5. Leave `main` at `origin/main`. The abandoned merge commits stay in the reflog for safety.

### Enforcement layers

**GitHub branch protection on `main` is the only unconditional enforcement.** Once enabled (see
`### Enabling GitHub branch protection` below), even an admin cannot push directly. Per-developer safeguards in
the local coding agent (opencode, Claude Code) are *deterrents*, not backstops — they live in the developer's
dotfiles, are out of scope for this project, and will drift as the tools change. **Do not treat them as a
substitute for branch protection.**

### Enabling GitHub branch protection

Requires repo admin. The rule requires a PR for any change to `main`, enforces admin bypass off, and pins the
required status checks to the existing CI workflows (`build` and `test`):

```bash
gh api \
  --method PUT \
  -H "Accept: application/vnd.github+json" \
  /repos/gdellis/restic-manager/branches/main/protection \
  -f required_pull_request_reviews[required_approving_review_count]=0 \
  -f required_pull_request_reviews[dismiss_stale_reviews]=false \
  -f required_status_checks[strict]=false \
  -F required_status_checks[contexts]=build \
  -F required_status_checks[contexts]=test \
  -f enforce_admins=true \
  -F restrictions=null
```

Adjust the `contexts` array if the CI job names in `.github/workflows/*.yml` change. Verify with
`gh api /repos/gdellis/restic-manager/branches/main/protection`.

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

1. Never commit directly to main. See `## Branch Workflow` for the rule, the recovery procedure, and how
   branch protection is the unconditional backstop.
2. Run clippy before committing
3. Write tests for new features
4. `.pre-commit-config.yaml` is the authoritative source of pre-submit checks (fmt, clippy,
   markdownlint, plus generic YAML/whitespace hooks). If `pre-commit` is installed, `pre-commit
   run --all-files` runs everything above in one command.
