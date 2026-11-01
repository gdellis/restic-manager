use crate::config::ResolvedConfig;
use crate::errors::{AppError, ResticError};
use crate::restic_child;
use std::process::Command;
use tracing::info;

pub struct Restore;

impl Restore {
    pub fn restore_latest(
        config: &ResolvedConfig,
        job_name: &str,
        target: &str,
    ) -> Result<String, AppError> {
        let (_job, repo, password) = config.resolve_job(job_name)?;

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
        let (_job, repo, password) = config.resolve_job(job_name)?;

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
        let mut cmd = Command::new("restic");
        cmd.args(["snapshots", "--json", "--latest", "1", "--repo", repo])
            .env("RESTIC_PASSWORD", password);
        let output = restic_child::capture_output(&mut cmd).map_err(ResticError::from_io)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ResticError::CommandFailed(stderr.to_string()).into());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Self::parse_latest_snapshot_id(&stdout)
    }

    fn parse_latest_snapshot_id(output: &str) -> Result<String, AppError> {
        let snapshots: Vec<serde_json::Value> = serde_json::from_str(output.trim())
            .map_err(|e| AppError::Other(format!("Failed to parse snapshots JSON: {}", e)))?;

        snapshots
            .first()
            .and_then(|s| s["id"].as_str())
            .map(String::from)
            .ok_or_else(|| AppError::Other("No snapshots found".into()))
    }

    /// Pure builder for the `restic restore` argv, extracted so tests can
    /// lock the exact shape `restore_snapshot_id` sends to restic. See the
    /// `build_restore_args` tests below for the locked contract and the
    /// planned PR that changes it.
    fn build_restore_args(repo: &str, target: &str, snapshot_id: &str) -> Vec<String> {
        vec![
            "restore".to_string(),
            "--repo".to_string(),
            repo.to_string(),
            "--target".to_string(),
            target.to_string(),
            snapshot_id.to_string(),
        ]
    }

    fn restore_snapshot_id(
        repo: &str,
        password: &str,
        snapshot_id: &str,
        target: &str,
    ) -> Result<(), AppError> {
        // M0's pure build_restore_args() builder + #112's tracked-child
        // invocation, merged behavior-neutrally.
        let args = Self::build_restore_args(repo, target, snapshot_id);
        let mut cmd = Command::new("restic");
        cmd.args(&args).env("RESTIC_PASSWORD", password);
        let output = restic_child::capture_output(&mut cmd).map_err(ResticError::from_io)?;

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
    fn test_parse_latest_snapshot_id_empty() {
        let result = Restore::parse_latest_snapshot_id("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_latest_snapshot_id_invalid_json() {
        let result = Restore::parse_latest_snapshot_id("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_latest_snapshot_id_no_snapshots() {
        let result = Restore::parse_latest_snapshot_id("[]");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_latest_snapshot_id_valid() {
        let json = r#"[{"id":"abc123def456","short_id":"abc123d"}]"#;
        let result = Restore::parse_latest_snapshot_id(json);
        assert_eq!(result.unwrap(), "abc123def456");
    }

    /// CONTRACT LOCK: the exact argv shape `restore_snapshot_id` sends to
    /// restic today. Any change to the restore invocation must update this
    /// test deliberately - that is the point. PR-4.2 hardens this shape
    /// (inserting `--` before the positional so leading-dash values can't
    /// be read as options) and flips this assertion on purpose.
    #[test]
    fn test_build_restore_args_matches_current_invocation() {
        let args = Restore::build_restore_args("/srv/repo", "/restore/here", "abc123");
        assert_eq!(
            args,
            vec![
                "restore",
                "--repo",
                "/srv/repo",
                "--target",
                "/restore/here",
                "abc123"
            ]
        );
    }

    /// CONTRACT LOCK (flips in PR-4.2): today a `target` or `snapshot_id`
    /// beginning with `-` is forwarded to restic unguarded - there is no
    /// `--` separator - so restic could read it as an option. This test
    /// documents that current shape so the hardening in PR-4.2 is a
    /// conscious change to this contract, not an accident.
    #[test]
    fn test_build_restore_args_forwards_leading_dash_values_unguarded() {
        let args = Restore::build_restore_args("/srv/repo", "--delete", "-x");
        assert_eq!(
            args,
            vec![
                "restore",
                "--repo",
                "/srv/repo",
                "--target",
                "--delete",
                "-x"
            ]
        );
        assert!(
            !args.contains(&"--".to_string()),
            "no `--` guard exists yet; PR-4.2 adds one"
        );
    }
}
