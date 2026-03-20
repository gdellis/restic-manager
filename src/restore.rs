use crate::config::ResolvedConfig;
use crate::errors::{AppError, ResticError};
use std::process::Command;
use tracing::info;

pub struct Restore;

impl Restore {
    pub fn restore_latest(
        config: &ResolvedConfig,
        job_name: &str,
        target: &str,
    ) -> Result<String, AppError> {
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

        let snapshot_id = Self::find_latest_snapshot(&repo.repo, password)?;

        Self::restore_snapshot_id(&repo.repo, password, &snapshot_id, target)?;

        info!(
            job = job_name,
            snapshot = snapshot_id,
            target = target,
            "Restore completed"
        );

        Ok(snapshot_id)
    }

    pub fn restore_snapshot(
        config: &ResolvedConfig,
        job_name: &str,
        snapshot_id: &str,
        target: &str,
    ) -> Result<(), AppError> {
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

        Self::restore_snapshot_id(&repo.repo, password, snapshot_id, target)?;

        info!(
            job = job_name,
            snapshot = snapshot_id,
            target = target,
            "Restore completed"
        );

        Ok(())
    }

    fn find_latest_snapshot(repo: &str, password: &str) -> Result<String, AppError> {
        let output = Command::new("restic")
            .args(["snapshots", "--json", "--latest", "1", "--repo", repo])
            .env("RESTIC_PASSWORD", password)
            .output()
            .map_err(|_| ResticError::NotFound)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ResticError::CommandFailed(stderr.to_string()).into());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let snapshots: Vec<serde_json::Value> = serde_json::from_str(stdout.trim())
            .map_err(|e| AppError::Other(format!("Failed to parse snapshots JSON: {}", e)))?;

        snapshots
            .first()
            .and_then(|s| s["id"].as_str())
            .map(String::from)
            .ok_or_else(|| AppError::Other("No snapshots found".into()))
    }

    fn restore_snapshot_id(
        repo: &str,
        password: &str,
        snapshot_id: &str,
        target: &str,
    ) -> Result<(), AppError> {
        let output = Command::new("restic")
            .args(["restore", "--repo", repo, "--target", target, snapshot_id])
            .env("RESTIC_PASSWORD", password)
            .output()
            .map_err(|_| ResticError::NotFound)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ResticError::CommandFailed(stderr.to_string()).into());
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
    fn test_missing_job() {
        let resolved = test_config();
        let result = Restore::restore_latest(&resolved, "nonexistent", "/tmp");
        assert!(result.is_err());
    }

    #[test]
    fn test_restore_snapshot_missing_job() {
        let resolved = test_config();
        let result = Restore::restore_snapshot(&resolved, "nonexistent", "snap123", "/tmp");
        assert!(result.is_err());
    }

    #[test]
    fn test_find_latest_snapshot_empty() {
        let result = Restore::find_latest_snapshot("/tmp/repo", "password");
        assert!(result.is_err());
    }

    #[test]
    fn test_find_latest_snapshot_invalid_json() {
        let result = Restore::find_latest_snapshot("/tmp/repo", "password");
        assert!(result.is_err());
    }
}
