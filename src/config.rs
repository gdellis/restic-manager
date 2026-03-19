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
        let config_dir = dirs::config_dir()
            .ok_or_else(|| ConfigError::NotFound("Cannot find config directory".into()))?;
        Ok(config_dir.join("restic-manager").join("config.yaml"))
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
    Command { command: String, args: Vec<String> },
    Wait { seconds: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Job {
    pub repository: String,
    pub paths: Vec<PathBuf>,
    #[serde(default)]
    pub exclude: Vec<String>,
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
}
