use crate::backup::Backup;
use crate::config::ResolvedConfig;
use crate::errors::AppError;
use crate::repository::Repository;
use crate::restore::Restore;
use crate::scheduler::Scheduler;
use crate::snapshot::SnapshotManager;
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "restic-manager")]
#[command(about = "Manage restic backups with scheduling and notifications", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShowKind {
    Job,
    Repo,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "Run a backup job")]
    Run {
        name: String,
        #[arg(
            long,
            help = "Preview the backup without writing data; also skips pre/post-backup hooks"
        )]
        dry_run: bool,
    },
    #[command(about = "Restore from a backup job")]
    Restore {
        name: String,
        #[arg(long)]
        snapshot: Option<String>,
        #[arg(
            long,
            help = "Directory to restore into (required, to avoid accidentally overwriting the current directory)"
        )]
        target: String,
    },
    #[command(about = "Prune old snapshots for a job")]
    Prune { name: String },
    #[command(about = "List snapshots for a job")]
    List {
        name: String,
        #[arg(
            long,
            help = "Output format, only 'json' supported. Defaults to plain text.",
            value_parser = clap::builder::PossibleValuesParser::new(["json"])
        )]
        format: Option<String>,
    },
    #[command(about = "Check repository integrity for a job")]
    Check { name: String },
    #[command(about = "Unlock repository for a job")]
    Unlock { name: String },
    #[command(
        about = "Run the scheduler daemon",
        long_about = "Run the scheduler daemon in the foreground. SIGINT (Ctrl-C) or SIGTERM \
                      stops scheduling and drains in-flight backups before exiting; a second \
                      signal during the drain force-exits immediately (exit code 130). See the \
                      README section \"Running as a systemd service\" for production use."
    )]
    Daemon,
    #[command(about = "List all jobs")]
    Jobs {
        #[arg(
            long,
            help = "Output format, only 'json' supported. Defaults to plain text.",
            value_parser = clap::builder::PossibleValuesParser::new(["json"])
        )]
        format: Option<String>,
    },
    Repos {
        #[arg(
            long,
            help = "Output format, only 'json' supported. Defaults to plain text.",
            value_parser = clap::builder::PossibleValuesParser::new(["json"])
        )]
        format: Option<String>,
    },

    #[command(about = "Initialize a repository")]
    Init { name: String },
    #[command(about = "Initialize or reset the exclude file with defaults")]
    InitExclude,
    #[command(about = "Open the interactive terminal dashboard")]
    Tui,
    #[command(about = "Show full details for a job or repository as JSON")]
    Show {
        kind: ShowKind,
        name: String,
        #[arg(
            long,
            help = "Output format, only 'json' supported. Defaults to plain text.",
            value_parser = clap::builder::PossibleValuesParser::new(["json"])
        )]
        format: Option<String>,
    },
}

