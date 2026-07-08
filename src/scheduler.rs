use crate::backup::Backup;
use crate::config::ResolvedConfig;
use crate::errors::AppError;
use crate::notifications::NotificationManager;
use chrono::DateTime;
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

    /// Atomically checks whether `schedule` is due at `now` and, if so,
    /// records `job_name` as triggered for this exact (minute-truncated)
    /// instant so a burst of ticks landing in the same minute can't
    /// double-dispatch the job. The check and the record happen together
    /// so there's no separate step for a caller to forget.
    fn mark_if_due(
        schedule: &Schedule,
        now: DateTime<Local>,
        last_triggered: &mut HashMap<String, DateTime<Local>>,
        job_name: &str,
    ) -> bool {
        if !schedule.includes(now) {
            return false;
        }
        if last_triggered.get(job_name) == Some(&now) {
            return false;
        }
        last_triggered.insert(job_name.to_string(), now);
        true
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
        // tokio::time::interval fires its first tick immediately rather than
        // after one full interval; consume it here so a job scheduled for
        // "every minute" doesn't get a spurious extra run right at startup.
        ticker.tick().await;
        let mut shutdown_received = false;
        let mut last_triggered: HashMap<String, DateTime<Local>> = HashMap::new();

        info!(job_count = self.jobs.len(), "Scheduler started");

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if shutdown_received {
                        continue;
                    }
                    // with_second(0)/with_nanosecond(0) only return None when the
                    // *argument* is out of range (sec >= 60, nanos >= 2_000_000_000),
                    // never for the literal 0 passed here, so this branch is
                    // unreachable in practice. Skip the tick rather than panic
                    // the daemon if that ever changes.
                    let Some(now) = Local::now()
                        .with_second(0)
                        .and_then(|t| t.with_nanosecond(0))
                    else {
                        error!("Failed to truncate current time to the minute; skipping tick");
                        continue;
                    };
                    for (job_name, entry) in &self.jobs {
                        if Self::mark_if_due(&entry.schedule, now, &mut last_triggered, job_name) {
                            let job_name = job_name.clone();
                            if let Err(e) = tx.send(job_name).await {
                                warn!("Failed to queue job: {}", e);
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

                                let blocking_name = name.clone();
                                let join_result = tokio::task::spawn_blocking(move || {
                                    Backup::run(&config, &blocking_name, false)
                                })
                                .await;

                                match join_result {
                                    Ok(Ok(result)) => {
                                        info!(job = %name, snapshot = ?result.snapshot_id, "Backup completed");
                                        if let Some(ref n) = notifier {
                                            let _ = n.notify_success(&name, result.snapshot_id.as_deref());
                                        }
                                    }
                                    Ok(Err(e)) => {
                                        error!(job = %name, error = %e, "Backup failed");
                                        if let Some(ref n) = notifier {
                                            let _ = n.notify_failure(&name, &e.to_string());
                                        }
                                    }
                                    Err(join_err) => {
                                        error!(job = %name, error = %join_err, "Backup task panicked");
                                        if let Some(ref n) = notifier {
                                            let _ = n.notify_failure(
                                                &name,
                                                &format!("Backup task panicked: {}", join_err),
                                            );
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
    use chrono::TimeZone;

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
    fn test_mark_if_due_fires_on_matching_day() {
        // The `cron` crate uses Quartz-style day-of-week numbering
        // (1=Sunday .. 7=Saturday), so "1" here means Sundays, not
        // Mondays. 2026-01-04 is a Sunday.
        let schedule = Scheduler::parse_cron("0 2 * * 1").unwrap(); // Sundays 02:00
        let sunday_2am = Local.with_ymd_and_hms(2026, 1, 4, 2, 0, 0).unwrap();
        let mut last_triggered = HashMap::new();
        assert!(Scheduler::mark_if_due(
            &schedule,
            sunday_2am,
            &mut last_triggered,
            "job"
        ));
    }

    #[test]
    fn test_mark_if_due_skips_wrong_day() {
        // This is the exact bug in #8: old code only compared hour/minute
        // and would have fired here too. 2026-01-05 is a Monday, not a
        // Sunday, so a Sundays-only schedule must not fire.
        let schedule = Scheduler::parse_cron("0 2 * * 1").unwrap();
        let monday_2am = Local.with_ymd_and_hms(2026, 1, 5, 2, 0, 0).unwrap();
        let mut last_triggered = HashMap::new();
        assert!(!Scheduler::mark_if_due(
            &schedule,
            monday_2am,
            &mut last_triggered,
            "job"
        ));
    }

    #[test]
    fn test_mark_if_due_skips_duplicate_minute() {
        let schedule = Scheduler::parse_cron("* * * * *").unwrap();
        let now = Local.with_ymd_and_hms(2026, 1, 5, 2, 0, 0).unwrap();
        let mut last_triggered = HashMap::new();
        assert!(Scheduler::mark_if_due(
            &schedule,
            now,
            &mut last_triggered,
            "job"
        ));
        assert!(!Scheduler::mark_if_due(
            &schedule,
            now,
            &mut last_triggered,
            "job"
        ));
    }

    #[test]
    fn test_mark_if_due_fires_again_next_minute() {
        let schedule = Scheduler::parse_cron("* * * * *").unwrap();
        let minute_one = Local.with_ymd_and_hms(2026, 1, 5, 2, 0, 0).unwrap();
        let minute_two = Local.with_ymd_and_hms(2026, 1, 5, 2, 1, 0).unwrap();
        let mut last_triggered = HashMap::new();
        assert!(Scheduler::mark_if_due(
            &schedule,
            minute_one,
            &mut last_triggered,
            "job"
        ));
        assert!(Scheduler::mark_if_due(
            &schedule,
            minute_two,
            &mut last_triggered,
            "job"
        ));
    }

    #[test]
    fn test_mark_if_due_tracks_jobs_independently() {
        let schedule = Scheduler::parse_cron("* * * * *").unwrap();
        let now = Local.with_ymd_and_hms(2026, 1, 5, 2, 0, 0).unwrap();
        let mut last_triggered = HashMap::new();
        assert!(Scheduler::mark_if_due(
            &schedule,
            now,
            &mut last_triggered,
            "job-a"
        ));
        // A different job name must not be shadowed by job-a's entry.
        assert!(Scheduler::mark_if_due(
            &schedule,
            now,
            &mut last_triggered,
            "job-b"
        ));
    }

    #[tokio::test]
    async fn test_spawn_blocking_panic_yields_join_error() {
        let handle: tokio::task::JoinHandle<()> =
            tokio::task::spawn_blocking(|| panic!("simulated backup panic"));
        let result = handle.await;
        assert!(result.is_err());
        assert!(result.unwrap_err().is_panic());
    }

    #[tokio::test]
    async fn test_backup_run_inside_spawn_blocking_returns_err_for_missing_job() {
        let config = test_config();
        let join_result =
            tokio::task::spawn_blocking(move || Backup::run(&config, "nonexistent-job", false))
                .await;
        assert!(
            join_result.is_ok(),
            "spawn_blocking closure should not panic"
        );
        assert!(
            join_result.unwrap().is_err(),
            "unknown job should error out before touching restic"
        );
    }

    #[test]
    fn test_no_scheduled_jobs() {
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
