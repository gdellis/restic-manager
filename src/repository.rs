use crate::config::ResolvedConfig;
use crate::errors::{AppError, ResticError};
use std::process::Command;

pub struct Repository;

impl Repository {
    pub fn init(config: &ResolvedConfig, repo_name: &str) -> Result<(), AppError> {
        let repo = config
            .config
            .get_repository(repo_name)
            .ok_or_else(|| AppError::Other(format!("Repository '{}' not found", repo_name)))?;

        let password = config.get_repo_password(repo_name).ok_or_else(|| {
            AppError::Other(format!("No password found for repository '{}'", repo_name))
        })?;

        let output = Command::new("restic")
            .args(["init", "--repo", &repo.repo])
            .env("RESTIC_PASSWORD", password)
            .output()
            .map_err(|_| ResticError::NotFound)?;

        if output.status.success() {
            println!("Repository '{}' initialized successfully", repo_name);
            println!("  Path: {}", repo.repo);
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(ResticError::CommandFailed(stderr.to_string()).into())
        }
    }

    pub fn check(config: &ResolvedConfig, repo_name: &str) -> Result<(), AppError> {
        let repo = config
            .config
            .get_repository(repo_name)
            .ok_or_else(|| AppError::Other(format!("Repository '{}' not found", repo_name)))?;

        let password = config.get_repo_password(repo_name).ok_or_else(|| {
            AppError::Other(format!("No password found for repository '{}'", repo_name))
        })?;

        let output = Command::new("restic")
            .args(["check", "--repo", &repo.repo])
            .env("RESTIC_PASSWORD", password)
            .output()
            .map_err(|_| ResticError::NotFound)?;

        if output.status.success() {
            println!("Repository '{}' check passed", repo_name);
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(ResticError::CommandFailed(stderr.to_string()).into())
        }
    }

    pub fn unlock(config: &ResolvedConfig, repo_name: &str) -> Result<(), AppError> {
        let repo = config
            .config
            .get_repository(repo_name)
            .ok_or_else(|| AppError::Other(format!("Repository '{}' not found", repo_name)))?;

        let password = config.get_repo_password(repo_name).ok_or_else(|| {
            AppError::Other(format!("No password found for repository '{}'", repo_name))
        })?;

        let output = Command::new("restic")
            .args(["unlock", "--repo", &repo.repo, "--remove-all"])
            .env("RESTIC_PASSWORD", password)
            .output()
            .map_err(|_| ResticError::NotFound)?;

        if output.status.success() {
            println!("Repository '{}' unlocked successfully", repo_name);
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(ResticError::CommandFailed(stderr.to_string()).into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Repository as RepoConfig};
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

        let mut secrets_values = std::collections::HashMap::new();
        secrets_values.insert("test-password".to_string(), "test-secret".to_string());

        let config = Config {
            repositories,
            ..Default::default()
        };

        let secrets = Secrets {
            values: secrets_values,
            telegram: None,
        };

        ResolvedConfig { config, secrets }
    }

    #[test]
    fn test_repository_lookup() {
        let resolved = test_config();
        let repo = resolved.config.get_repository("test");
        assert!(repo.is_some());
        assert_eq!(repo.unwrap().repo, "/tmp/test-repo");
    }

    #[test]
    fn test_password_resolution() {
        let resolved = test_config();
        let password = resolved.get_repo_password("test");
        assert_eq!(password, Some("test-secret"));
    }

    #[test]
    fn test_missing_repository() {
        let resolved = test_config();
        let result = Repository::init(&resolved, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_password_key() {
        let mut repositories = std::collections::HashMap::new();
        repositories.insert(
            "test".to_string(),
            RepoConfig {
                repo: "/tmp/test-repo".to_string(),
                password_key: "nonexistent-key".to_string(),
            },
        );

        let config = Config {
            repositories,
            ..Default::default()
        };

        let secrets = Secrets {
            values: std::collections::HashMap::new(),
            telegram: None,
        };

        let resolved = ResolvedConfig { config, secrets };

        let result = Repository::init(&resolved, "test");
        assert!(result.is_err());
    }
}
