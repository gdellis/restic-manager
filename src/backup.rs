use crate::config::{Hook, ResolvedConfig};
use crate::errors::{AppError, ResticError};
use crate::exclude;
use std::process::Command;
use tracing::{debug, info, warn};

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

#[derive(Debug)]
pub struct BackupResult {
    pub job_name: String,
    pub snapshot_id: Option<String>,
    pub files_new: u32,
    pub files_changed: u32,
    pub files_unmodified: u32,
    pub dirs_new: u32,
    pub dirs_changed: u32,
    pub dirs_unmodified: u32,
    pub data_blobs: u32,
    pub tree_blobs: u32,
    pub data_added: u64,
    pub duration_secs: f64,
}

pub struct Backup;

impl Backup {
    pub fn run(
        config: &ResolvedConfig,
        job_name: &str,
        dry_run: bool,
    ) -> Result<BackupResult, AppError> {
        let job = config
            .config
            .get_job(job_name)
            .ok_or_else(|| AppError::Other(format!("Job '{}' not found", job_name)))?;

        let repo = config
            .config
            .get_repository(&job.repository)
            .ok_or_else(|| AppError::Other(format!("Repository '{}' not found", job.repository)))?;

        let password = config.get_repo_password(&job.repository).ok_or_else(|| {
            AppError::Other(format!(
                "No password found for repository '{}'",
                job.repository
            ))
        })?;

        Self::execute_hooks(&job.pre_backup, "pre-backup")?;

        if dry_run {
            info!(
                job = job_name,
                "DRY-RUN MODE: No data will be written to repository"
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
                "Dry-run completed, skipping post-backup hooks"
            );
        }

        info!(
            job = job_name,
            snapshot = ?result.snapshot_id,
            files_new = result.files_new,
            data_added = result.data_added,
            "Backup completed successfully"
        );

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
            args.push(path.to_str().unwrap_or(".").to_string());
        }

        debug!("Executing: restic {}", args.join(" "));

        if dry_run {
            debug!("DRY-RUN flag confirmed in command args");
        }

        let mut cmd = Command::new("restic");
        cmd.args(&args).env("RESTIC_PASSWORD", password);

        if let Some(log_path) = log_file {
            if let Some(parent) = log_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            cmd.env("DEBUG_LOG", log_path);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|_| ResticError::NotFound)?;

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
                let _ = reader.read_to_string(&mut text);
                text
            })
        });

        let mut lines: Vec<String> = Vec::new();

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
                                if total_files > 0 {
                                    let progress = format_progress(
                                        files_done,
                                        total_files,
                                        bytes_done,
                                        total_bytes,
                                        errors,
                                    );
                                    eprint!("\r{}", progress);
                                }
                            }
                            "verbose_status" => {
                                if let Some(item) = json.get("item").and_then(|v| v.as_str()) {
                                    let action =
                                        json.get("action").and_then(|v| v.as_str()).unwrap_or("");
                                    eprint!("\r{}: {}", action, item);
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

        let status = child.wait().map_err(|_| ResticError::NotFound)?;

        let stderr_text = stderr_handle.and_then(Self::join_stderr_thread);

        if let Some(text) = stderr_text.as_deref() {
            warn!(stderr = text, "restic wrote to stderr");
        }

        if !status.success() {
            return Err(ResticError::CommandFailed(stderr_text.unwrap_or_default()).into());
        }

        let output = lines.join("\n");
        let result = Self::parse_backup_output(&output)?;

        info!(
            snapshot = ?result.snapshot_id,
            files_new = result.files_new,
            files_changed = result.files_changed,
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
        })
    }

    fn execute_hooks(hooks: &[Hook], hook_type: &str) -> Result<(), AppError> {
        for hook in hooks {
            match hook {
                Hook::Command { command, args } => {
                    info!(hook = hook_type, cmd = command, "Executing hook command");
                    let output = Command::new(command).args(args).output().map_err(|e| {
                        AppError::Other(format!("Failed to execute {} hook: {}", hook_type, e))
                    })?;

                    if !output.status.success() {
                        let _stderr = String::from_utf8_lossy(&output.stderr);
                        warn!(
                            hook = hook_type,
                            cmd = command,
                            exit_code = output.status.code(),
                            "Hook command failed"
                        );
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

    fn test_config() -> ResolvedConfig {
        let mut repositories = std::collections::HashMap::new();
        repositories.insert(
            "test".to_string(),
            RepoConfig {
                repo: "/tmp/test-repo".to_string(),
                password_key: "test-password".to_string(),
                log_cli_output: None,
            },
        );

        let mut jobs = std::collections::HashMap::new();
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
    fn test_hooks_multiple_wait() {
        let hooks = vec![Hook::Wait { seconds: 0 }, Hook::Wait { seconds: 0 }];
        let result = Backup::execute_hooks(&hooks, "test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_hooks_command_failure_silent() {
        let hooks = vec![Hook::Command {
            command: "echo".to_string(),
            args: vec!["fail".to_string()],
        }];
        let result = Backup::execute_hooks(&hooks, "test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_missing_repository() {
        let mut repositories = std::collections::HashMap::new();
        repositories.insert(
            "test".to_string(),
            RepoConfig {
                repo: "/tmp/test-repo".to_string(),
                password_key: "test-password".to_string(),
                log_cli_output: None,
            },
        );

        let mut jobs = std::collections::HashMap::new();
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
        let repositories = std::collections::HashMap::new();
        let mut jobs = std::collections::HashMap::new();
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
}
