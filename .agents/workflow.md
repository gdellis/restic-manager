# Workflow

Generic workflow for agents working on this project. The project-specific
information (layout, commands, repo quirks, build matrix, working agreement)
lives in `AGENTS.md` at the repo root.

## Loop

- Plan the work, then split independent tasks before starting edits.
- Run the cheapest useful failing check or missing-docs search before the fix.
- Apply the smallest change that satisfies the request.
- Run targeted checks and one real surface QA step.
- Summarize what changed, what passed, and any remaining risk.

## Why these steps

- **Plan first** — most mistakes are scope mistakes, not code mistakes.
- **Cheapest failing check** — proves the bug exists and gives a signal to
  re-run after the fix. Avoid guessing.
- **Smallest change** — a focused diff is easier to review and revert.
- **Real QA** — automated checks miss what users actually see. For this repo
  that means running the binary end-to-end, not just `cargo test`.
- **Summarize** — the human reviewing the work needs to know what changed and
  what to watch for.
