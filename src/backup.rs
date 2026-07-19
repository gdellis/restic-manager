use crate::cli_log::write_cli_output_log;
use crate::config::{Hook, ResolvedConfig};
use crate::errors::{AppError, ResticError};
use crate::exclude;
use std::path::Path;
use std::process::Command;
use tracing::{debug, info, warn};

/// Convert a path to a string suitable for use as a command argument.
/// Uses to_string_lossy() to handle non-UTF8 paths safely.
/// Regression fix for #54: to_str().unwrap_or(".") would panic or return "." on non-UTF8 paths.
pub fn path_to_arg(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn format_progress(
    files_done: u64,
    total_files: u64,
    bytes_done: u64,
    total_bytes: u64,
    errors: u64,
) -> String {
    let files_str = format!("{} files", files_done);
    let bytes_str = format_bytes(bytes_done);
    let total_str = format!("{} files {}", total_files, format_bytes(total_bytes));
    let error_str = if errors > 0 {
        format!("{} errors", errors)
    } else {
        String::new()
    };

    if error_str.is_empty() {
        format!("{}, total {}", files_str, total_str)
    } else {
        format!(
            "{}, total {} ({}), {}",
            files_str, total_str, error_str, bytes_str
        )
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} B", size as u64)
    } else {
        format!("{:.3} {}", size, UNITS[unit_idx])
    }
}

/// Summary of a single backup run, parsed from restic's JSON progress stream.
///
/// Constructed by `Backup::run` and returned to callers (the CLI, scheduler, and
/// notification layer) so they can render a result message and decide whether
/// the run counts as success, partial, or failure.
#[derive(Debug)]
pub struct BackupResult {
    /// Name of the job that produced this backup.
    pub job_name: String,
    /// Snapshot ID assigned by restic, if the run reached the summary line.
    pub snapshot_id: Option<String>,
    /// Number of new files added to the repository.
    pub files_new: u32,
    /// Number of files whose content changed since the last snapshot.
    pub files_changed: u32,
    /// Number of files identical to the previous snapshot.
    pub files_unmodified: u32,
    /// Number of new directories indexed.
    pub dirs_new: u32,
    /// Number of directories whose contents changed.
    pub dirs_changed: u32,
    /// Number of directories identical to the previous snapshot.
    pub dirs_unmodified: u32,
    /// Number of data blobs written.
    pub data_blobs: u32,
    /// Number of tree blobs written.
    pub tree_blobs: u32,
    /// Bytes added to the repository during this run.
    pub data_added: u64,
    /// Total wall-clock duration of the restic invocation, in seconds.
    pub duration_secs: f64,
    /// True if restic exited with code 3 (backup completed but some source
    /// files could not be read). A snapshot was still created in this case.
    pub partial: bool,
    /// Highest error_count seen across restic's "status" progress messages
    /// during this run. Only meaningful when `partial` is true.
    pub errors_count: u64,
}

pub struct Backup;

impl Backup {
    pub fn run(
        config: &ResolvedConfig,
        job_name: &str,
        dry_run: bool,
    ) -> Result<BackupResult, AppError> {
        let (job, repo, password) = config.resolve_job(job_name)?;

        if !dry_run {
            Self::execute_hooks(&job.pre_backup, "pre-backup")?;
        } else {
            info!(
                job = job_name,
                "Dry-run: no data will be written, skipping pre-backup hooks"
            );
        }

        let exclude_file =
            exclude::resolve_exclude_file(job, job_name).map(|p| p.to_string_lossy().to_string());

        let log_file = repo.log_cli_output.as_deref();

        let result = Self::execute_backup(
            &repo.repo,
            password,
            &job.paths,
            exclude_file.as_deref(),
            dry_run,
            log_file,
        )?;

        if !dry_run {
            Self::execute_hooks(&job.post_backup, "post-backup")?;
        } else {
            info!(
                job = job_name,
                "Dry-run: backup completed, skipping post-backup hooks"
            );
        }

        if result.partial {
            warn!(
                job = job_name,
                snapshot = ?result.snapshot_id,
                files_new = result.files_new,
                data_added = result.data_added,
                errors_count = result.errors_count,
                "Backup completed with errors (partial, some files unreadable)"
            );
        } else {
            info!(
                job = job_name,
                snapshot = ?result.snapshot_id,
                files_new = result.files_new,
                data_added = result.data_added,
                "Backup completed successfully"
            );
        }

        Ok(result)
    }

