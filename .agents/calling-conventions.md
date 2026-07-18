# Calling conventions

How to delegate work to subagents and how to phrase instructions so they
produce useful results.

## Delegating

- Give each delegated task **one goal**, a **clear deliverable**, and
  **exact paths or commands** to inspect.
- State required artifacts directly: evidence files, command output, or
  PR links.
- Ask implementation agents to include the verification commands they ran.
- Name the allowed work area when the task must avoid unrelated files.
- Retrieve background output before marking a delegated task complete.
  Save long subagent transcripts to `./.omo/session-work/<task>.log` when
  they are useful for later review.

## Phrasing

- Prefer **affirmative constraints**: "Keep scratch files under
  `./.omo/session-work/`" instead of broad negative phrasing.
- Specify the deliverable shape: "Reply with a list of file paths and the
  evidence you found in each" rather than "find stuff about X".
- When asking for an implementation, include the success criteria:
  "Modify `src/foo.rs` so `cargo test --lib foo` passes; report the
  command output in your final message."

## Why these matter

Subagents with vague instructions produce vague results. The more concrete
the goal, deliverable, and success criteria, the less you have to clean up
after them.
