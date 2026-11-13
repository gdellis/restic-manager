use crate::errors::ConfigError;
use crate::secrets::Secrets;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub repositories: BTreeMap<String, Repository>,
    #[serde(default)]
    pub jobs: BTreeMap<String, Job>,
}

fn join_errors(prefix: &str, errors: Vec<String>) -> String {
    format!(
        "{} has the following problems:\n{}",
        prefix,
        errors
            .into_iter()
            .map(|e| format!("- {}", e))
            .collect::<Vec<_>>()
            .join("\n")
    )
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
        let mut all_errors: Vec<String> = Vec::new();

        for (repo_name, repo) in &self.repositories {
            if let Err(e) = repo.validate(repo_name) {
                all_errors.push(e.to_string());
            }
        }

        for (job_name, job) in &self.jobs {
            // Skip the missing-repository check when the repository is empty;
            // Job::validate already reports "empty repository reference" and we
            // don't want to double-report it as a missing repository.
            if !job.repository.trim().is_empty() && !self.repositories.contains_key(&job.repository)
            {
                all_errors.push(format!(
                    "Job '{}' references non-existent repository '{}'",
                    job_name, job.repository
                ));
            }
            if let Err(e) = job.validate(job_name) {
                all_errors.push(e.to_string());
            }
        }

        if all_errors.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::Invalid(join_errors(
                "Configuration",
                all_errors,
            )))
        }
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
#[serde(deny_unknown_fields)]
pub struct Repository {
    pub repo: String,
    pub password_key: String,
    #[serde(default)]
    pub log_cli_output: Option<PathBuf>,
}

