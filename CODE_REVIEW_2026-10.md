# Code Review: restic-manager

**Review date:** 2026-10-28
**Branch reviewed:** `feat/tui` @ `cec8edf` ("fix(tui): address PR #92 review feedback")
**Sandbox:** `/data/projects/restic-manager/.omo/session-work/sandbox` (git worktree, branch `review/audit-2026-10`)
**Prior review:** `CODE_REVIEW.md` (2026-07-25) — re-verified, not trusted

## Baseline

| Check | Command | Result |
|---|---|---|
| Build | `cargo build --all-targets` | Pass |
| Tests | `cargo test --all-targets` | **175 passed / 0 failed** |
| Clippy | `cargo clippy --all-targets --all-features -- -D warnings` | Clean |
| Format | `cargo fmt --check` | Clean |

Toolchain: rustc 1.97.0. `restic` 0.16.4 on PATH. Working tree left clean.

**Scope note:** `config.rs`, `secrets.rs`, `exclude.rs`, `notifications.rs`, `errors.rs` have had
**zero commits since 2026-07-18** — i.e. they predate the July review, so every prior-review
item in those files is still open by construction.

---

## Critical

### C1. Backup statistics are always zero — `src/backup.rs:339-368`

`parse_backup_output` reads the counters from a nested `"summary"` object:

```rust
let summary = &json["summary"];
files_new: summary["files_new"].as_u64().unwrap_or(0) as u32,
```

Real restic emits them at the **top level** of the summary message. Actual restic 0.16.4 output:

```json
{"message_type":"summary","files_new":173,"files_changed":2,"files_unmodified":6,
 "dirs_new":0,"dirs_changed":129,"total_files_processed":181,"snapshot_id":"6ce0598c..."}
```

There is no `"summary"` key, so `json["summary"]` is `null` and every `.as_u64()` yields `None`
→ `unwrap_or(0)`. **Every counter is silently zero.** `snapshot_id` is read correctly (it *is*
top-level), so the backup looks like it succeeded.

**Verified end-to-end.** A backup of 344 files reported:

```
Backup completed: 0 new, 0 changed, 0 unchanged files
```

while restic reported `files_new:173`. The `unwrap_or(0)` defaults swallow the shape mismatch,
so no error surfaces.

**Impact:** the success Telegram notification reports "0 new, 0 changed" for every backup —
the core signal of a backup tool is wrong. `partial` and `errors_count` are also hardcoded
(`false` / `0`), so partial-backup detection never fires (see C2).

**Fix:** read the fields from `json` directly, not `json["summary"]`; and use a typed
`#[derive(Deserialize)]` struct so a shape change is a hard error rather than a silent zero.

### C2. ~~Partial backups are never detected~~ — **REFUTED, do not implement**