pub fn cli_run() -> Result<(), AppError> {
    // Logs go to stderr so command output on stdout stays clean; under
    // systemd both streams land in the journal. RUST_LOG overrides the
    // default `info` level. ANSI colors only when stderr is a terminal,
    // so journal/file output stays free of escape sequences. try_init
    // rather than init so a second call (e.g. from a test harness) is a
    // no-op instead of a panic.
    let env_filter = build_env_filter();

    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .try_init()
        .ok();

    let cli = Cli::parse();
    let config = ResolvedConfig::load()?;

    match cli.command {
        Commands::Run { name, dry_run } => {
            let result = Backup::run(&config, &name, dry_run)?;
            // This one-shot `run` invocation has no NotificationManager, so
            // a partial result is only ever surfaced here via println! -
            // there's no separate notify_partial call to add for the CLI
            // path, unlike the scheduler's dispatch arm.
            if result.partial {
                println!(
                    "Backup completed with errors (partial): {} new, {} changed, {} unchanged files. \
                     {} file(s) could not be read.",
                    result.files_new,
                    result.files_changed,
                    result.files_unmodified,
                    result.errors_count
                );
            } else {
                println!(
                    "Backup completed: {} new, {} changed, {} unchanged files",
                    result.files_new, result.files_changed, result.files_unmodified
                );
            }
            if let Some(snap) = result.snapshot_id {
                println!("Snapshot ID: {}", snap);
            }
        }
        Commands::Restore {
            name,
            snapshot,
            target,
        } => {
            if let Some(snap) = snapshot {
                Restore::restore_snapshot(&config, &name, &snap, &target)?;
                println!("Restored snapshot {} to {}", snap, target);
            } else {
                let snap = Restore::restore_latest(&config, &name, &target)?;
                println!("Restored latest snapshot {} to {}", snap, target);
            }
        }
        Commands::Prune { name } => {
            let removed = SnapshotManager::apply_retention(&config, &name, false)?;
            println!(
                "Retention applied for job '{}': removed {} snapshot(s)",
                name,
                removed.len()
            );
        }
        Commands::List { name, format } => {
            let snapshots = SnapshotManager::list(&config, &name)?;
            if format.as_deref() == Some("json") {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&snapshots.snapshots)
                        .unwrap_or_else(|e| format!("[json error: {}]", e))
                );
            } else {
                println!("Snapshots for job '{}':", name);
                for snap in snapshots.snapshots {
                    println!("  {}  {}  {:?}", snap.short_id, snap.time, snap.paths);
                }
            }
        }
        Commands::Check { name } => {
            Repository::check(&config, &name)?;
        }
        Commands::Unlock { name } => {
            Repository::unlock(&config, &name)?;
        }
        Commands::Daemon => {
            let mut scheduler = Scheduler::new(config)?;
            scheduler.run()?;
        }
        Commands::Jobs { format } => {
            let jobs = config.config.list_jobs();
            if format.as_deref() == Some("json") {
                let names: Vec<String> = jobs.into_iter().map(String::from).collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&names)
                        .unwrap_or_else(|e| format!("[json error: {}]", e))
                );
            } else {
                println!("Configured jobs:");
                for job in jobs {
                    println!("  - {}", job);
                }
            }
        }
        Commands::Repos { format } => {
            let repos = config.config.list_repositories();
            if format.as_deref() == Some("json") {
                let names: Vec<String> = repos.into_iter().map(String::from).collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&names)
                        .unwrap_or_else(|e| format!("[json error: {}]", e))
                );
            } else {
                println!("Configured repositories:");
                for repo in repos {
                    println!("  - {}", repo);
                }
            }
        }
        Commands::Init { name } => {
            Repository::init(&config, &name)?;
        }
        Commands::InitExclude => {
            let path = crate::exclude::ensure_default_exclude_file()?;
            println!("Created default exclude file at: {}", path.display());
        }
        Commands::Tui => {
            crate::tui::run()?;
        }
        Commands::Show { kind, name, format } => match kind {
            ShowKind::Job => {
                let job = config
                    .config
                    .get_job(&name)
                    .ok_or_else(|| AppError::Other(format!("Job '{}' not found", name)))?;
                if format.as_deref() == Some("json") {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(job)
                            .unwrap_or_else(|e| format!("[json error: {}]", e))
                    );
                } else {
                    println!("Job '{}' details:", name);
                    println!("  repository: {}", job.repository);
                    println!(
                        "  paths: {}",
                        job.paths
                            .iter()
                            .filter_map(|p| p.to_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    if let Some(schedule) = &job.schedule {
                        println!("  schedule: {}", schedule);
                    } else {
                        println!("  schedule: (none)");
                    }
                    if let Some(retention) = &job.retention {
                        println!("  retention:");
                        if let Some(v) = retention.keep_last {
                            println!("    keep_last: {}", v);
                        }
                        if let Some(v) = retention.keep_hourly {
                            println!("    keep_hourly: {}", v);
                        }
                        if let Some(v) = retention.keep_daily {
                            println!("    keep_daily: {}", v);
                        }
                        if let Some(v) = retention.keep_weekly {
                            println!("    keep_weekly: {}", v);
                        }
                        if let Some(v) = retention.keep_monthly {
                            println!("    keep_monthly: {}", v);
                        }
                        if let Some(v) = retention.keep_yearly {
                            println!("    keep_yearly: {}", v);
                        }
                    } else {
                        println!("  retention: (none)");
                    }
                    println!(
                        "  notifications: on_failure={} on_success={}",
                        job.notifications.on_failure, job.notifications.on_success
                    );
                    println!(
                        "  hooks: pre_backup={} post_backup={}",
                        job.pre_backup.len(),
                        job.post_backup.len()
                    );
                }
            }
            ShowKind::Repo => {
                let repo = config
                    .config
                    .get_repository(&name)
                    .ok_or_else(|| AppError::Other(format!("Repository '{}' not found", name)))?;
                if format.as_deref() == Some("json") {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(repo)
                            .unwrap_or_else(|e| format!("[json error: {}]", e))
                    );
                } else {
                    println!("Repository '{}' details:", name);
                    println!("  repo: {}", repo.repo);
                    let masked = if repo.password_key.len() > 2 {
                        format!("{}***", &repo.password_key[..2])
                    } else {
                        "***".to_string()
                    };
                    println!("  password_key: {}", masked);
                    if let Some(log_cli_output) = &repo.log_cli_output {
                        println!("  log_cli_output: {}", log_cli_output.display());
                    }
                }
            }
        },
    }

    Ok(())
}

