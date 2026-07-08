use crate::errors::ConfigError;
use crate::secrets::Secrets;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub repositories: HashMap<String, Repository>,
    #[serde(default)]
    pub jobs: HashMap<String, Job>,
}

impl Config {
    pub fn load() -> Result<Self, ConfigError> {
        let path = Self::path()?;
        if !path.exists() {
            return Err(ConfigError::NotFound(format!(
                "Config file not found at {}",
                path.display()
            )));
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Config = serde_yaml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    fn path() -> Result<PathBuf, ConfigError> {
        Ok(crate::exclude::config_dir()?.join("config.yaml"))
    }

    fn validate(&self) -> Result<(), ConfigError> {
        for (job_name, job) in &self.jobs {
            if !self.repositories.contains_key(&job.repository) {
                return Err(ConfigError::Invalid(format!(
                    "Job '{}' references non-existent repository '{}'",
                    job_name, job.repository
                )));
            }
        }
        Ok(())
    }

    pub fn get_repository(&self, name: &str) -> Option<&Repository> {
        self.repositories.get(name)
    }

    pub fn get_job(&self, name: &str) -> Option<&Job> {
        self.jobs.get(name)
    }

    pub fn list_jobs(&self) -> Vec<&String> {
        self.jobs.keys().collect()
    }

    pub fn list_repositories(&self) -> Vec<&String> {
        self.repositories.keys().collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub repo: String,
    pub password_key: String,
    #[serde(default)]
    pub log_cli_output: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    #[serde(default)]
    pub keep_daily: Option<u32>,
    #[serde(default)]
    pub keep_weekly: Option<u32>,
    #[serde(default)]
    pub keep_monthly: Option<u32>,
    #[serde(default)]
    pub keep_yearly: Option<u32>,
    #[serde(default)]
    pub keep_hourly: Option<u32>,
    #[serde(default = "default_keep_last")]
    pub keep_last: Option<u32>,
}

fn default_keep_last() -> Option<u32> {
    Some(3)
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            keep_daily: Some(7),
            keep_weekly: Some(4),
            keep_monthly: Some(6),
            keep_yearly: None,
            keep_hourly: None,
            keep_last: Some(3),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationConfig {
    #[serde(default = "default_true")]
    pub on_failure: bool,
    #[serde(default)]
    pub on_success: bool,
}

fn default_true() -> bool {
    true
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            on_failure: true,
            on_success: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Hook {
    Command {
        command: String,
        args: Vec<String>,
        /// If true, a failing command is logged as a warning and the
        /// backup proceeds anyway. Defaults to false: hook failures abort
        /// the backup, since a pre-backup hook failing silently (e.g. a
        /// database dump that didn't actually run) can mean the backup
        /// captures inconsistent data.
        #[serde(default)]
        continue_on_error: bool,
    },
    Wait {
        seconds: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Job {
    pub repository: String,
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub exclude_file: Option<String>,
    #[serde(default)]
    pub exclude_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub retention: Option<RetentionPolicy>,
    #[serde(default)]
    pub notifications: NotificationConfig,
    #[serde(default)]
    pub pre_backup: Vec<Hook>,
    #[serde(default)]
    pub post_backup: Vec<Hook>,
}

#[derive(Clone)]
pub struct ResolvedConfig {
    pub config: Config,
    pub secrets: Secrets,
}

impl ResolvedConfig {
    pub fn load() -> Result<Self, crate::errors::AppError> {
        let config = Config::load().map_err(crate::errors::AppError::Config)?;
        let secrets = Secrets::load_optional()
            .map_err(crate::errors::AppError::Secrets)?
            .unwrap_or_default();
        Ok(Self { config, secrets })
    }

    pub fn get_repo_password(&self, repo_name: &str) -> Option<&str> {
        let repo = self.config.get_repository(repo_name)?;
        self.secrets.get(&repo.password_key)
    }

    /// Resolves a job name to its `Job`, its `Repository`, and the
    /// repository's password, in one call.
    pub fn resolve_job(
        &self,
        job_name: &str,
    ) -> Result<(&Job, &Repository, &str), crate::errors::AppError> {
        let job = self.config.get_job(job_name).ok_or_else(|| {
            crate::errors::AppError::Other(format!("Job '{}' not found", job_name))
        })?;

        let (repo, password) = self.resolve_repo(&job.repository)?;

        Ok((job, repo, password))
    }

    /// Resolves a repository name to its `Repository` and password in one
    /// call.
    pub fn resolve_repo(
        &self,
        repo_name: &str,
    ) -> Result<(&Repository, &str), crate::errors::AppError> {
        let repo = self.config.get_repository(repo_name).ok_or_else(|| {
            crate::errors::AppError::Other(format!("Repository '{}' not found", repo_name))
        })?;

        let password = self.get_repo_password(repo_name).ok_or_else(|| {
            crate::errors::AppError::Other(format!(
                "No password found for repository '{}'",
                repo_name
            ))
        })?;

        Ok((repo, password))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::Secrets;

    fn test_config() -> ResolvedConfig {
        let mut repositories = HashMap::new();
        repositories.insert(
            "test-repo".to_string(),
            Repository {
                repo: "/tmp/test-repo".to_string(),
                password_key: "test-password".to_string(),
                log_cli_output: None,
            },
        );

        let mut jobs = HashMap::new();
        jobs.insert(
            "test-job".to_string(),
            Job {
                repository: "test-repo".to_string(),
                paths: vec!["/tmp".into()],
                ..Default::default()
            },
        );

        let mut secrets_values = HashMap::new();
        secrets_values.insert("test-password".to_string(), "test-secret".to_string());

        ResolvedConfig {
            config: Config { repositories, jobs },
            secrets: Secrets {
                values: secrets_values,
                telegram: None,
            },
        }
    }

    #[test]
    fn test_resolve_job_returns_job_repo_and_password() {
        let config = test_config();
        let (job, repo, password) = config.resolve_job("test-job").unwrap();
        assert_eq!(job.repository, "test-repo");
        assert_eq!(repo.repo, "/tmp/test-repo");
        assert_eq!(password, "test-secret");
    }

    #[test]
    fn test_resolve_job_errors_for_missing_job() {
        let config = test_config();
        assert!(config.resolve_job("nonexistent").is_err());
    }

    #[test]
    fn test_resolve_job_errors_for_missing_repository() {
        let mut config = test_config();
        config.config.jobs.get_mut("test-job").unwrap().repository = "nonexistent-repo".to_string();
        assert!(config.resolve_job("test-job").is_err());
    }

    #[test]
    fn test_resolve_job_errors_for_missing_password() {
        let mut config = test_config();
        config.secrets.values.clear();
        assert!(config.resolve_job("test-job").is_err());
    }

    #[test]
    fn test_resolve_repo_returns_repo_and_password() {
        let config = test_config();
        let (repo, password) = config.resolve_repo("test-repo").unwrap();
        assert_eq!(repo.repo, "/tmp/test-repo");
        assert_eq!(password, "test-secret");
    }

    #[test]
    fn test_resolve_repo_errors_for_missing_repository() {
        let config = test_config();
        assert!(config.resolve_repo("nonexistent").is_err());
    }

    #[test]
    fn test_resolve_repo_errors_for_missing_password() {
        let mut config = test_config();
        config.secrets.values.clear();
        assert!(config.resolve_repo("test-repo").is_err());
    }
}
