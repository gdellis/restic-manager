use crate::backup::Backup;
use crate::config::Job;
use crate::config::ResolvedConfig;
use crate::errors::AppError;
use crate::notifications::NotificationManager;
use chrono::DateTime;
use chrono::Local;
use chrono::Timelike;
use cron::Schedule;
use std::collections::HashMap;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::signal;
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

/// Removes its job name from the in-flight set when dropped, including
/// during a panic unwind, so a job can never get stuck permanently marked
/// as running if something in its dispatch task panics.
struct InFlightGuard {
    in_flight: Arc<Mutex<HashSet<String>>>,
    name: String,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.in_flight.lock().unwrap().remove(&self.name);
    }
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
        let normalized = crate::config::normalize_cron(s);
        Schedule::from_str(&normalized)
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

    /// Checks whether `job_name` is already running and, if not, marks it as
    /// running. Returns `false` if it was already in-flight (in which case
    /// the caller should skip dispatching this trigger). This function does
    /// no locking of its own: the caller must hold a lock on `in_flight` for
    /// the duration of the call for the check-and-mark to be atomic. Release
    /// is handled separately by `InFlightGuard`, not by this function.
    fn try_start(in_flight: &mut HashSet<String>, job_name: &str) -> bool {
        if in_flight.contains(job_name) {
            false
        } else {
            in_flight.insert(job_name.to_string());
            true
        }
    }

    pub fn run(&mut self) -> Result<(), AppError> {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(self.run_async())
    }

    /// Returns a future that resolves when a shutdown signal arrives:
    /// Ctrl-C on all platforms, plus SIGTERM on unix so `systemctl stop`
    /// triggers the same graceful drain of in-flight backups instead of
    /// killing the process outright. A plain fn returning a future (not an
    /// `async fn`) so the SIGTERM handler is installed at call time, before
    /// the caller logs that it is ready, rather than lazily on first poll.
    ///
    /// Note: `tokio::signal::ctrl_c()` is process-global — the main-loop
    /// future and the drain-phase `force_exit` future share one underlying
    /// handler. That's sound while the two are created sequentially in a
    /// single scheduler run, but a refactor toward concurrent callers would
    /// need to route signals through one shared subscription instead.
    #[cfg(unix)]
    fn shutdown_signal() -> impl std::future::Future<Output = ()> {
        use tokio::signal::unix::{signal as unix_signal, SignalKind};
        let sigterm = unix_signal(SignalKind::terminate());
        async move {
            match sigterm {
                Ok(mut sigterm) => tokio::select! {
                    res = signal::ctrl_c() => if let Err(e) = res {
                        error!(error = %e, "Failed to listen for Ctrl-C");
                    },
                    _ = sigterm.recv() => {}
                },
                Err(e) => {
                    error!(error = %e, "Failed to install SIGTERM handler; Ctrl-C only");
                    if let Err(e) = signal::ctrl_c().await {
                        error!(error = %e, "Failed to listen for Ctrl-C");
                    }
                }
            }
        }
    }

    #[cfg(not(unix))]
    fn shutdown_signal() -> impl std::future::Future<Output = ()> {
        async {
            if let Err(e) = signal::ctrl_c().await {
                error!(error = %e, "Failed to listen for Ctrl-C");
            }
        }
    }

    async fn run_async(&mut self) -> Result<(), AppError> {
        if self.jobs.is_empty() {
            info!("No scheduled jobs configured");
            return Ok(());
        }

        let (tx, mut rx) = mpsc::channel::<String>(100);

        // Cron granularity is one minute, so tick every 60s by default. The
        // env override exists solely for the integration tests, which
        // otherwise would have to wait out a real minute boundary to see a
        // job trigger. It is NOT a stable public interface: don't set it in
        // production, and it may change or disappear without notice.
        let tick_interval = std::env::var("RESTIC_MANAGER_TEST_TICK_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|&s| s > 0)
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(60));
        let mut ticker = interval(tick_interval);
        // tokio::time::interval fires its first tick immediately rather than
        // after one full interval; consume it here so a job scheduled for
        // "every minute" doesn't get a spurious extra run right at startup.
        ticker.tick().await;
        let mut last_triggered: HashMap<String, DateTime<Local>> = HashMap::new();
        let in_flight: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        let mut tasks = tokio::task::JoinSet::new();

        // Created once outside the loop so the SIGTERM stream isn't
        // re-registered on every select iteration, and before the
        // "Scheduler started" log so that line reliably means signals are
        // being handled (the integration test depends on that ordering).
        let shutdown = Self::shutdown_signal();
        tokio::pin!(shutdown);

        info!(job_count = self.jobs.len(), "Scheduler started");

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    // with_second(0)/with_nanosecond(0) only return None when the
                    // *argument* is out of range (sec >= 60, nanos >= 2_000_000_000);
                    // chrono guarantees Local::now() itself never produces a value
                    // outside that range, so this branch is unreachable today. Skip
                    // the tick rather than panic the daemon if that invariant ever
                    // changes in a future chrono release.
                    //
                    // Caveat: dedup below is wall-clock based, so a backward NTP/DST
                    // clock jump can re-trigger a job and a forward jump can skip one;
                    // this uses Local::now() rather than a monotonic clock by design
                    // for a personal backup tool's simplicity.
                    let Some(now) = Local::now()
                        .with_second(0)
                        .and_then(|t| t.with_nanosecond(0))
                    else {
                        error!("Failed to truncate current time to the minute; skipping tick");
                        continue;
                    };
                    for (job_name, entry) in &self.jobs {
                        if Self::mark_if_due(&entry.schedule, now, &mut last_triggered, job_name) {
                            if let Err(e) = tx.send(job_name.clone()).await {
                                warn!("Failed to queue job: {}", e);
                            }
                        }
                    }
                }

                Some(name) = rx.recv() => {
                    {
                        let mut running = in_flight.lock().unwrap();
                        if !Self::try_start(&mut running, &name) {
                            warn!(job = %name, "Skipping trigger: job is already running");
                            continue;
                        }
                    }

                    let config = self.config.clone();
                    let job = config.config.get_job(&name).cloned();
                    let in_flight = Arc::clone(&in_flight);
                    tasks.spawn(Self::run_backup_job(config, job, name, in_flight));
                }

                _ = &mut shutdown => {
                    info!("Shutdown signal received, finishing current jobs");
                    break;
                }
            }
        }

        // Drain in-flight backups so Ctrl+C or SIGTERM doesn't cancel a
        // running restic job mid-write and leave a stale repository lock
        // behind. Join errors (a panicked backup task) are deliberately
        // degraded to logs here: the drain must keep waiting on the
        // remaining tasks, and per-job failures were already reported via
        // notifications. A second signal during the drain force-exits
        // immediately so an operator isn't held hostage by a long backup,
        // at the cost of possibly leaving a stale restic lock. 130 is used
        // as a conventional "interrupted" exit code for either signal.
        let force_exit = Self::shutdown_signal();
        tokio::pin!(force_exit);
        // Logged only after the force-exit handler is armed, mirroring the
        // "Scheduler started" ordering above; the integration test keys off
        // this line before sending the second signal.
        info!("Draining in-flight jobs; a second signal force-exits");
        loop {
            tokio::select! {
                res = tasks.join_next() => match res {
                    None => break,
                    Some(Err(join_err)) => {
                        error!(error = %join_err, "Backup task failed to join during shutdown");
                    }
                    Some(Ok(())) => {}
                },
                _ = &mut force_exit => {
                    warn!("Second shutdown signal received, exiting without waiting for in-flight jobs");
                    // process::exit skips destructors, so flush explicitly:
                    // the warn line above must not be lost if the log writer
                    // ever becomes buffered.
                    use std::io::Write;
                    let _ = std::io::stderr().flush();
                    std::process::exit(130);
                }
            }
        }

        info!("Scheduler stopped");
        Ok(())
    }

    async fn run_backup_job(
        config: ResolvedConfig,
        job: Option<Job>,
        name: String,
        in_flight: Arc<Mutex<HashSet<String>>>,
    ) {
        let _guard = InFlightGuard {
            in_flight,
            name: name.clone(),
        };

        info!(job = %name, "Starting scheduled backup");

        let notifier = job
            .as_ref()
            .map(|j| NotificationManager::new(&config, j.notifications.clone()));

        let blocking_name = name.clone();
        let join_result =
            tokio::task::spawn_blocking(move || Backup::run(&config, &blocking_name, false)).await;

        match join_result {
            Ok(Ok(result)) if result.partial => {
                warn!(job = %name, snapshot = ?result.snapshot_id, errors_count = result.errors_count, "Backup completed with errors (partial)");
                if let Some(ref n) = notifier {
                    let error_detail = if result.errors_count > 0 {
                        format!("{} file(s) could not be read", result.errors_count)
                    } else {
                        "some files could not be read".to_string()
                    };
                    // Best-effort: a Telegram outage here shouldn't fail the whole
                    // scheduled-run task (the backup itself already
                    // succeeded/partially succeeded), but log it so a real outage
                    // is still visible.
                    if let Err(e) = n
                        .notify_partial(&name, result.snapshot_id.as_deref(), &error_detail)
                        .await
                    {
                        warn!(job = %name, error = %e, "Failed to send partial-backup notification");
                    }
                }
            }
            Ok(Ok(result)) => {
                info!(job = %name, snapshot = ?result.snapshot_id, "Backup completed");
                if let Some(ref n) = notifier {
                    // Best-effort: see the partial-notification comment above for
                    // why send failures are logged, not propagated.
                    if let Err(e) = n.notify_success(&name, result.snapshot_id.as_deref()).await {
                        warn!(job = %name, error = %e, "Failed to send success notification");
                    }
                }
            }
            Ok(Err(e)) => {
                error!(job = %name, error = %e, "Backup failed");
                if let Some(ref n) = notifier {
                    if let Err(notify_err) = n.notify_failure(&name, &e.to_string()).await {
                        warn!(job = %name, error = %notify_err, "Failed to send failure notification");
                    }
                }
            }
            Err(join_err) => {
                // join_result's Err only comes from the spawn_blocking closure
                // above today (Notifications::new is sync and send_telegram only
                // does a plain HTTP request, neither of which can panic), but this
                // arm would also catch a panic anywhere else in this async block if
                // one were ever introduced - that's intentional, not a bug.
                error!(job = %name, error = %join_err, "Backup task panicked");
                if let Some(ref n) = notifier {
                    if let Err(notify_err) = n
                        .notify_failure(&name, &format!("Backup task panicked: {}", join_err))
                        .await
                    {
                        warn!(job = %name, error = %notify_err, "Failed to send panic-failure notification");
                    }
                }
            }
        }
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
        assert_eq!(last_triggered.get("job"), Some(&minute_one));
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

    #[test]
    fn test_try_start_prevents_concurrent_same_job() {
        let mut in_flight = HashSet::new();
        assert!(Scheduler::try_start(&mut in_flight, "job"));
        assert!(!Scheduler::try_start(&mut in_flight, "job"));
    }

    #[test]
    fn test_try_start_allows_different_jobs_concurrently() {
        let mut in_flight = HashSet::new();
        assert!(Scheduler::try_start(&mut in_flight, "job-a"));
        assert!(Scheduler::try_start(&mut in_flight, "job-b"));
    }

    #[test]
    fn test_try_start_allows_rerun_after_release() {
        let mut in_flight = HashSet::new();
        assert!(Scheduler::try_start(&mut in_flight, "job"));
        in_flight.remove("job");
        assert!(Scheduler::try_start(&mut in_flight, "job"));
    }

    #[test]
    fn test_in_flight_guard_removes_on_normal_drop() {
        let in_flight = Arc::new(Mutex::new(HashSet::new()));
        assert!(Scheduler::try_start(&mut in_flight.lock().unwrap(), "job"));
        {
            let _guard = InFlightGuard {
                in_flight: Arc::clone(&in_flight),
                name: "job".to_string(),
            };
        }
        assert!(!in_flight.lock().unwrap().contains("job"));
    }

    #[test]
    fn test_in_flight_guard_removes_on_panic_unwind() {
        // This is the exact scenario flagged in review of PR #25: if
        // something between dispatch and completion panics, the guard's
        // Drop impl must still release the in-flight entry so the job
        // isn't permanently stuck as "running".
        let in_flight = Arc::new(Mutex::new(HashSet::new()));
        assert!(Scheduler::try_start(&mut in_flight.lock().unwrap(), "job"));

        let guard_in_flight = Arc::clone(&in_flight);
        // AssertUnwindSafe: Arc<Mutex<_>> isn't UnwindSafe by default since a
        // panic could in theory leave the Mutex poisoned mid-mutation, but
        // InFlightGuard's Drop only does a single `remove` call and doesn't
        // observe any partially-mutated state, so asserting unwind safety
        // here is sound.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = InFlightGuard {
                in_flight: guard_in_flight,
                name: "job".to_string(),
            };
            panic!("simulated dispatch panic");
        }));

        assert!(result.is_err());
        assert!(!in_flight.lock().unwrap().contains("job"));
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
