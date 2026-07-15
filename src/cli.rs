use crate::backup::Backup;
use crate::config::ResolvedConfig;
use crate::errors::AppError;
use crate::repository::Repository;
use crate::restore::Restore;
use crate::scheduler::Scheduler;
use crate::snapshot::SnapshotManager;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "restic-manager")]
#[command(about = "Manage restic backups with scheduling and notifications", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
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
    List { name: String },
    #[command(about = "Check repository integrity for a job")]
    Check { name: String },
    #[command(about = "Unlock repository for a job")]
    Unlock { name: String },
    #[command(about = "Run the scheduler daemon")]
    Daemon,
    #[command(about = "List all jobs")]
    Jobs,
    #[command(about = "List all repositories")]
    Repos,
    #[command(about = "Initialize a repository")]
    Init { name: String },
    #[command(about = "Initialize or reset the exclude file with defaults")]
    InitExclude,
}

pub fn cli_run() -> Result<(), AppError> {
    // Logs go to stderr so command output on stdout stays clean; under
    // systemd both streams land in the journal. RUST_LOG overrides the
    // default `info` level. ANSI colors only when stderr is a terminal,
    // so journal/file output stays free of escape sequences. try_init
    // rather than init so a second call (e.g. from a test harness) is a
    // no-op instead of a panic.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
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
        Commands::List { name } => {
            let snapshots = SnapshotManager::list(&config, &name)?;
            println!("Snapshots for job '{}':", name);
            for snap in snapshots.snapshots {
                println!("  {}  {}  {:?}", snap.short_id, snap.time, snap.paths);
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
        Commands::Jobs => {
            let jobs = config.config.list_jobs();
            println!("Configured jobs:");
            for job in jobs {
                println!("  - {}", job);
            }
        }
        Commands::Repos => {
            let repos = config.config.list_repositories();
            println!("Configured repositories:");
            for repo in repos {
                println!("  - {}", repo);
            }
        }
        Commands::Init { name } => {
            Repository::init(&config, &name)?;
        }
        Commands::InitExclude => {
            let path = crate::exclude::ensure_default_exclude_file()?;
            println!("Created default exclude file at: {}", path.display());
        }
    }

    Ok(())
}
