use restic_manager::cli_run;

fn main() {
    if let Err(e) = cli_run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
