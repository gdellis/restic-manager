use crate::config::ResolvedConfig;
use crate::errors::AppError;

pub struct Repository;

impl Repository {
    pub fn init(config: &ResolvedConfig, repo_name: &str) -> Result<(), AppError> {
        let (repo, password) = config.resolve_repo(repo_name)?;

        crate::restic::run_restic(&["init", "--repo", &repo.repo], password, None)?;

        println!("Repository '{}' initialized successfully", repo_name);
        println!("  Path: {}", repo.repo);
        Ok(())
    }

    pub fn check(config: &ResolvedConfig, repo_name: &str) -> Result<(), AppError> {
        let (repo, password) = config.resolve_repo(repo_name)?;
        let log_file = repo.log_cli_output.as_deref();

        crate::restic::run_restic(&["check", "--repo", &repo.repo], password, log_file)?;

        println!("Repository '{}' check passed", repo_name);
        Ok(())
    }

    pub fn unlock(config: &ResolvedConfig, repo_name: &str) -> Result<(), AppError> {
        let (repo, password) = config.resolve_repo(repo_name)?;
        let log_file = repo.log_cli_output.as_deref();

        crate::restic::run_restic(
            &["unlock", "--repo", &repo.repo, "--remove-all"],
            password,
            log_file,
        )?;

        println!("Repository '{}' unlocked successfully", repo_name);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Repository as RepoConfig};
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
        let mut repositories = std::collections::BTreeMap::new();
        repositories.insert(
            "test".to_string(),
            RepoConfig {
                repo: "/tmp/test-repo".to_string(),
                password_key: "nonexistent-key".to_string(),
                log_cli_output: None,
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

    #[test]
    fn test_check_missing_repository() {
        let resolved = test_config();
        let result = Repository::check(&resolved, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_unlock_missing_repository() {
        let resolved = test_config();
        let result = Repository::unlock(&resolved, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_init_missing_password() {
        let repositories = std::collections::BTreeMap::new();
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