/// Build a filter from a `RUST_LOG`-style directive string, with
/// fallback to default `info` level on parse error. Emits a warning
/// to stderr if the directive is non-empty but invalid.
///
/// Pure: no environment access, so callers (and tests) can drive any
/// directive without touching the process-global `RUST_LOG`.
fn filter_from_directive(directive: &str) -> tracing_subscriber::EnvFilter {
    if directive.is_empty() {
        return tracing_subscriber::EnvFilter::new("info");
    }
    match tracing_subscriber::EnvFilter::try_new(directive) {
        Ok(filter) => filter,
        Err(e) => {
            eprintln!("Invalid RUST_LOG directive ({}); using default 'info'", e);
            tracing_subscriber::EnvFilter::new("info")
        }
    }
}

/// Build an EnvFilter from the `RUST_LOG` env var, with fallback to
/// default 'info' level. Thin wrapper over `filter_from_directive`:
/// reads the env, then delegates. The split exists so the directive
/// parser is unit-testable in isolation; the env read is a single
/// line that only the production caller needs.
fn build_env_filter() -> tracing_subscriber::EnvFilter {
    let directive = std::env::var("RUST_LOG").unwrap_or_default();
    filter_from_directive(&directive)
}

#[cfg(test)]
mod tests {
    use super::filter_from_directive;
    use super::{Cli, Commands};
    use clap::Parser;
    use std::sync::Mutex;
    use tracing_subscriber::filter::LevelFilter;

