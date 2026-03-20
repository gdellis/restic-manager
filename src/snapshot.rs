use crate::config::{ResolvedConfig, RetentionPolicy};
use crate::errors::{AppError, ResticError};
use serde::{Deserialize, Serialize};
use std::process::Command;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: String,
    pub time: String,
    pub hostname: Option<String>,
    pub tags: Vec<String>,
    pub paths: Vec<String>,
    pub short_id: String,
}

#[derive(Debug)]
pub struct SnapshotList {
    pub snapshots: Vec<Snapshot>,
}

pub struct SnapshotManager;

impl SnapshotManager {
    pub fn list(config: &ResolvedConfig, job_name: &str) -> Result<SnapshotList, AppError> {
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

        let output = Command::new("restic")
            .args(["snapshots", "--json", "--repo", &repo.repo])
            .env("RESTIC_PASSWORD", password)
            .output()
            .map_err(|_| ResticError::NotFound)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ResticError::CommandFailed(stderr.to_string()).into());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Self::parse_snapshots(stdout.trim())
    }

    fn parse_snapshots(output: &str) -> Result<SnapshotList, AppError> {
        if output.is_empty() {
            return Ok(SnapshotList { snapshots: vec![] });
        }

        let parsed: Vec<serde_json::Value> = serde_json::from_str(output)
            .map_err(|e| AppError::Other(format!("Failed to parse snapshots JSON: {}", e)))?;

        let snapshots: Vec<Snapshot> = parsed
            .into_iter()
            .map(|v| Snapshot {
                id: v["id"].as_str().unwrap_or("").to_string(),
                time: v["time"].as_str().unwrap_or("").to_string(),
                hostname: v["hostname"].as_str().map(String::from),
                tags: v["tags"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|t| t.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                paths: v["paths"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|p| p.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                short_id: v["short_id"].as_str().unwrap_or("").to_string(),
            })
            .collect();

        Ok(SnapshotList { snapshots })
    }

    pub fn forget(
        config: &ResolvedConfig,
        job_name: &str,
        dry_run: bool,
    ) -> Result<Vec<String>, AppError> {
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

        let retention = job.retention.as_ref().ok_or_else(|| {
            AppError::Other(format!(
                "No retention policy defined for job '{}'",
                job_name
            ))
        })?;

        let mut args = vec![
            "forget".to_string(),
            "--json".to_string(),
            "--repo".to_string(),
            repo.repo.clone(),
        ];

        Self::apply_retention_args(&mut args, retention);

        if dry_run {
            args.push("--dry-run".to_string());
        }

        info!(
            job = job_name,
            dry_run = dry_run,
            "Applying retention policy"
        );

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
        Self::parse_forget_output(stdout.trim())
    }

    fn apply_retention_args(args: &mut Vec<String>, retention: &RetentionPolicy) {
        if let Some(n) = retention.keep_daily {
            args.push("--keep-daily".to_string());
            args.push(n.to_string());
        }
        if let Some(n) = retention.keep_weekly {
            args.push("--keep-weekly".to_string());
            args.push(n.to_string());
        }
        if let Some(n) = retention.keep_monthly {
            args.push("--keep-monthly".to_string());
            args.push(n.to_string());
        }
        if let Some(n) = retention.keep_yearly {
            args.push("--keep-yearly".to_string());
            args.push(n.to_string());
        }
        if let Some(n) = retention.keep_hourly {
            args.push("--keep-hourly".to_string());
            args.push(n.to_string());
        }
        if let Some(n) = retention.keep_last {
            args.push("--keep-last".to_string());
            args.push(n.to_string());
        }
    }

    fn parse_forget_output(output: &str) -> Result<Vec<String>, AppError> {
        if output.is_empty() {
            return Ok(vec![]);
        }

        let lines: Vec<&str> = output.lines().collect();
        let mut removed_ids = Vec::new();

        for line in lines {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(removed) = json["removed"].as_array() {
                    for snap in removed {
                        if let Some(id) = snap["id"].as_str() {
                            removed_ids.push(id.to_string());
                        }
                    }
                }
            }
        }

        Ok(removed_ids)
    }

    pub fn prune(config: &ResolvedConfig, job_name: &str) -> Result<(), AppError> {
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

        info!(job = job_name, "Pruning repository");

        let output = Command::new("restic")
            .args(["prune", "--repo", &repo.repo])
            .env("RESTIC_PASSWORD", password)
            .output()
            .map_err(|_| ResticError::NotFound)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ResticError::CommandFailed(stderr.to_string()).into());
        }

        Ok(())
    }

    pub fn apply_retention(
        config: &ResolvedConfig,
        job_name: &str,
        dry_run: bool,
    ) -> Result<Vec<String>, AppError> {
        let removed = Self::forget(config, job_name, dry_run)?;

        if !dry_run && !removed.is_empty() {
            warn!(
                job = job_name,
                count = removed.len(),
                "Removed snapshots, running prune"
            );
            Self::prune(config, job_name)?;
        }

        Ok(removed)
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
                retention: Some(RetentionPolicy::default()),
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
    fn test_retention_args() {
        let mut args = vec![];
        let retention = RetentionPolicy::default();
        SnapshotManager::apply_retention_args(&mut args, &retention);
        assert!(args.contains(&"--keep-daily".to_string()));
        assert!(args.contains(&"7".to_string()));
        assert!(args.contains(&"--keep-weekly".to_string()));
        assert!(args.contains(&"4".to_string()));
        assert!(args.contains(&"--keep-last".to_string()));
        assert!(args.contains(&"3".to_string()));
    }

    #[test]
    fn test_retention_args_empty() {
        let mut args = vec![];
        let retention = RetentionPolicy {
            keep_daily: None,
            keep_weekly: None,
            keep_monthly: None,
            keep_yearly: None,
            keep_hourly: None,
            keep_last: None,
        };
        SnapshotManager::apply_retention_args(&mut args, &retention);
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_snapshots_empty() {
        let result = SnapshotManager::parse_snapshots("");
        assert!(result.is_ok());
        let list = result.unwrap();
        assert!(list.snapshots.is_empty());
    }

    #[test]
    fn test_parse_forget_output_empty() {
        let result = SnapshotManager::parse_forget_output("");
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_job_not_found() {
        let config = test_config();
        let result = SnapshotManager::list(&config, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_forget_no_retention_policy() {
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

        let config = ResolvedConfig {
            config: Config { repositories, jobs },
            secrets: Secrets {
                values: secrets_values,
                telegram: None,
            },
        };

        let result = SnapshotManager::forget(&config, "test-job", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_snapshots_json() {
        let json = r#"[{"id":"abc123","short_id":"abc123","time":"2024-01-15T10:30:00Z","hostname":"server1","tags":["daily"],"paths":["/home"]}]"#;
        let result = SnapshotManager::parse_snapshots(json);
        assert!(result.is_ok());
        let list = result.unwrap();
        assert_eq!(list.snapshots.len(), 1);
        assert_eq!(list.snapshots[0].id, "abc123");
        assert_eq!(list.snapshots[0].hostname.as_deref(), Some("server1"));
        assert_eq!(list.snapshots[0].tags, vec!["daily"]);
    }

    #[test]
    fn test_parse_forget_output_json() {
        let output = r#"{"removed":[{"id":"snap1","short_id":"abc"}],"kept":3}"#;
        let result = SnapshotManager::parse_forget_output(output);
        assert!(result.is_ok());
        let removed = result.unwrap();
        assert_eq!(removed, vec!["snap1"]);
    }

    #[test]
    fn test_forget_missing_password() {
        let mut repositories = std::collections::HashMap::new();
        repositories.insert(
            "test".to_string(),
            RepoConfig {
                repo: "/tmp/test-repo".to_string(),
                password_key: "nonexistent-key".to_string(),
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
                retention: Some(RetentionPolicy::default()),
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

        let result = SnapshotManager::forget(&config, "test-job", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_prune_missing_job() {
        let config = test_config();
        let result = SnapshotManager::prune(&config, "nonexistent");
        assert!(result.is_err());
    }
}