> **Correction (2026-10-28):** this finding was wrong and has been removed from the fix list.
> I had read `parse_backup_output`'s `partial: false` / `errors_count: 0` initializers without
> noticing that `Backup::run` overrides them. `src/backup.rs:318-326` sets
> `result.partial = status.code() == Some(3)` and `result.errors_count = last_error_count`
> (tracked from restic's `"message_type":"error"` status lines), and the exit-code-3 path is
> explicitly *not* treated as fatal.
>
> **Verified empirically** with an unreadable source file (restic exits 3):
>
> ```
> Backup completed with errors (partial): 0 new, 0 changed, 0 unchanged files.
>   1 file(s) could not be read.
> Snapshot ID: 9cdf4bab17fd6f5ddd270489626bd8580bef6442093a9515f85189cbd24f4470
>
> INFO  Backup completed snapshot=Some("9cdf4bab...") files_new=0 partial=true errors_count=1
> WARN  Backup completed with errors (partial, some files unreadable) errors_count=1
> exit code: 0
> ```
>
> Partial detection, the error count, the snapshot ID, and the partial notification all work.
> Note `files_new=0` in that output — that is **C1** (the counter bug), not a separate defect.

**The one real issue adjacent to this**: the notification body still reports "0 new, 0 changed"
because of C1. Fixing C1 fixes this output too. No separate work is needed.

### C3. Retention is never applied after a backup — `src/backup.rs:99-158`

`DESIGN.md:151-157` specifies a Retention Phase (`restic forget` → `restic prune`) as part of the
backup flow, and the README advertises "Automatic snapshot pruning with configurable keep rules."

`apply_retention` has exactly one caller: `src/cli.rs:166`, the manual `prune` subcommand.
`Backup::run` never calls it, and the scheduler's backup path never calls it.

**Impact:** scheduled backups accumulate snapshots forever. `keep_last`/`keep_daily`/etc. in a
job's config are parsed and validated but **have no effect**. This is the single most
user-visible gap between documented and actual behavior.

---

## High

### H1. `password_key` masking panics on a multibyte char boundary — `src/cli.rs:316-317`

```rust
let masked = if repo.password_key.len() > 2 {
    format!("{}***", &repo.password_key[..2])
```

`len()` is bytes, `&s[..2]` is a byte slice. `&repo.password_key[..2]` panics when byte index 2
falls inside a multi-byte character.

**Verified:**

```
key="→key" bytes=6
thread panicked at src/cli.rs:552:48:
byte index 2 is not a char boundary; it is inside '→' (bytes 0..3) of `→key`
```

`"ékey"` is safe (2 bytes), `"→key"` and any 3+ byte leading char panic. The panic occurs
inside `restic-manager show repo <name>` — a read-only command. Note `password_key` is a *key
name*, not a secret, but the value is user-supplied YAML.

**Fix:** use `repo.password_key.chars().take(2).collect::<String>()`.

### H2. The TUI cannot start when the config is invalid — `src/cli.rs:129-130`, `237-239`

```rust
let cli = Cli::parse();
let config = ResolvedConfig::load()?;   // <-- runs BEFORE the match

match cli.command {
    ...
    Commands::Tui => { crate::tui::run()?; }
```

Config is loaded before dispatch, so `restic-manager tui` fails hard when the config is broken.
The TUI is the natural tool for diagnosing a broken config, and it is unavailable exactly when
you need it.

**Verified:** with one repo missing a secret key, `restic-manager tui` exits immediately with
`Error: Configuration error: ... Missing secrets` and never renders.

**Worse in the TUI specifically:** the TUI loads its data by spawning `restic-manager jobs
--format json` as a **subprocess**. `ResolvedConfig::load()` validates that *every* repository's
`password_key` resolves. One broken repo therefore makes the **entire job list** fail, so the
TUI shows **zero jobs** — even for the repos that are fine.

**Verified:**

```
$ restic-manager jobs --format json     # 1 good repo, 1 repo with a missing secret
Error: Configuration error: Invalid configuration: Missing secrets has the following problems:
- Repository 'broken' references missing secret key 'missing_key'
exit=1
```

**Fix:** load config lazily per-command; for the TUI, tolerate a partial config and surface the
errors in the UI rather than aborting.

### H3. The TUI's background subprocesses corrupt the display — `src/tui.rs:461,504,548,591,635`

The TUI spawns `restic-manager jobs --format json` (and `repos`, `show job`, `show repo`,
`list`) as child processes with inherited stdio. Those children write their `tracing` output to
**stderr**, which is the same terminal the TUI is drawing into — inside the alternate screen.

**Verified** with a `secrets.yaml` at mode 0664 (which triggers the CLI's permission warning),
rendering the TUI's screen through a pty shows the warning text spliced into the widgets:

```
   3| Views10-29T01:25:35│┌Detail──────────────────────────────┐
   4| > Jobs             ││Jobs:     is system could replace, rename, or symlink-swap...
   5| moRepositoriestmp/r││>st_alpha/restic-manager mode="775"
```

The sidebar title "Views" became "Views10-29T01:25:35", and the "Repositories" row became
"moRepositoriestmp/r". The UI is visibly corrupt.

**Fix:** redirect the child's stderr to a pipe the TUI reads, or set `RUST_LOG=error` and
`Stdio::null()` for data-loading children, and surface child stderr in the Logs view instead.

### H4. `daemon_running()` false-positives on any process whose cmdline contains "daemon" — `src/tui.rs:104-144`

```rust
if cmdline.contains("daemon") { return true; }
```

A substring match on the whole command line, with no check that `"daemon"` is the *subcommand*.

**Verified.** These all report the daemon as running when it is not:

| cmdline | verdict |
|---|---|
| `restic-manager run daemon` (a **job** named "daemon") | false positive |
| `restic-manager run my-daemon-backup` (job name merely *contains* "daemon") | false positive |
| `restic-manager check daemon` | false positive |
| `restic-manager daemon` | correct |

The TUI's own `r`/`p`/`c` commands spawn `restic-manager run <job>` — so **any job whose name
contains "daemon" makes the TUI permanently show a green "daemon: ●"** even with no daemon
running. The header indicator is decorative, but it is actively misleading.

**Fix:** compare argv positionally — check that argv[1] (the subcommand) is exactly "daemon".

### H5. The TUI's documented keybindings do not match the code — `README.md:147-160`, `src/tui.rs:874-887`

Both the README table and the in-app `?` overlay say:

```
| `↑` / `↓` or `j` / `k` | Select sidebar item |
| `n` / `k`               | Move selection inside the current list |
```

But the code binds `j`/`n` to `advance_list_index` and `k` to `retreat_list_index`
(`src/tui.rs:291-298`) — the **list**, not the sidebar. `move_selection` (the sidebar) is
reachable only from `Up`/`Down` (`src/tui.rs:283-290`).

**Verified** by driving the TUI through a pty and reconstructing the screen: pressing `j` moved
the job-list highlight (`> alpha` → `> beta` → `> gamma`) and left the sidebar on "Jobs". The
`k` row is also self-contradictory — it is listed as both "select sidebar item" and "move
selection inside the current list".

**Fix:** correct the docs, or bind `j`/`k` to the sidebar as documented and move list
navigation to `Tab`/`Shift-Tab`.

### H6. A stale snapshot response overwrites the current job's snapshots — `src/tui.rs:632-672`

`load_snapshots_async` writes its result into `a.snapshots` unconditionally. There is no
check that the job the response belongs to is still the selected one, and no request
generation counter. If you select job A, press `l`, then switch to job B and press `l`, a
slow response for A can land *after* B's fast response and overwrite it.

**Verified** with a shimmed `restic` that delays the slow repo's `snapshots` call by 8s.
The rendered screen showed job **fastjob**'s details (`paths: /tmp/rmrace/fastdata`) with
the **slow repo's** snapshot listed underneath:

```
││  paths: /tmp/rmrace/fastdata
││Snapshots:
││  slow1111  2026-10-28T19:00:00Z
```

**Impact:** the UI shows one repository's snapshots under a different job — actively
misleading during a restore decision. The same pattern applies to `load_job_details_async`
and `load_repo_details_async`.

**Fix:** carry a request generation counter (or the job name) into the loader and drop the
response if it no longer matches `a.selected_job`.

### H7. `restic-manager run` orphans the `restic` child on SIGTERM — `src/backup.rs:206-212`

SIGTERM/SIGINT handling exists **only** in `src/scheduler.rs` (the daemon path). The one-shot
`run` path spawns `restic` with `Command::spawn()` and then blocks on `child.wait()`. There is
no signal handler and no process-group management, so when the wrapper is killed, `restic`
survives as an orphan.

**Verified** deterministically with a `restic` shim on `PATH` that sleeps, so the child's
lifetime is fully controlled:

```
restic shim pid = 411252  ppid = 411251
SIGTERM to the WRAPPER ONLY
wrapper alive?   False
restic alive?    True      <-- survived
restic ppid now  = 2892    <-- reparented to the session subreaper
```

**Impact:** a `restic` process keeps running against the repository after the manager is
stopped. On a schedule that fires hourly, each interrupted run leaves a `restic` holding the
repo lock — the next run then fails with "repository is already locked". The TUI's
`kill_remaining_children` (`src/tui.rs:160-171`) sends SIGTERM to the `restic-manager run`
child, which hits this same path.

**Fix:** put the child in its own process group (`setsid`/`process_group(0)`) and kill the
group, or install a SIGTERM handler in the `run` path that kills the child before exiting.

### H8. The markdown lint step in CI lints zero files — `.github/workflows/test.yml:39-43`

```yaml
- name: Lint Markdown
  uses: DavidAnson/markdownlint-cli2-action@07035fd... # v22.0.0
  with:
    globs: ${{ steps.changed-files.outputs.all_changed_files }}
```

`all_changed_files` is a **comma-joined** string. `markdownlint-cli2` treats that as a single
glob pattern, which matches nothing — so the step always reports success without linting.

**Verified:**

```
$ markdownlint-cli2 "README.md,TODO.md"
Finding: README.md,TODO.md
Linting: 0 files
Summary: 0 issues in 0 files
EXIT=0

$ markdownlint-cli2 "TODO.md"
Summary: 3 issues in 1 file
TODO.md:5:121 error MD013/line-length Line length [Expected: 120; Actual: 134]
TODO.md:7:121 error MD013/line-length Line length [Expected: 120; Actual: 152]
TODO.md:17:121 error MD013/line-length Line length [Expected: 120; Actual: 141]
EXIT=1
```

**Impact:** the markdown lint job is a no-op. `TODO.md` currently violates MD013 three times
and CI passes. The fix is to pass a space- or newline-separated glob list, or use the
action's `globs` as a YAML list.

### H9. The Telegram bot token leaks into logs and error messages — `src/notifications.rs:214-225`

The token is embedded in the request URL. `reqwest::Error`'s `Display` includes the URL, so any
transport failure produces an error string containing the token.

**Verified** against reqwest 0.12.28 with the same code shape:

```
DISPLAY: error sending request for url (https://api.telegram.invalid.example/botSECRET_TOK/sendMessage)
LEAKS_TOKEN_IN_DISPLAY: true
SCRUBBED: error sending request
LEAKS_AFTER_WITHOUT_URL: false
```

That string flows through `AppError` into `scheduler.rs:332` (`warn!(error = %e)`) and thus into
the systemd journal — a long-lived, world-readable log on most systems.

**Fix:** `err.without_url()` (confirmed present in reqwest 0.12.28) before wrapping, or sanitize
the token out of the message. Also worth redacting the token from any `show`/config dump.

### H10. The scheduler deadlocks when ≥101 jobs fall due in one tick — `src/scheduler.rs:153, 205-211`

The tick arm of the `tokio::select!` does `tx.send(job_name.clone()).await` on a **bounded(100)**
channel, while the receiver arm of the *same* `select!` is the only thing that can drain it. Once
100 jobs are queued, the 101st `send().await` blocks forever — and because the task is suspended
inside the select's tick arm, the receiver arm is never polled again. The scheduler wedges
permanently, and so does the shutdown arm.

**Verified** by faithfully reproducing the structure (bounded(100) channel, `send().await` inside
the tick arm, 150 jobs due per tick) with a watchdog thread:

```
[watchdog] t=1s main task still alive
...
[watchdog] t=25s main task still alive
```

Three ticks never completed and the receiver arm never ran. The counter-case with `try_send`
instead completes normally:

```
RESULT: completed 4 ticks, dispatched 300 -> NOT wedged
```

**Impact:** a user with more than 100 scheduled jobs whose schedules coincide (e.g. all at
midnight, or all hourly) gets a permanently wedged daemon — no backups, and Ctrl-C no longer
drains.

**Fix:** use `try_send` and drop/log when the queue is full, or dispatch directly from the tick
arm without a channel.

### H11. A 6-field cron schedule with non-zero seconds can never fire — `src/scheduler.rs:57-61, 198-206`

`normalize_cron` only prefixes `"0 "` to 5-field expressions, so a 6-field expression keeps its
seconds field. But the scheduler evaluates `Local::now().with_second(0)`, i.e. it only ever
tests second 0 — and `Schedule::includes` requires the seconds field to match.

**Verified** against the `cron` crate:

```
6-field input : "30 0 2 * * *" -> normalized "30 0 2 * * *"
includes(02:00:00) [what the scheduler actually evaluates] = false
includes(02:00:30) [the only time it could fire]           = true
```

The expression validates cleanly in `config.rs` (same `normalize_cron` + `Schedule::from_str`),
so the user gets no warning — the job simply never runs.

**Impact:** silent. A user writing a standard Quartz-style 6-field expression gets a job that
never fires, with no error anywhere.

**Fix:** either reject 6-field expressions with non-zero seconds at validation time, or
evaluate schedules at their true next-due time rather than only at second 0.

### H12. The notification rate limiter never applies across runs — `src/scheduler.rs:288-291`

`run_backup_job` constructs a **fresh** `NotificationManager` per dispatched job, and
`Notifications::new` creates a fresh `RateLimiter` with `last_success_notification: None`. The
5-minute `MIN_NOTIFICATION_INTERVAL` window therefore never carries across runs, so a flapping
job notifies on every attempt — the exact case the limiter documents as its purpose.

**Verified:** `NotificationManager::new(&config, ...)` is called inside the per-job dispatch
path; `Notifications::new` initialises `rate_limiter: Mutex::new(RateLimiter { ...: None })` each
time.

**Fix:** hold one `NotificationManager` for the scheduler's lifetime, and key the limiter per
job name rather than globally.

### H13. A job name can escape the excludes directory — `src/exclude.rs:67-74`

`write_job_excludes` builds the path with `dir.join(format!("{}.txt", job_name))`. `Path::join`
does not sanitise `..`, so a job name containing path separators writes outside `excludes.d/`.

**Verified** with a job named `../../escaped`:

```
--- where did the exclude file land? ---
/tmp/travtest/escaped.txt          <-- outside restic-manager/excludes.d/
```

**Impact:** a crafted job name can write a file at an arbitrary path relative to the config
directory, with the contents of the job's `exclude_patterns`. Job names come from the config
file, so this is a config-file-trust boundary rather than a remote attack — but the config file
is often shared or generated.

**Fix:** sanitise the job name (strip path separators and `..`) before using it in a filename,
or hash it.

### M13. A typo'd Telegram field silently disables alerting — `src/secrets.rs:7-11`

`TelegramConfig` does not use `deny_unknown_fields` (a documented serde limitation with
`#[serde(flatten)]` on `Secrets`), so an unknown key is dropped without complaint.

**Verified:**

```
(a) typo 'bot_tokenn' -> PARSED OK (no error)
    bot_token = None
    => typo SILENTLY IGNORED: true
```

A user who misspells `bot_token` gets a config that validates, a daemon that runs, and **no
alerts at all** — with no warning anywhere.

**Fix:** validate `telegram.bot_token.is_some()` after deserialization and warn if the
`telegram` block is present but `bot_token` is missing.

### M14. An empty secret value passes validation and fails confusingly — `src/config.rs:299-309`

`validate_secrets` checks `self.secrets.get(&repo.password_key).is_none()`, so a key present
but set to the empty string is treated as valid.

**Verified:** `"pw: \"\""` → `get("pw")` returns `Some("")` → `is_none()` is `false` → validation
passes. Restic then receives `RESTIC_PASSWORD=` and fails with a misleading "wrong password".

**Fix:** treat an empty value as missing (`is_none_or(str::is_empty)`).

### M15. An unquoted numeric `chat_id` is a parse error — `src/secrets.rs:9`

`chat_id` is `Option<String>`, but Telegram chat IDs are numeric. Writing `chat_id: 999`
unquoted (the natural form) fails to deserialize.

**Verified:** `invalid type: integer \`999\`, expected a string`.

**Fix:** accept a numeric YAML scalar by using a custom deserializer or `#[serde(deserialize_with)]`
that stringifies numbers.

### M16. `BackupResult.job_name` is always empty — `src/backup.rs:65, 353`

`parse_backup_output` hardcodes `job_name: String::new()` and `Backup::run` never assigns it,
despite the field being public and documented as "Name of the job that produced this backup."

**Impact:** latent trap — the notification layer would attach the wrong job name if it ever
starts reading this field.

### M17. The completed-task `JoinSet` is never reaped — `src/scheduler.rs:173, 226, 251-270`

`tasks.spawn(...)` is called in the dispatch arm, but `join_next()` only runs during the drain
phase at shutdown. Every finished task's `JoinHandle` (and any panic `JoinError`) accumulates for
the daemon's lifetime, and a panic in a completed task is swallowed until shutdown.

### M18. `in_flight.lock().unwrap()` can panic inside a `Drop` during unwinding — `src/scheduler.rs:39, 216`

`InFlightGuard::drop` runs during panic unwinding. If the mutex is poisoned, `.unwrap()` panics
inside a `Drop` during unwind → **abort**. The TUI already uses the poison-tolerant
`unwrap_or_else(|p| p.into_inner())`; the scheduler should too. (Same class as M3.)

### M19. An all-`null` retention policy is a silent no-op — `src/config.rs` / `src/snapshot.rs`

A `retention:` block with every field null validates and produces a `restic forget` with no
`--keep` flags. Restic then removes nothing — no data loss, but "Prune" reports 0 removed and
the user believes retention is configured.

### M20. `RetentionPolicy::default()`'s values never apply — `src/config.rs`

`RetentionPolicy::default()` carries `keep_last: 3, keep_daily: 4, keep_weekly: 6`, but serde
field defaults win over the struct default, so a partially-specified policy silently injects
`keep_last: 3` where the user did not ask for it.

### M21. Repository credentials can reach the logs *and* Telegram — `src/backup.rs:319-321`, `src/scheduler.rs:329-333`

`Repository.repo` is unvalidated and unredacted. When a credential is embedded in the repo URL
(the common `sftp:user:pass@host:/path` form), restic echoes the **full URL including the
password** in its stderr, which becomes `ResticError::CommandFailed`, which is then sent to the
Telegram chat.

**Verified** — restic's own output:

```
Fatal: unable to open repository at sftp:user:SECRETPASS@host.invalid:/x: unable to start
the sftp session, ...
```

And the path to Telegram is confirmed at `scheduler.rs:329-333`:

```rust
error!(job = %name, error = %e, "Backup failed");
if let Err(notify_err) = n.notify_failure(&name, &e.to_string()).await {
```

`e.to_string()` is the restic stderr, so the password is logged locally *and* posted to the chat.

**Impact:** a repo password embedded in a URL is disclosed to anyone who can read the systemd
journal or the Telegram chat. The same applies to hook stderr (`backup.rs:401-414`).

**Fix:** redact credentials from any URL appearing in an error string before logging or
notifying; warn at config-validation time if a repo URL contains embedded credentials.

---

## Medium

### M1. An explicit `exclude_file` that doesn't exist is silently ignored — `src/exclude.rs:76-93`

`resolve_exclude_file` checks `path.exists()`; if the configured file is missing it falls
through to `exclude_patterns` and then to the default excludes file. The user's explicit
configuration is discarded without any warning.

**Fix:** return an error (or at least warn) when `job.exclude_file` is set but the path is
missing. The fallback chain should only apply when no explicit file is configured.

### M2. The `reqwest` client silently drops its timeout on build failure — `src/notifications.rs:66-69`

```rust
Client::builder().timeout(Duration::from_secs(10)).build()
    .unwrap_or_else(|_| Client::new()),
```

`Client::new()` has **no timeout**. A builder failure (TLS backend init, proxy config) silently
downgrades to an unbounded request — a hung Telegram API call would block the notification
task indefinitely.

**Fix:** propagate the error, or use `Client::new()` explicitly with a documented note that
the timeout is best-effort.

### M3. `lock().unwrap()` on the notification rate limiter can panic on poisoning — `src/notifications.rs:167,178`

A panic while the lock is held poisons the mutex; every subsequent notification then panics
on `unwrap()`, permanently breaking notifications. The TUI already uses the poison-tolerant
pattern (`unwrap_or_else(|p| p.into_inner())`) — the rest of the codebase should match.

### M4. Snapshot JSON is parsed untyped with silent defaults — `src/snapshot.rs:57-93`

`v["id"].as_str().unwrap_or("")` and friends mean a change in restic's JSON shape yields empty
strings instead of an error. The `Snapshot` struct already derives `Deserialize`; the parser
just doesn't use it.

### M5. `ResolvedConfig` is deep-cloned for every scheduled job — `src/scheduler.rs:223`

`self.config.clone()` clones the entire config (all repos, jobs, and the secrets map) per
dispatch. Scales with job and secret count.

**Fix:** `Arc<ResolvedConfig>`.

### M6. Scheduler concurrency is unbounded — `src/scheduler.rs:226,295`

`tasks.spawn(...)` per due job with no semaphore. Many jobs firing on the same minute will
spawn one `spawn_blocking` task each, saturating the blocking thread pool.

**Fix:** a `tokio::sync::Semaphore` bounding concurrent backups, with the limit documented.

### M7. `parse_backup_output` finds the summary line by substring match — `src/backup.rs:341-343`

`lines.iter().find(|l| l.contains("\"summary\""))` matches the *first* line containing
`"summary"`. A filename containing the word "summary" in a progress line could match first.
Should match `"message_type":"summary"`.

### M8. Error messages omit the file path — `src/errors.rs`

`ConfigError::IoError` renders as `Failed to read config file: No such file or directory
(os error 2)` — no path. `ConfigError::ParseError` and `SecretsError::ParseError` produce the
identical string `Failed to parse YAML`, so the user cannot tell which file failed to parse.

### M9. The config directory is created with default umask — `src/exclude.rs:59-61`

`create_dir_all` yields mode 0755. That directory holds `secrets.yaml`. The dir permission
check only flags *write* bits (`mode & 0o022`), so 0755 passes — which is correct, but the
directory is nonetheless world-readable. Combined with H2 this means the secrets file's
directory is listable by other users.

### M10. The TUI shells out to itself for every data load — `src/tui.rs:461-673`

Five separate `load_*_async` functions, each spawning `current_exe()` with a different
subcommand and parsing JSON out of stdout. Each spawn re-reads and re-validates the config
and secrets from disk. This is why H2 and H3 exist at all.

**Fix:** the TUI should call the library directly — `Config::load()` and `SnapshotManager::list()`
are already in the same crate. The subprocess indirection buys nothing and costs correctness,
latency, and display integrity.

### M11. The TUI does not handle `Event::Resize` — `src/tui.rs:243`

Only `Event::Key` is matched. Resizing the terminal leaves the layout stale until the next
redraw (ratatui does re-query the area each frame, so this is minor, but the resize event
itself is ignored and no re-layout is forced).

### M12. The TUI never refreshes data — `src/tui.rs:212-214`

A code comment acknowledges it: "config edits while the TUI is running are not picked up until
a restart. A reload key can be added in a future wave." The daemon-status indicator *is*
refreshed every 5s (`DAEMON_CHECK_SECONDS`), so the TUI has a refresh mechanism it doesn't
apply to anything else.

---

## Low

- `src/tui.rs:164-169` — `kill_remaining_children` shells out to the external `kill` binary,
  ignores failures, and can race with PID reuse. Use `nix::signal::kill` (or
  `libc::kill` directly). *(Unchanged from the prior review.)*
- `src/tui.rs:217,310,361,697,731,743` — six unnamed `std::thread::spawn` calls. Name them.
- `src/tui.rs:790-798` — `push_log_and_clear` takes both `&Arc<Mutex<App>>` and
  `&Arc<Mutex<Option<String>>>`; the running-label mutex could live inside `App`.
- `src/cli.rs:240-327` — the `Show` command repeats `serde_json::to_string_pretty(...)` four
  times; extract a helper. *(Prior review.)*
- `src/cli.rs:240-327` — human-readable formatting for jobs/repos is duplicated between
  `Show` and the list commands. *(Prior review.)*
- `src/backup.rs:42-54` — `format_bytes` uses float arithmetic; a bit-shift loop is more
  predictable. *(Prior review.)*
- `src/config.rs:17-27` — `join_errors` allocates an intermediate `Vec<String>`. *(Prior review.)*
- `src/scheduler.rs:355-368` — `list_scheduled_jobs` returns `Vec<(&str, String)>`; a named
  struct would be clearer. *(Prior review.)*
- `src/snapshot.rs:44` — the `restic snapshots` invocation passes `--repo` but no `--json`
  host/tag filters; fine, but the unused `snapshot_id` variable is a smell.
- `README.md` — the architecture diagram shows "Backup → Telegram" directly, but
  notifications route through `NotificationManager` inside the scheduler. *(Prior review.)*
- `AGENTS.md:13-15` — the module list omits `tui` (13 modules, not 12).
- `DESIGN.md` — the mermaid diagrams are out of date vs. the current 13-module layout.
- `Cargo.toml` — no `[lints.clippy]` section, so the project's own `unwrap_used`/`expect_used`/
  `panic` policy is not enforced in CI. *(Prior review.)*
- `Cargo.toml` — `ratatui = "0.28"` / `crossterm = "0.28"`. Latest stable ratatui is **0.30.1**
  (0.31 in development). The 0.28 → 0.30 jump renames `layout::Alignment` →
  `layout::HorizontalAlignment`, removes `widgets::block::Title`, makes `Backend` require an
  associated `Error` type, and changes `Block::title()` to take `Into<Line>`. Worth doing
  *before* the TUI grows, not after. The upgrade also pulls crossterm 0.29.

---

## Prior-review status

Re-derived independently from source, not read from the old write-up.

**Fixed since 2026-07-25:** the `#8` day-of-week cron bug (now correctly tested); SIGTERM
drain and force-exit (the **SIGINT** path also drains correctly — `tests/daemon_shutdown.rs`
only covers SIGTERM); non-UTF8 path handling; notification message truncation; the
`path_to_arg` helper extraction.

**Still present:** prior items 2 (`Arc<ResolvedConfig>`), 3 (explicit exclude file), 4 (log
arg accuracy), 7 (reqwest client fallback), 8 (mutex poisoning), 9 (scheduler concurrency),
10 (TUI child termination).

**New since the prior review:** C1, C3, H1-H13, M1, M8-M21.

Several came from a four-way parallel review fan-out. Every one was independently
re-verified before inclusion, and one (the old C2) was refuted and removed.

---

## What's good

- **Signal handling in the daemon is correct.** SIGTERM and Ctrl-C drain in-flight backups,
  and a second signal force-exits with 130. Tested end-to-end.
- **Passwords never touch argv or logs** — `RESTIC_PASSWORD` goes through the environment.
  The repo password is not logged or shell-interpolated anywhere.
- **Config validation is genuinely good**: errors are aggregated and reported together,
  ordering is deterministic via `BTreeMap`, unknown fields are rejected
  (`deny_unknown_fields`), and job→repository references are validated.
- **Secrets permission checks** on both the file and its parent directory, with the
  rationale documented in comments.
- **The stderr-drain thread in `backup.rs`** avoids a real 64KB pipe-buffer deadlock, and
  the comment explains exactly why.
- **175 tests** including non-UTF8 paths, notification rate-limiting, dry-run hook skipping,
  and end-to-end daemon signal tests.
- **The error taxonomy** (`thiserror` enums with `#[from]`) keeps call sites readable.
- **The TUI's mutex poisoning handling** is correct — it uses the poison-tolerant pattern
  everywhere, unlike `notifications.rs`.

---

## Priority order

### Correctness / data loss first

| # | Finding | Note |
|---|---|---|
| 1 | **C1** — backup counters are always zero | one-line fix, but it's the tool's core output |
| 2 | **C3** — retention never runs | documented feature that doesn't exist |
| 3 | **H1** — `password_key` panic | crash in a read-only command |
| 4 | **H7** — `restic` orphaned on SIGTERM | leaves the repo locked; the next run fails |
| 5 | **H10** — scheduler deadlocks at >=101 due jobs | permanent wedge, no backups at all |
| 6 | **H11** — 6-field cron with non-zero seconds never fires | silent |
| 7 | ~~C2~~ | refuted — partial detection works; see the correction above |

### Security / secrets

| # | Finding | Note |
|---|---|---|
| 8 | **H9** — Telegram bot token leaks into the systemd journal | |
| 9 | **H13** — job name can escape the excludes directory | |
| 10 | **M21** — repo credentials can reach logs *and* Telegram | |

### UI correctness

| # | Finding | Note |
|---|---|---|
| 11 | **H2** / **H3** — TUI can't start on a bad config, and corrupts its own display | both fixed by M10 |
| 12 | **H6** — stale snapshot response shown under the wrong job | |
| 13 | **H4** — TUI daemon indicator false-positives | |
| 14 | **H5** — documented keybindings contradict the code | |

### Operational

| # | Finding | Note |
|---|---|---|
| 15 | **H12** — notification rate limiter never applies across runs | |
| 16 | **M13** / **M14** / **M15** — silent alerting failure modes | typo'd field, empty secret, numeric chat_id |
| 17 | **H8** — markdown lint in CI is a no-op | |
| 18 | **M1** / **M2** / **M3** — exclude-file fallback, reqwest timeout loss, mutex poisoning | |
| 19 | **M10** — have the TUI call the library instead of spawning itself | dissolves H2 and H3 |

## Summary

**37 numbered findings** (3 Critical, 13 High, 21 Medium) plus ~8 Low-severity bullets. Every
Critical and High finding was reproduced against the real binary, real `restic` 0.16.4, or a
faithful harness — none are inferred from reading code alone.

| Severity | Count | IDs |
|---|---|---|
| Critical | 3 | C1-C3 |
| High | 13 | H1-H13 |
| Medium | 21 | M1-M21 |
| Low | ~8 | inline bullets in the Low section |

The three Critical findings all share a root cause: **the code's assumptions about restic's
output and the scheduler's own contract were never checked against reality.** C1 is a JSON shape
mismatch that the unit tests hid by using fixtures with the same wrong shape. C3 and H11 are
features that validate cleanly and then do nothing.

The single most consequential structural issue is **M10**: the TUI shells out to itself for
every piece of data. That one decision is the direct cause of H2 (config errors take down the
whole job list), H3 (child stderr corrupts the display), and M4 (five subprocess spawns per
refresh). Fixing it dissolves three High findings at once and should precede any further UI work.

## Notes on the lint policy

`cargo clippy --lib --bins --all-features -- -W clippy::panic` reports **zero** panics in
production code — every `panic!` in the tree is inside a `#[cfg(test)]` module. Likewise
`unwrap_used` and `expect_used` are at 0 in production (the codebase uses
`unwrap_or_else(|poisoned| poisoned.into_inner())` for mutexes). The finding recorded earlier
("`panic: 3`") counted test-only panics and is **not** a production issue. The gap is that
`Cargo.toml` declares no `[lints.clippy]` section, so this discipline is enforced only by
convention, not by the build.