    fn join_stderr_thread(handle: std::thread::JoinHandle<String>) -> Option<String> {
        match handle.join() {
            Ok(text) if !text.is_empty() => Some(text),
            Ok(_) => None,
            Err(_) => {
                warn!("stderr reader thread panicked; stderr output unavailable");
                None
            }
        }
    }

    fn execute_backup(
        repo: &str,
        password: &str,
        paths: &[std::path::PathBuf],
        exclude_file: Option<&str>,
        dry_run: bool,
        log_file: Option<&std::path::Path>,
    ) -> Result<BackupResult, AppError> {
        info!(paths = ?paths, "Starting backup to {}", repo);

        let mut args = vec!["backup".to_string()];

        if dry_run {
            args.push("--dry-run".to_string());
        }

        args.push("--json".to_string());
        args.push("--repo".to_string());
        args.push(repo.to_string());

        if let Some(file) = exclude_file {
            args.push("--exclude-file".to_string());
            args.push(file.to_string());
        }

        for path in paths {
            args.push(path_to_arg(path));
        }

        debug!("Executing: restic {}", args.join(" "));

        if dry_run {
            debug!("DRY-RUN flag confirmed in command args");
        }

        let mut cmd = Command::new("restic");
        cmd.args(&args).env("RESTIC_PASSWORD", password);

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(ResticError::from_io)?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // Drain stderr on a dedicated thread concurrently with the stdout
        // read loop below. If we read stdout to completion (or to the
        // "summary" line) before touching stderr, restic can fill the
        // ~64KB stderr pipe buffer and block on write() while we're
        // blocked reading stdout -> deadlock.
        let stderr_handle = stderr.map(|s| {
            std::thread::spawn(move || {
                use std::io::Read;
                let mut reader = std::io::BufReader::new(s);
                let mut text = String::new();
                if let Err(e) = reader.read_to_string(&mut text) {
                    debug!("Failed to read restic stderr: {}", e);
                }
                text
            })
        });

        let mut lines: Vec<String> = Vec::new();
        // Tracks the highest error_count seen across "status" messages, so
        // a partial backup's notification can report how many files were
        // actually unreadable instead of a generic message.
        let mut last_error_count: u64 = 0;

        if let Some(stdout) = stdout {
            use std::io::{BufRead, BufReader};
            let mut stdout_reader = BufReader::new(stdout).lines();
            for line in stdout_reader.by_ref().flatten() {
                lines.push(line.clone());

                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                    if let Some(msg_type) = json.get("message_type").and_then(|v| v.as_str()) {
                        match msg_type {
                            "status" => {
                                let files_done =
                                    json.get("files_done").and_then(|v| v.as_u64()).unwrap_or(0);
                                let total_files = json
                                    .get("total_files")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let bytes_done =
                                    json.get("bytes_done").and_then(|v| v.as_u64()).unwrap_or(0);
                                let total_bytes = json
                                    .get("total_bytes")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let errors = json
                                    .get("error_count")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                last_error_count = last_error_count.max(errors);
                                if total_files > 0 {
                                    let progress = format_progress(
                                        files_done,
                                        total_files,
                                        bytes_done,
                                        total_bytes,
                                        errors,
                                    );
                                    eprint!("\r\x1B[2K{}", progress);
                                }
                            }
                            "verbose_status" => {
                                if let Some(item) = json.get("item").and_then(|v| v.as_str()) {
                                    let action =
                                        json.get("action").and_then(|v| v.as_str()).unwrap_or("");
                                    eprint!("\r\x1B[2K{}: {}", action, item);
                                }
                            }
                            "summary" => {
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        let status = child.wait().map_err(ResticError::from_io)?;

        let stderr_text = stderr_handle.and_then(Self::join_stderr_thread);

        if let Some(text) = stderr_text.as_deref() {
            warn!(stderr = text, "restic wrote to stderr");
        }

        // Write the CLI output log before handling the exit status, so a
        // hard failure (wrong password, network error, unreachable repo)
        // still gets logged - that's exactly the case log_cli_output is
        // most useful for, and it would otherwise be silently skipped by
        // the early return below.
        if let Some(log_path) = log_file {
            write_cli_output_log(log_path, &lines, stderr_text.as_deref());
        }

        // Exit code 3: backup completed but some source files could not be
        // read. Restic still creates a snapshot and writes its summary, so
        // treat this as a partial success rather than a fatal failure -
        // otherwise the already-parsed snapshot ID gets discarded and the
        // caller can't tell "nothing was backed up" from "everything except
        // a few unreadable files was backed up."
        let partial = status.code() == Some(3);
        if !status.success() && !partial {
            return Err(ResticError::CommandFailed(stderr_text.unwrap_or_default()).into());
        }

        let output = lines.join("\n");
        let mut result = Self::parse_backup_output(&output)?;
        result.partial = partial;
        result.errors_count = last_error_count;

        info!(
            snapshot = ?result.snapshot_id,
            files_new = result.files_new,
            files_changed = result.files_changed,
            partial = partial,
            errors_count = last_error_count,
            "Backup completed"
        );

        Ok(result)
    }

    fn parse_backup_output(output: &str) -> Result<BackupResult, AppError> {
        let lines: Vec<&str> = output.lines().collect();
        let summary_line = lines
            .iter()
            .find(|l| l.contains("\"summary\""))
            .ok_or_else(|| AppError::Other("No summary in backup output".into()))?;

        let json: serde_json::Value = serde_json::from_str(summary_line)
            .map_err(|e| AppError::Other(format!("Failed to parse backup JSON: {}", e)))?;

        let summary = &json["summary"];

        Ok(BackupResult {
            job_name: String::new(),
            snapshot_id: json["snapshot_id"].as_str().map(String::from),
            files_new: summary["files_new"].as_u64().unwrap_or(0) as u32,
            files_changed: summary["files_changed"].as_u64().unwrap_or(0) as u32,
            files_unmodified: summary["files_unmodified"].as_u64().unwrap_or(0) as u32,
            dirs_new: summary["dirs_new"].as_u64().unwrap_or(0) as u32,
            dirs_changed: summary["dirs_changed"].as_u64().unwrap_or(0) as u32,
            dirs_unmodified: summary["dirs_unmodified"].as_u64().unwrap_or(0) as u32,
            data_blobs: summary["data_blobs"].as_u64().unwrap_or(0) as u32,
            tree_blobs: summary["tree_blobs"].as_u64().unwrap_or(0) as u32,
            data_added: summary["data_added"].as_u64().unwrap_or(0),
            duration_secs: summary["duration"].as_f64().unwrap_or(0.0),
            partial: false,
            errors_count: 0,
        })
    }

    /// Caps embedded hook stderr so a chatty hook (e.g. a DB dump command
    /// that logs its whole progress) can't produce an unbounded error
    /// message that then propagates through logs and Telegram.
    const MAX_HOOK_STDERR_LEN: usize = 4096;

    fn truncate_hook_stderr(text: &str) -> String {
        if text.len() <= Self::MAX_HOOK_STDERR_LEN {
            return text.to_string();
        }
        let suffix = "... (truncated)";
        let mut end = Self::MAX_HOOK_STDERR_LEN.saturating_sub(suffix.len());
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}{}", &text[..end], suffix)
    }

    fn execute_hooks(hooks: &[Hook], hook_type: &str) -> Result<(), AppError> {
        for hook in hooks {
            match hook {
                Hook::Command {
                    command,
                    args,
                    continue_on_error,
                } => {
                    info!(hook = hook_type, cmd = command, "Executing hook command");
                    let output = Command::new(command).args(args).output().map_err(|e| {
                        AppError::Other(format!("Failed to execute {} hook: {}", hook_type, e))
                    })?;

                    if !output.status.success() {
                        let stderr =
                            Self::truncate_hook_stderr(&String::from_utf8_lossy(&output.stderr));
                        if *continue_on_error {
                            warn!(
                                hook = hook_type,
                                cmd = command,
                                exit_code = output.status.code(),
                                stderr = %stderr,
                                "Hook command failed, continuing (continue_on_error=true)"
                            );
                        } else {
                            return Err(AppError::Other(format!(
                                "{} hook '{}' failed (exit {:?}): {}",
                                hook_type,
                                command,
                                output.status.code(),
                                stderr
                            )));
                        }
                    }
                }
                Hook::Wait { seconds } => {
                    info!(hook = hook_type, seconds = seconds, "Waiting");
                    std::thread::sleep(std::time::Duration::from_secs(*seconds));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Job, Repository as RepoConfig};
    use crate::secrets::Secrets;

    #[test]
    fn test_truncate_hook_stderr_short_text_unchanged() {
        let text = "a normal error message";
        assert_eq!(Backup::truncate_hook_stderr(text), text);
    }

    #[test]
    fn test_truncate_hook_stderr_over_limit_is_truncated() {
        let text = "a".repeat(Backup::MAX_HOOK_STDERR_LEN + 500);
        let result = Backup::truncate_hook_stderr(&text);
        assert!(result.len() <= Backup::MAX_HOOK_STDERR_LEN);
        assert!(result.ends_with("(truncated)"));
    }

    #[test]
    fn test_truncate_hook_stderr_respects_char_boundaries() {
        let text = "é".repeat(Backup::MAX_HOOK_STDERR_LEN);
        let result = Backup::truncate_hook_stderr(&text);
        assert!(result.len() <= Backup::MAX_HOOK_STDERR_LEN);
    }

    fn test_config() -> ResolvedConfig {
        let mut repositories = std::collections::BTreeMap::new();
        repositories.insert(
            "test".to_string(),
            RepoConfig {
                repo: "/tmp/test-repo".to_string(),
                password_key: "test-password".to_string(),
                log_cli_output: None,
            },
        );

        let mut jobs = std::collections::BTreeMap::new();
        jobs.insert(
            "test-job".to_string(),
            Job {
                repository: "test".to_string(),
                paths: vec!["/tmp".into()],

                exclude_file: None,
                exclude_patterns: None,
                schedule: None,
                retention: None,
                notifications: Default::default(),
                pre_backup: vec![],
                post_backup: vec![],
            },
        );

        let mut secrets_values = std::collections::HashMap::new();
        secrets_values.insert("test-password".to_string(), "test-secret".to_string());

        let config = Config { repositories, jobs };

        let secrets = Secrets {
            values: secrets_values,
            telegram: None,
        };

        ResolvedConfig { config, secrets }
    }

    #[test]
    fn test_job_lookup() {
        let resolved = test_config();
        let job = resolved.config.get_job("test-job");
        assert!(job.is_some());
        assert_eq!(job.unwrap().repository, "test");
    }

    #[test]
    fn test_missing_job() {
        let resolved = test_config();
        let result = Backup::run(&resolved, "nonexistent", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_dry_run_skips_pre_backup_hooks() {
        // A pre-backup hook pointing at a command that doesn't exist would
        // normally abort the backup before it ever reaches restic (hook
        // failures are fatal by default). If dry-run correctly skips
        // pre-backup hooks, Backup::run should instead fail later, trying
        // to invoke restic itself (which also isn't present in this test
        // environment) - so the error must NOT mention the hook.
        let mut resolved = test_config();
        resolved.config.jobs.get_mut("test-job").unwrap().pre_backup = vec![Hook::Command {
            command: "definitely-not-a-real-command-98765".to_string(),
            args: vec![],
            continue_on_error: false,
        }];

        let result = Backup::run(&resolved, "test-job", true);
        assert!(result.is_err());
        let message = result.unwrap_err().to_string();
        // Broader than the positive-side assertion's "pre-backup hook" on
        // purpose: any hook-related wording at all here would mean the
        // hook ran, regardless of how execute_hooks' error text evolves.
        assert!(
            !message.contains("hook"),
            "dry-run should skip pre-backup hooks entirely, got: {message}"
        );
    }

    #[test]
    fn test_non_dry_run_runs_pre_backup_hooks_and_propagates_failure() {
        let mut resolved = test_config();
        resolved.config.jobs.get_mut("test-job").unwrap().pre_backup = vec![Hook::Command {
            command: "definitely-not-a-real-command-98765".to_string(),
            args: vec![],
            continue_on_error: false,
        }];

        let result = Backup::run(&resolved, "test-job", false);
        assert!(result.is_err());
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("pre-backup hook"),
            "non-dry-run should still run and fail on the pre-backup hook, got: {message}"
        );
    }

    #[test]
    fn test_dry_run_skips_pre_backup_wait_hook() {
        // A Wait hook has no failure mode to assert on, so instead assert
        // it doesn't actually sleep: a hook that would sleep far longer
        // than any reasonable test timeout must not run in dry-run mode.
        // 30s is generous enough to avoid flaking on a loaded CI runner,
        // while still being 120x tighter than the 3600s being proved-skipped.
        let mut resolved = test_config();
        resolved.config.jobs.get_mut("test-job").unwrap().pre_backup =
            vec![Hook::Wait { seconds: 3600 }];

        let start = std::time::Instant::now();
        let result = Backup::run(&resolved, "test-job", true);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(30),
            "dry-run should skip pre-backup Wait hooks entirely, not sleep"
        );
        // Confirms the run actually progressed past the (skipped) hook to
        // attempt a restic invocation, rather than short-circuiting on Ok
        // before ever reaching that point.
        assert!(result.is_err());
    }

    #[test]
    fn test_hooks_wait() {
        let _config = test_config();
        let hooks = vec![Hook::Wait { seconds: 0 }];
        let result = Backup::execute_hooks(&hooks, "test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_hooks_command_not_found() {
        let _config = test_config();
        let hooks = vec![Hook::Command {
            command: "nonexistent-command-12345".to_string(),
            args: vec![],
            continue_on_error: false,
        }];
        let result = Backup::execute_hooks(&hooks, "test");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_backup_output_valid() {
        let json = r#"{"summary":{"files_new":1,"files_changed":2,"files_unmodified":3,"dirs_new":1,"dirs_changed":0,"dirs_unmodified":2,"data_blobs":5,"tree_blobs":3,"data_added":1024,"duration":1.5},"snapshot_id":"abc123"}"#;
        let result = Backup::parse_backup_output(json);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.files_new, 1);
        assert_eq!(r.files_changed, 2);
        assert_eq!(r.files_unmodified, 3);
        assert_eq!(r.data_added, 1024);
        assert_eq!(r.snapshot_id, Some("abc123".to_string()));
    }

    #[test]
    fn test_parse_backup_output_no_summary() {
        let result = Backup::parse_backup_output("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_backup_output_empty() {
        let result = Backup::parse_backup_output("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_backup_output_missing_fields() {
        let json = r#"{"summary":{}}"#;
        let result = Backup::parse_backup_output(json);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.files_new, 0);
        assert_eq!(r.data_added, 0);
        assert_eq!(r.snapshot_id, None);
    }

    #[test]
    fn test_parse_backup_output_partial_defaults_false() {
        // parse_backup_output has no notion of the process exit code, so
        // `partial` must always come back false here - execute_backup is
        // responsible for setting it based on the exit status afterward.
        let json = r#"{"summary":{"files_new":1},"snapshot_id":"abc123"}"#;
        let result = Backup::parse_backup_output(json).unwrap();
        assert!(!result.partial);
    }

    #[test]
    fn test_parse_backup_output_errors_count_defaults_zero() {
        // Same reasoning as partial: errors_count comes from "status"
        // messages tracked during the run, not the final summary, so
        // parse_backup_output alone must always return 0.
        let json = r#"{"summary":{"files_new":1},"snapshot_id":"abc123"}"#;
        let result = Backup::parse_backup_output(json).unwrap();
        assert_eq!(result.errors_count, 0);
    }

    #[test]
    fn test_hooks_multiple_wait() {
        let hooks = vec![Hook::Wait { seconds: 0 }, Hook::Wait { seconds: 0 }];
        let result = Backup::execute_hooks(&hooks, "test");
        assert!(result.is_ok());
    }

    #[cfg(windows)]
    fn failing_command() -> (String, Vec<String>) {
        (
            "cmd".to_string(),
            vec!["/C".to_string(), "exit 1".to_string()],
        )
    }

    #[cfg(not(windows))]
    fn failing_command() -> (String, Vec<String>) {
        ("false".to_string(), vec![])
    }

    #[test]
    fn test_hooks_command_failure_aborts_by_default() {
        let (command, args) = failing_command();
        let hooks = vec![Hook::Command {
            command,
            args,
            continue_on_error: false,
        }];
        let result = Backup::execute_hooks(&hooks, "test");
        assert!(result.is_err());
    }

    #[test]
    fn test_hooks_command_failure_continues_when_opted_in() {
        let (command, args) = failing_command();
        let hooks = vec![Hook::Command {
            command,
            args,
            continue_on_error: true,
        }];
        let result = Backup::execute_hooks(&hooks, "test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_missing_repository() {
        let mut repositories = std::collections::BTreeMap::new();
        repositories.insert(
            "test".to_string(),
            RepoConfig {
                repo: "/tmp/test-repo".to_string(),
                password_key: "test-password".to_string(),
                log_cli_output: None,
            },
        );

        let mut jobs = std::collections::BTreeMap::new();
        jobs.insert(
            "test-job".to_string(),
            Job {
                repository: "test".to_string(),
                paths: vec!["/tmp".into()],

                exclude_file: None,
                exclude_patterns: None,
                schedule: None,
                retention: None,
                notifications: Default::default(),
                pre_backup: vec![],
                post_backup: vec![],
            },
        );

        let config = ResolvedConfig {
            config: Config { repositories, jobs },
            secrets: Secrets {
                values: std::collections::HashMap::new(),
                telegram: None,
            },
        };

        let result = Backup::run(&config, "test-job", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_join_stderr_thread_returns_text() {
        let handle = std::thread::spawn(|| "warning: permission denied\n".to_string());
        assert_eq!(
            Backup::join_stderr_thread(handle),
            Some("warning: permission denied\n".to_string())
        );
    }

    #[test]
    fn test_join_stderr_thread_empty_returns_none() {
        let handle = std::thread::spawn(String::new);
        assert_eq!(Backup::join_stderr_thread(handle), None);
    }

    #[test]
    fn test_join_stderr_thread_panic_returns_none() {
        let handle = std::thread::spawn(|| -> String { panic!("simulated reader panic") });
        assert_eq!(Backup::join_stderr_thread(handle), None);
    }

    #[test]
    fn test_missing_password() {
        let repositories = std::collections::BTreeMap::new();
        let mut jobs = std::collections::BTreeMap::new();
        jobs.insert(
            "test-job".to_string(),
            Job {
                repository: "test".to_string(),
                paths: vec!["/tmp".into()],

                exclude_file: None,
                exclude_patterns: None,
                schedule: None,
                retention: None,
                notifications: Default::default(),
                pre_backup: vec![],
                post_backup: vec![],
            },
        );

        let config = ResolvedConfig {
            config: Config { repositories, jobs },
            secrets: Secrets {
                values: std::collections::HashMap::new(),
                telegram: None,
            },
        };

        let result = Backup::run(&config, "test-job", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_path_to_arg_with_utf8_path() {
        // Test normal UTF-8 path
        let path = std::path::PathBuf::from("/normal/path");
        let result = path_to_arg(&path);
        assert_eq!(result, "/normal/path");
        assert_ne!(result, ".");
    }

    #[test]
    fn test_path_to_arg_with_non_utf8_path() {
        // Regression test for #54: to_str().unwrap_or() would panic or return "."
        // on non-UTF8 paths; path_to_arg uses to_string_lossy() which handles them safely.
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let non_utf8_path =
                std::path::PathBuf::from(std::ffi::OsStr::from_bytes(b"test/\xFF\xFE/file"));
            let result = path_to_arg(&non_utf8_path);
            // Should not be "." and should not be empty
            assert_ne!(result, ".");
            assert!(!result.is_empty());
            // Should contain the valid UTF-8 prefix
            assert!(result.contains("test/"));
        }
        #[cfg(not(unix))]
        {
            // On non-Unix systems, we can still test that the function works
            // with a normal path (the non-UTF8 case is harder to test on Windows)
            let path = std::path::PathBuf::from("test/path");
            let result = path_to_arg(&path);
            assert_ne!(result, ".");
            assert!(!result.is_empty());
            assert!(result.contains("test"));
        }
    }

    #[test]
    fn test_path_to_arg_never_returns_dot_for_valid_paths() {
        // Ensure we never return "." which was the bug in the original code
        let test_paths = vec![
            std::path::PathBuf::from("file.txt"),
            std::path::PathBuf::from("/absolute/path"),
            std::path::PathBuf::from("./relative"),
        ];

        for path in test_paths {
            let result = path_to_arg(&path);
            // The bug was returning "." for non-UTF8 paths
            // path_to_arg should never return "." for any valid path
            assert_ne!(result, ".", "path_to_arg returned '.' for path: {:?}", path);
        }
    }
}
