use crate::config::ResolvedConfig;
use crate::errors::AppError;
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
    Run { name: String },
    #[command(about = "Restore from a backup job")]
    Restore {
        name: String,
        #[arg(long)]
        snapshot: Option<String>,
        #[arg(long, default_value = ".")]
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
}

pub fn cli_run() -> Result<(), AppError> {
    let cli = Cli::parse();
    let config = ResolvedConfig::load()?;

    match cli.command {
        Commands::Run { name } => {
            println!("Running backup job: {}", name);
        }
        Commands::Restore {
            name,
            snapshot,
            target,
        } => {
            println!("Restoring job: {} to {}", name, target);
            if let Some(snap) = snapshot {
                println!("Snapshot: {}", snap);
            }
        }
        Commands::Prune { name } => {
            println!("Pruning job: {}", name);
        }
        Commands::List { name } => {
            println!("Listing snapshots for: {}", name);
        }
        Commands::Check { name } => {
            println!("Checking repository for: {}", name);
        }
        Commands::Unlock { name } => {
            println!("Unlocking repository for: {}", name);
        }
        Commands::Daemon => {
            println!("Starting scheduler daemon...");
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
            println!("Initializing repository: {}", name);
        }
    }

    Ok(())
}
