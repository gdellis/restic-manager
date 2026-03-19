use crate::config::{Hook, ResolvedConfig};
use crate::errors::{AppError, ResticError};
use std::process::Command;
use tracing::{info, warn};

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
    pub fn run(config: &ResolvedConfig, job_name: &str) -> Result<BackupResult, AppError> {
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

        let result = Self::execute_backup(&repo.repo, password, &job.paths, &job.exclude)?;

        Self::execute_hooks(&job.post_backup, "post-backup")?;

        info!(
            job = job_name,
            snapshot = ?result.snapshot_id,
            files_new = result.files_new,
            data_added = result.data_added,
            "Backup completed successfully"
        );

        Ok(result)
    }

    fn execute_backup(
        repo: &str,
        password: &str,
        paths: &[std::path::PathBuf],
        exclude: &[String],
    ) -> Result<BackupResult, AppError> {
        let mut args = vec!["backup", "--json", "--repo", repo];

        for path in paths {
            args.push("--files-from");
            args.push(path.to_str().unwrap_or("."));
        }

        for pattern in exclude {
            args.push("--exclude");
            args.push(pattern);
        }

        let output = Command::new("restic")
            .args(&args)
            .env("RESTIC_PASSWORD", password)
            .output()
            .map_err(|_| ResticError::NotFound)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ResticError::CommandFailed(stderr.to_string()).into());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Self::parse_backup_output(stdout.trim())
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
            },
        );

        let mut jobs = std::collections::HashMap::new();
        jobs.insert(
            "test-job".to_string(),
            Job {
                repository: "test".to_string(),
                paths: vec!["/tmp".into()],
                exclude: vec![],
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
        let result = Backup::run(&resolved, "nonexistent");
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
            },
        );

        let mut jobs = std::collections::HashMap::new();
        jobs.insert(
            "test-job".to_string(),
            Job {
                repository: "nonexistent".to_string(),
                paths: vec!["/tmp".into()],
                exclude: vec![],
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

        let result = Backup::run(&config, "test-job");
        assert!(result.is_err());
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
                exclude: vec![],
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

        let result = Backup::run(&config, "test-job");
        assert!(result.is_err());
    }
}
