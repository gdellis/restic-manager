use crate::backup::Backup;
use crate::config::ResolvedConfig;
use crate::errors::AppError;
use crate::notifications::NotificationManager;
use chrono::Local;
use chrono::Timelike;
use cron::Schedule;
use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{error, info, warn};

pub struct Scheduler {
    config: ResolvedConfig,
    jobs: HashMap<String, JobEntry>,
}

struct JobEntry {
    schedule: Schedule,
}

impl Scheduler {
    pub fn new(config: ResolvedConfig) -> Result<Self, AppError> {
        let mut jobs = HashMap::new();

        for (job_name, job) in config.config.jobs.iter() {
            if let Some(schedule_str) = &job.schedule {
                let schedule = Self::parse_cron(schedule_str)?;
                jobs.insert(job_name.clone(), JobEntry { schedule });
            }
        }

        Ok(Self { config, jobs })
    }

    fn parse_cron(s: &str) -> Result<Schedule, AppError> {
        let with_seconds = if s.split_whitespace().count() == 5 {
            format!("0 {}", s)
        } else {
            s.to_string()
        };

        Schedule::from_str(&with_seconds)
            .map_err(|e| AppError::Other(format!("Invalid cron expression: {}", e)))
    }

    pub fn run(&mut self, shutdown_rx: mpsc::Receiver<()>) -> Result<(), AppError> {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(self.run_async(shutdown_rx))
    }

    async fn run_async(&mut self, mut shutdown_rx: mpsc::Receiver<()>) -> Result<(), AppError> {
        if self.jobs.is_empty() {
            info!("No scheduled jobs configured");
            return Ok(());
        }

        let (tx, mut rx) = mpsc::channel::<String>(100);

        let tick_interval = Duration::from_secs(60);
        let mut ticker = interval(tick_interval);
        let mut shutdown_received = false;

        info!(job_count = self.jobs.len(), "Scheduler started");

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if shutdown_received {
                        continue;
                    }
                    let now = Local::now();
                    for (job_name, entry) in &self.jobs {
                        let next_run = entry.schedule.upcoming(Local).next();
                        if let Some(t) = next_run {
                            if t.hour() == now.hour() && t.minute() == now.minute() {
                                let job_name = job_name.clone();
                                if let Err(e) = tx.send(job_name).await {
                                    warn!("Failed to queue job: {}", e);
                                }
                            }
                        }
                    }
                }

                job_name = rx.recv() => {
                    match job_name {
                        Some(name) => {
                            let config = self.config.clone();
                            let job = config.config.get_job(&name).cloned();
                            tokio::spawn(async move {
                                info!(job = %name, "Starting scheduled backup");

                                let notifier = job
                                    .as_ref()
                                    .map(|j| NotificationManager::new(&config, j.notifications.clone()));

                                match Backup::run(&config, &name) {
                                    Ok(result) => {
                                        info!(job = %name, snapshot = ?result.snapshot_id, "Backup completed");
                                        if let Some(ref n) = notifier {
                                            let _ = n.notify_success(&name, result.snapshot_id.as_deref());
                                        }
                                    }
                                    Err(e) => {
                                        error!(job = %name, error = %e, "Backup failed");
                                        if let Some(ref n) = notifier {
                                            let _ = n.notify_failure(&name, &e.to_string());
                                        }
                                    }
                                }
                            });
                        }
                        None => break,
                    }
                }

                _ = shutdown_rx.recv() => {
                    if !shutdown_received {
                        info!("Shutdown signal received, finishing current jobs");
                        shutdown_received = true;
                    }
                }
            }
        }

        info!("Scheduler stopped");
        Ok(())
    }

    pub fn list_scheduled_jobs(&self) -> Vec<(&str, String)> {
        self.jobs
            .iter()
            .map(|(name, entry)| {
                let next = entry
                    .schedule
                    .upcoming(Local)
                    .next()
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_default();
                (name.as_str(), next)
            })
            .collect()
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
                schedule: Some("0 2 * * *".to_string()),
                retention: None,
                notifications: Default::default(),
                pre_backup: vec![],
                post_backup: vec![],
            },
        );

        let mut secrets_values = std::collections::HashMap::new();
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
    fn test_parse_cron_5_fields() {
        let scheduler = Scheduler::new(test_config()).unwrap();
        let jobs = scheduler.list_scheduled_jobs();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].0, "test-job");
    }

    #[test]
    fn test_parse_cron_6_fields() {
        let mut config = test_config();
        config.config.jobs.get_mut("test-job").unwrap().schedule = Some("0 0 * * * *".to_string());

        let scheduler = Scheduler::new(config).unwrap();
        let jobs = scheduler.list_scheduled_jobs();
        assert_eq!(jobs.len(), 1);
    }

    #[test]
    fn test_parse_cron_invalid() {
        let result = Scheduler::parse_cron("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_no_scheduled_jobs() {
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

        let secrets_values = std::collections::HashMap::new();
        let config = ResolvedConfig {
            config: Config { repositories, jobs },
            secrets: Secrets {
                values: secrets_values,
                telegram: None,
            },
        };

        let scheduler = Scheduler::new(config).unwrap();
        let jobs = scheduler.list_scheduled_jobs();
        assert!(jobs.is_empty());
    }
}
