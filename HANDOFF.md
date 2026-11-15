# Agent Handoff — restic-manager

Read this first if you are taking over mid-stream. It records where the work stands,
what is verified, and what is next. Update it when you finish a phase.

## Repo facts

- Repo: `github.com/gdellis/restic-manager` — single-crate Rust CLI wrapping `restic`.
- Read `AGENTS.md` at repo root for commands, quirks, and the working agreement
  (feature branches only; never commit/merge to `main` directly; no merges without
  explicit user approval).
- CI runs four gates on every PR: `cargo build --all-targets`, `cargo test --all-targets`,
  `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --check`.
- Markdown PRs also run `markdownlint-cli2` (120-char limit, config in `.markdownlint.json`).
- `Cargo.lock` is gitignored — do not `git add -f` it.
- Toolchain pinned to 1.97.0 in `rust-toolchain.toml`.
- Scratch space: `.omo/session-work/`; durable QA evidence: `.omo/evidence/`.
- SSH to GitHub is flaky on this machine; use the HTTPS token form for git remote ops:
  `git fetch https://x-access-token:$(gh auth token)@github.com/gdellis/restic-manager.git ...`

## Where the work stands

**Merged this session (all squash-merged to `main`, CI green at merge time):**

| PR | What | Verification |
|---|---|---|
| #102 | markdownlint job actually lints files | local `npx markdownlint-cli2` + CI |
| #109 | October 2026 review report as reference docs | docs-only |
| #118 | ordered remediation execution plan | docs-only |
| #133 | typed snapshot JSON parsing (M4) | implementer + independent review agent |
| #135 | TUI resize + config reload (M11/M12) | implementer + review agent (nits fixed) |
| #136 | notification/scheduler lifecycle hygiene (H12/M17/M3/M18) | implementer + review agent |

**Open PRs (awaiting review/merge):**

| PR | What | Status |
|---|---|---|
| #137 | Arc<ResolvedConfig> + concurrency semaphore (M5/M6) | implemented, CI green on branch; review agent still pending (prior attempt was a phantom) |

**Earlier this session (pre-pipeline, also merged):**

- Docs Phase 0: #102, #109, #118
- Phase 1 (correctness/secrets): #103, #104, #105, #107, #119, #106
- Phase 2 (process lifecycle): #112
- Phase 3 (TUI stream): #111, #92, #108, #114, #115, #120, #121, #122
- Docs consolidation: #131, #132

Full evidence under `.omo/evidence/remediation/`.

**Open issues (the live backlog, in execution-plan order):**

| Issue | What | Status |
|---|---|---|
| #125 | M5/M6 scheduler scale (Arc + semaphore) | **PR #137 open, review pending** — start here |
| #134 | `parse_forget_output` untyped JSON (same class as M4) | open — not started |
| #127 | H13+M1 exclude hardening | open — not started |
| #128 | M2/M8/M9 error ergonomics | open — not started |
| #129 | M20 retention serde defaults | open — not started |
| #130 | low-severity bundle | open — not started |
| #74 | secret-backend abstraction | open — not started (Phase 6) |
| #45 | GUI component | icebox — needs scheduler state exposure first |

**Closed this session:** #71, #73, #80, #82, #85, #87, #123, #126.

## Process used this session (keep using it)

1. One logical change per branch: `type/short-kebab-description`, never commit to `main`.
2. Four CI gates locally before pushing: build, test, clippy `-D warnings`, fmt.
3. E2E verification with the real binary, not just unit tests (scratch configs under
   `.omo/session-work/e2e/`; evidence files under `.omo/evidence/`).
4. Every PR gets an independent review agent before merge; merge only with explicit
   user approval. Review agents are read-only (no source edits, no push, no PR comments).
5. **Beware phantom subagent results**: a couple of "subagent completed" messages this
   session were echoes, not real sessions (their session IDs did not resolve). Before
   trusting a subagent's report, verify its artifacts exist (PR number, branch, evidence
   file) — or re-run the subagent with its session ID to confirm it responds.

## Working checklist for the next agent

1. `git fetch --all --prune` and confirm `main` matches `origin/main`.
2. **First task: get PR #137 reviewed** (spawn a read-only review agent per the process
   below; it closes #125). Verify its CI is green on the latest head first.
3. Then work down the open-issues table above, one at a time, in order.
4. Work in a fresh worktree: `git worktree add .omo/session-work/<task> -b <branch> origin/main`.
5. Four gates + E2E + evidence file, then PR, then review agent, then merge (only with
   explicit user approval).

## Environment quirks

- restic 0.16.4 is on PATH; scratch configs live under `.omo/session-work/e2e/`.
- `RESTIC_MANAGER_TEST_TICK_SECS` (test-only) speeds the scheduler in integration tests.
- `RESTIC_MANAGER_CONFIG_DIR` (added this session) overrides the config directory for tests.
- AGENTS.md lists 13 modules including `tui` (added by #92 this session).
- The OpenCode review workflow is triggered only by PR comments from maintainers.