impl Repository {
    fn validate(&self, name: &str) -> Result<(), ConfigError> {
        let mut errors: Vec<String> = Vec::new();

        if self.repo.trim().is_empty() {
            errors.push("empty repo path".to_string());
        }
        if self.password_key.trim().is_empty() {
            errors.push("empty password_key".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::Invalid(join_errors(
                &format!("Repository '{}'", name),
                errors,
            )))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(tag = "type", deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

impl Job {
    fn validate(&self, name: &str) -> Result<(), ConfigError> {
        let mut errors: Vec<String> = Vec::new();

        if self.repository.trim().is_empty() {
            errors.push("empty repository reference".to_string());
        }
        if self.paths.is_empty() {
            errors.push("no backup paths".to_string());
        }
        for (i, path) in self.paths.iter().enumerate() {
            if path.to_str().map(|s| s.trim().is_empty()).unwrap_or(false) {
                errors.push(format!("empty backup path at index {}", i + 1));
            }
        }
        if let Some(schedule) = &self.schedule {
            if let Err(e) = Self::validate_cron(schedule, name) {
                errors.push(e.to_string());
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::Invalid(join_errors(
                &format!("Job '{}'", name),
                errors,
            )))
        }
    }

    /// The seconds field of a 6-field schedule, if the schedule has one.
    ///
    /// The cron crate's 6-field form is `sec min hour dom mon dow`, so the
    /// first field is the seconds field. A 5-field schedule is left alone:
    /// `normalize_cron` turns it into a 6-field expression with `0` seconds,
    /// which is exactly what the scheduler can evaluate.
    fn seconds_field(schedule: &str) -> Option<&str> {
        let fields: Vec<&str> = schedule.split_whitespace().collect();
        if fields.len() == 6 {
            Some(fields[0])
        } else {
            None
        }
    }

    /// Whether a cron seconds field can match second 0.
    ///
    /// The seconds field is a *set*, not a literal: `"00"`, `"*"`, `"*/5"`,
    /// `"0-30"` and `"0,15"` all include second 0 and therefore do fire.
    /// Only a field that genuinely excludes 0 makes the schedule unrunnable.
    ///
    /// Rather than re-implementing cron's field grammar, this asks the cron
    /// crate itself: it builds a probe schedule that matches one known
    /// instant and checks whether the seconds field matches at second 0.
    fn seconds_field_can_match_zero(seconds: &str) -> bool {
        use chrono::{Local, TimeZone, Timelike};

        // "sec 0 0 1 1 *" matches Jan 1 at 00:00:00 in any year.
        let probe = format!("{} 0 0 1 1 *", seconds);
        let Ok(schedule) = cron::Schedule::from_str(&probe) else {
            // An unparseable field is reported by the normal schedule
            // validation, so treat it as "not our problem" here.
            return true;
        };

        let Some(instant) = Local
            .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
            .single()
            .and_then(|dt| dt.with_second(0))
        else {
            return true;
        };

        schedule.includes(instant)
    }

    fn validate_cron(schedule: &str, job_name: &str) -> Result<(), ConfigError> {
        let normalized = normalize_cron(schedule);

        // The scheduler evaluates `Local::now().with_second(0)`, so it only
        // ever tests a schedule against second 0 of the minute. A 6-field
        // schedule whose seconds field cannot match 0 therefore parses fine
        // but can never fire, and would do so silently.
        //
        // The cron crate's 6-field form is `sec min hour dom mon dow`, so the
        // FIRST field is the seconds field.
        //
        // The seconds field is matched as a set, not compared literally:
        // "00", "*", "*/5", "0-30" and "0,15" all include second 0 and do
        // fire, so only a field that genuinely excludes 0 is rejected.
        if let Some(seconds) = Self::seconds_field(schedule) {
            if !Self::seconds_field_can_match_zero(seconds) {
                return Err(ConfigError::Invalid(format!(
                    "Job '{}' has an invalid schedule '{}': a 6-field schedule with a \
                     non-zero seconds field ({}) can never fire, because the scheduler \
                     only evaluates schedules at second 0. Use a 5-field schedule, or \
                     set the seconds field to 0.",
                    job_name, schedule, seconds
                )));
            }
        }
        cron::Schedule::from_str(&normalized).map_err(|e| {
            ConfigError::Invalid(format!(
                "Job '{}' has an invalid schedule '{}': {}",
                job_name, schedule, e
            ))
        })?;
        Ok(())
    }
}

/// Normalizes a cron expression to the 6-field format expected by the `cron`
/// crate. Standard 5-field expressions (minute hour dom month dow) are
/// prefixed with `0` seconds so they work out of the box. Expressions that
/// already include seconds (6 fields) are left unchanged.
pub(crate) fn normalize_cron(schedule: &str) -> String {
    if schedule.split_whitespace().count() == 5 {
        format!("0 {}", schedule)
    } else {
        schedule.to_string()
    }
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
        let resolved = Self { config, secrets };
        resolved.validate_secrets()?;
        Ok(resolved)
    }

    fn validate_secrets(&self) -> Result<(), crate::errors::AppError> {
        let mut missing: Vec<String> = Vec::new();

        for (repo_name, repo) in &self.config.repositories {
            if self.secrets.get(&repo.password_key).is_none() {
                missing.push(format!(
                    "Repository '{}' references missing secret key '{}'",
                    repo_name, repo.password_key
                ));
            }
        }

        if missing.is_empty() {
            Ok(())
        } else {
            Err(crate::errors::AppError::Config(ConfigError::Invalid(
                join_errors("Missing secrets", missing),
            )))
        }
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
    use std::collections::HashMap;

    fn test_config() -> ResolvedConfig {
        let mut repositories = BTreeMap::new();
        repositories.insert(
            "test-repo".to_string(),
            Repository {
                repo: "/tmp/test-repo".to_string(),
                password_key: "test-password".to_string(),
                log_cli_output: None,
            },
        );

        let mut jobs = BTreeMap::new();
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

    /// Regression test for #31: a typo'd retention field must fail to
    /// deserialize instead of silently no-oping with the default policy.
    #[test]
    fn test_retention_policy_rejects_unknown_field() {
        let yaml = "keep_dayly: 7\nkeep_weekly: 4\n";
        let result: Result<RetentionPolicy, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_job_rejects_unknown_field() {
        let yaml = r#"
repository: test-repo
paths:
  - /home
shedule: "0 2 * * *"
"#;
        let result: Result<Job, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_rejects_unknown_top_level_field() {
        let yaml = r#"
repositories: {}
jobs: {}
extra_unknown_key: true
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_hook_command_rejects_unknown_field() {
        let yaml = r#"
type: Command
command: /usr/bin/true
args: []
unknown_field: 1
"#;
        let result: Result<Hook, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_repository_validation_rejects_empty_repo_path() {
        let yaml = r#"
repositories:
  bad:
    repo: ""
    password_key: pass
jobs: {}
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        assert!(result.unwrap().validate().is_err());
    }

    #[test]
    fn test_repository_validation_rejects_empty_password_key() {
        let yaml = r#"
repositories:
  bad:
    repo: /tmp/repo
    password_key: ""
jobs: {}
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        assert!(result.unwrap().validate().is_err());
    }

    #[test]
    fn test_job_validation_rejects_empty_repository() {
        let yaml = r#"
repositories:
  test:
    repo: /tmp/repo
    password_key: pass
jobs:
  bad:
    repository: ""
    paths:
      - /home
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        assert!(result.unwrap().validate().is_err());
    }

    #[test]
    fn test_job_validation_rejects_no_paths() {
        let yaml = r#"
repositories:
  test:
    repo: /tmp/repo
    password_key: pass
jobs:
  bad:
    repository: test
    paths: []
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        assert!(result.unwrap().validate().is_err());
    }

    #[test]
    fn test_job_validation_rejects_invalid_cron() {
        let yaml = r#"
repositories:
  test:
    repo: /tmp/repo
    password_key: pass
jobs:
  bad:
    repository: test
    paths:
      - /home
    schedule: "not a cron"
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        assert!(result.unwrap().validate().is_err());
    }

    #[test]
    fn test_job_validation_accepts_valid_5_field_cron() {
        let yaml = r#"
repositories:
  test:
    repo: /tmp/repo
    password_key: pass
jobs:
  good:
    repository: test
    paths:
      - /home
    schedule: "0 2 * * *"
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        assert!(result.unwrap().validate().is_ok());
    }

    #[test]
    fn test_job_validation_accepts_valid_6_field_cron() {
        let yaml = r#"
repositories:
  test:
    repo: /tmp/repo
    password_key: pass
jobs:
  good:
    repository: test
    paths:
      - /home
    schedule: "0 0 2 * * *"
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        assert!(result.unwrap().validate().is_ok());
    }

    #[test]
    fn test_job_validation_rejects_invalid_6_field_cron() {
        let yaml = r#"
repositories:
  test:
    repo: /tmp/repo
    password_key: pass
jobs:
  bad:
    repository: test
    paths:
      - /home
    schedule: "not a cron"
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        assert!(result.unwrap().validate().is_err());
    }

    #[test]
    fn test_resolved_config_validation_rejects_missing_password_secret() {
        let mut repositories = BTreeMap::new();
        repositories.insert(
            "test-repo".to_string(),
            Repository {
                repo: "/tmp/test-repo".to_string(),
                password_key: "missing-secret".to_string(),
                log_cli_output: None,
            },
        );
        let mut jobs = BTreeMap::new();
        jobs.insert(
            "test-job".to_string(),
            Job {
                repository: "test-repo".to_string(),
                paths: vec!["/tmp".into()],
                ..Default::default()
            },
        );
        let resolved = ResolvedConfig {
            config: Config { repositories, jobs },
            secrets: Secrets::default(),
        };
        assert!(resolved.validate_secrets().is_err());
    }

    #[test]
    fn test_repository_reports_multiple_errors() {
        let yaml = r#"
repositories:
  bad:
    repo: ""
    password_key: ""
jobs: {}
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        let err = result.unwrap().validate().unwrap_err().to_string();
        assert!(err.contains("empty repo path"), "{}", err);
        assert!(err.contains("empty password_key"), "{}", err);
    }

    #[test]
    fn test_job_reports_multiple_errors() {
        let yaml = r#"
repositories:
  test:
    repo: /tmp/repo
    password_key: pass
jobs:
  bad:
    repository: ""
    paths: []
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        let err = result.unwrap().validate().unwrap_err().to_string();
        assert!(err.contains("empty repository reference"), "{}", err);
        assert!(err.contains("no backup paths"), "{}", err);
    }

    #[test]
    fn test_six_field_schedule_with_nonzero_seconds_is_rejected() {
        // The scheduler evaluates schedules at second 0 only, so a seconds
        // field that cannot match 0 makes the job silently never run.
        let err = Job::validate_cron("30 0 2 * * *", "j").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("seconds"), "{}", msg);
    }

    #[test]
    fn test_six_field_schedule_with_zero_seconds_is_accepted() {
        assert!(Job::validate_cron("0 0 2 * * *", "j").is_ok());
    }

    #[test]
    fn test_five_field_schedule_is_unaffected() {
        assert!(Job::validate_cron("0 2 * * *", "j").is_ok());
        assert!(Job::validate_cron("*/5 * * * *", "j").is_ok());
    }

    #[test]
    fn test_seconds_fields_that_include_zero_are_accepted() {
        // These are sets, not literals: each one matches second 0, so each
        // one fires and none of them may be rejected.
        for seconds in ["0", "00", "*", "*/5", "0-30", "0,15", "*/1", "0-59/7"] {
            let expr = format!("{} 0 2 * * *", seconds);
            assert!(
                Job::validate_cron(&expr, "j").is_ok(),
                "seconds field {:?} should be accepted",
                seconds
            );
        }
    }

    #[test]
    fn test_seconds_fields_that_exclude_zero_are_rejected() {
        for seconds in ["30", "1-30/5", "15,45", "1-59", "59"] {
            let expr = format!("{} 0 2 * * *", seconds);
            assert!(
                Job::validate_cron(&expr, "j").is_err(),
                "seconds field {:?} should be rejected",
                seconds
            );
        }
    }

    #[test]
    fn test_seven_field_schedule_is_unaffected() {
        // The integration suite uses a 7-field Quartz expression; it has a
        // seconds field of 0 and must keep working.
        assert!(Job::validate_cron("0 0 0 1 1 ? 2099", "j").is_ok());
    }

    #[test]
    fn test_job_reports_cron_and_other_errors() {
        let yaml = r#"
repositories:
  test:
    repo: /tmp/repo
    password_key: pass
jobs:
  bad:
    repository: ""
    paths: []
    schedule: "not a cron"
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        let err = result.unwrap().validate().unwrap_err().to_string();
        assert!(err.contains("empty repository reference"), "{}", err);
        assert!(err.contains("no backup paths"), "{}", err);
        assert!(err.contains("invalid schedule"), "{}", err);
    }

    #[test]
    fn test_job_reports_all_empty_backup_paths() {
        let yaml = r#"
repositories:
  test:
    repo: /tmp/repo
    password_key: pass
jobs:
  bad:
    repository: test
    paths:
      - ""
      - "   "
      - /home
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        let err = result.unwrap().validate().unwrap_err().to_string();
        assert!(err.contains("empty backup path at index 1"), "{}", err);
        assert!(err.contains("empty backup path at index 2"), "{}", err);
        assert!(!err.contains("empty backup path at index 3"), "{}", err);
    }

    #[test]
    fn test_empty_repository_does_not_report_missing_repo() {
        let yaml = r#"
repositories:
  test:
    repo: /tmp/repo
    password_key: pass
jobs:
  bad:
    repository: ""
    paths:
      - /home
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        let err = result.unwrap().validate().unwrap_err().to_string();
        assert!(err.contains("empty repository reference"), "{}", err);
        assert!(
            !err.contains("non-existent repository"),
            "empty repository should not also be reported as missing: {}",
            err
        );
    }

    #[test]
    fn test_config_reports_multiple_repo_and_job_errors() {
        let yaml = r#"
repositories:
  bad-repo-1:
    repo: ""
    password_key: ""
  bad-repo-2:
    repo: /tmp/repo
    password_key: ""
jobs:
  bad-job-1:
    repository: ""
    paths: []
  bad-job-2:
    repository: missing-repo
    paths:
      - /home
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        let err = result.unwrap().validate().unwrap_err().to_string();
        assert!(err.contains("Repository 'bad-repo-1'"), "{}", err);
        assert!(err.contains("Repository 'bad-repo-2'"), "{}", err);
        assert!(err.contains("Job 'bad-job-1'"), "{}", err);
        assert!(err.contains("Job 'bad-job-2'"), "{}", err);
        assert!(err.contains("missing-repo"), "{}", err);
    }

    #[test]
    fn test_config_error_order_is_deterministic() {
        let yaml = r#"
repositories:
  z-repo:
    repo: ""
    password_key: ""
  a-repo:
    repo: ""
    password_key: ""
jobs:
  z-job:
    repository: ""
    paths: []
  a-job:
    repository: ""
    paths: []
"#;
        let result: Result<Config, _> = serde_yaml::from_str(yaml);
        assert!(result.is_ok());
        let err = result.unwrap().validate().unwrap_err().to_string();

        let repo_a = err.find("Repository 'a-repo'").expect("a-repo error");
        let repo_z = err.find("Repository 'z-repo'").expect("z-repo error");
        let job_a = err.find("Job 'a-job'").expect("a-job error");
        let job_z = err.find("Job 'z-job'").expect("z-job error");

        assert!(repo_a < repo_z, "repo errors should be sorted: {}", err);
        assert!(job_a < job_z, "job errors should be sorted: {}", err);
    }

    #[test]
    fn test_resolved_config_reports_all_missing_secrets() {
        let mut repositories = BTreeMap::new();
        repositories.insert(
            "repo-a".to_string(),
            Repository {
                repo: "/tmp/repo-a".to_string(),
                password_key: "missing-a".to_string(),
                log_cli_output: None,
            },
        );
        repositories.insert(
            "repo-b".to_string(),
            Repository {
                repo: "/tmp/repo-b".to_string(),
                password_key: "missing-b".to_string(),
                log_cli_output: None,
            },
        );
        let mut jobs = BTreeMap::new();
        jobs.insert(
            "test-job".to_string(),
            Job {
                repository: "repo-a".to_string(),
                paths: vec!["/tmp".into()],
                ..Default::default()
            },
        );
        let resolved = ResolvedConfig {
            config: Config { repositories, jobs },
            secrets: Secrets::default(),
        };
        let err = resolved.validate_secrets().unwrap_err().to_string();
        assert!(err.contains("Missing secrets"), "{}", err);
        let repo_a = err.find("Repository 'repo-a'").expect("repo-a error");
        let repo_b = err.find("Repository 'repo-b'").expect("repo-b error");
        assert!(
            repo_a < repo_b,
            "missing secret errors should be sorted: {}",
            err
        );
    }

    #[test]
    fn test_list_jobs_returns_sorted_keys() {
        let yaml = r#"
repositories:
  r1:
    repo: /tmp/repo
    password_key: pass
jobs:
  z-job:
    repository: r1
    paths:
      - /tmp
  a-job:
    repository: r1
    paths:
      - /tmp
  m-job:
    repository: r1
    paths:
      - /tmp
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let jobs: Vec<_> = config.list_jobs().into_iter().cloned().collect();
        assert_eq!(jobs, vec!["a-job", "m-job", "z-job"]);
    }

    #[test]
    fn test_list_repositories_returns_sorted_keys() {
        let yaml = r#"
repositories:
  z-repo:
    repo: /tmp/z
    password_key: pass
  a-repo:
    repo: /tmp/a
    password_key: pass
jobs: {}
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let repos: Vec<_> = config.list_repositories().into_iter().cloned().collect();
        assert_eq!(repos, vec!["a-repo", "z-repo"]);
    }
}