    // `filter_from_directive` is pure (no env access), so the tests
    // for it don't race on RUST_LOG. The env-touching smoke test
    // for `build_env_filter` is serialized via this mutex because
    // it does set/unset the process-global `RUST_LOG`. tarpaulin
    // runs tests in a single process with its own thread
    // orchestration, where this race was previously visible.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn filter_from_empty_directive_uses_default() {
        let filter = filter_from_directive("");
        assert_eq!(filter.max_level_hint(), Some(LevelFilter::INFO));
    }

    #[test]
    fn filter_from_debug_directive_enables_debug() {
        let filter = filter_from_directive("debug");
        assert!(filter
            .max_level_hint()
            .is_some_and(|l| l >= tracing::Level::DEBUG));
    }

    #[test]
    fn filter_from_invalid_directive_falls_back_to_default() {
        // Regression test for #49: invalid RUST_LOG should fall back
        // to default info level, with a warning to stderr.
        let filter = filter_from_directive("foo=bar");
        assert_eq!(filter.max_level_hint(), Some(LevelFilter::INFO));
    }

    #[test]
    fn build_env_filter_reads_rust_log() {
        // Serialized because `build_env_filter` reads the process-global
        // RUST_LOG. The pure-function tests above cover the directive
        // parsing; this one only verifies the env read + delegation.
        let _guard = ENV_LOCK.lock().unwrap();
        let original = std::env::var("RUST_LOG").ok();

        std::env::set_var("RUST_LOG", "debug");
        let filter = super::build_env_filter();
        assert!(filter
            .max_level_hint()
            .is_some_and(|l| l >= tracing::Level::DEBUG));

        match original {
            Some(val) => std::env::set_var("RUST_LOG", val),
            None => std::env::remove_var("RUST_LOG"),
        }
    }

    #[test]
    fn jobs_format_json_parses() {
        let cli = Cli::try_parse_from(["restic-manager", "jobs", "--format", "json"])
            .expect("parse should succeed");
        match cli.command {
            Commands::Jobs { format } => assert_eq!(format.as_deref(), Some("json")),
            _ => panic!("expected Jobs variant"),
        }
    }

    #[test]
    fn repos_format_json_parses() {
        let cli = Cli::try_parse_from(["restic-manager", "repos", "--format", "json"])
            .expect("parse should succeed");
        match cli.command {
            Commands::Repos { format } => assert_eq!(format.as_deref(), Some("json")),
            _ => panic!("expected Repos variant"),
        }
    }

    #[test]
    fn list_format_json_parses() {
        let cli = Cli::try_parse_from(["restic-manager", "list", "documents", "--format", "json"])
            .expect("parse should succeed");
        match cli.command {
            Commands::List { name, format } => {
                assert_eq!(name, "documents");
                assert_eq!(format.as_deref(), Some("json"));
            }
            _ => panic!("expected List variant"),
        }
    }

    #[test]
    fn jobs_format_yaml_rejected() {
        assert!(Cli::try_parse_from(["restic-manager", "jobs", "--format", "yaml"]).is_err());
    }

    #[test]
    fn repos_format_yaml_rejected() {
        assert!(Cli::try_parse_from(["restic-manager", "repos", "--format", "yaml"]).is_err());
    }

    #[test]
    fn list_format_yaml_rejected() {
        assert!(
            Cli::try_parse_from(["restic-manager", "list", "documents", "--format", "yaml"])
                .is_err()
        );
    }

    #[test]
    fn show_job_json_parses() {
        let cli = Cli::try_parse_from([
            "restic-manager",
            "show",
            "job",
            "documents",
            "--format",
            "json",
        ])
        .expect("parse should succeed");
        match cli.command {
            Commands::Show { kind, name, format } => {
                assert_eq!(kind, super::ShowKind::Job);
                assert_eq!(name, "documents");
                assert_eq!(format.as_deref(), Some("json"));
            }
            _ => panic!("expected Show variant"),
        }
    }

    #[test]
    fn show_repo_json_parses() {
        let cli = Cli::try_parse_from([
            "restic-manager",
            "show",
            "repo",
            "local",
            "--format",
            "json",
        ])
        .expect("parse should succeed");
        match cli.command {
            Commands::Show { kind, name, format } => {
                assert_eq!(kind, super::ShowKind::Repo);
                assert_eq!(name, "local");
                assert_eq!(format.as_deref(), Some("json"));
            }
            _ => panic!("expected Show variant"),
        }
    }

    #[test]
    fn show_job_yaml_rejected() {
        assert!(Cli::try_parse_from([
            "restic-manager",
            "show",
            "job",
            "documents",
            "--format",
            "yaml"
        ])
        .is_err());
    }

    #[test]
    fn show_repo_yaml_rejected() {
        assert!(Cli::try_parse_from([
            "restic-manager",
            "show",
            "repo",
            "local",
            "--format",
            "yaml"
        ])
        .is_err());
    }
}
