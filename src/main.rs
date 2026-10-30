use restic_manager::{cli_run, shutdown};

fn main() {
    if let Err(e) = cli_run() {
        // A shutdown signal terminated the in-flight `restic` child: that is
        // an interruption of the command, not a failure of the command
        // itself.
        if shutdown::shutdown_requested() {
            eprintln!("Interrupted by shutdown signal.");
            std::process::exit(shutdown::INTERRUPTED_EXIT_CODE);
        }
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
