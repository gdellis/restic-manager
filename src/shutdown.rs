//! Shutdown handling for the one-shot commands.
//!
//! The daemon has its own shutdown handling (see `Scheduler::run_async`):
//! it drains in-flight backups on the first signal and force-exits on the
//! second. The one-shot commands have no such contract - they run a single
//! `restic` command - so a shutdown signal terminates the running `restic`
//! child and exits with the conventional "interrupted" status.
//!
//! Without this, a SIGTERM (systemd stopping the unit, or the TUI stopping
//! the child it spawned) killed the manager while `restic` kept running
//! against the repository, holding the repository lock so the next run
//! failed with "repository is already locked".

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// Set once a shutdown signal has been received.
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// The exit code used when a command is interrupted by a shutdown signal.
///
/// The same code the daemon uses for its force-exit, so a caller sees one
/// convention for "interrupted" across both modes.
pub const INTERRUPTED_EXIT_CODE: i32 = 130;

/// How long the shutdown watcher gives the main thread to notice the
/// interruption and exit on its own, before it exits the process itself.
const MAIN_THREAD_GRACE: Duration = Duration::from_millis(100);

/// Whether a shutdown signal has been received.
pub fn shutdown_requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

/// Records that a shutdown signal was received.
fn request_shutdown() {
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
}

/// Terminates the running `restic` children and exits with the
/// "interrupted" status.
///
/// Shared by the one-shot watcher and the daemon's force-exit, so both
/// terminate the `restic` child before the process goes away: a `restic`
/// that outlives the manager keeps the repository locked.
pub fn exit_interrupted() -> ! {
    crate::restic_child::terminate_all();

    // process::exit skips destructors, so flush explicitly: the log lines
    // written just before this must not be lost if the log writer ever
    // becomes buffered.
    use std::io::Write;
    let _ = std::io::stderr().flush();
    std::process::exit(INTERRUPTED_EXIT_CODE);
}

/// Returns a future that resolves when a shutdown signal arrives: Ctrl-C on
/// all platforms, plus SIGTERM on unix so `systemctl stop` asks for a
/// cleanup instead of killing the process outright.
///
/// A plain fn returning a future (not an `async fn`) so the handlers are
/// installed at call time, before the caller reports that it is ready,
/// rather than lazily on first poll.
#[cfg(unix)]
pub fn shutdown_signal() -> impl std::future::Future<Output = ()> {
    use tokio::signal::unix::{signal as unix_signal, SignalKind};
    let sigterm = unix_signal(SignalKind::terminate());
    let ctrl_c = tokio::signal::ctrl_c();
    async move {
        match sigterm {
            Ok(mut sigterm) => tokio::select! {
                res = ctrl_c => if let Err(e) = res {
                    tracing::error!(error = %e, "Failed to listen for Ctrl-C");
                },
                _ = sigterm.recv() => {}
            },
            Err(e) => {
                tracing::error!(error = %e, "Failed to install SIGTERM handler; Ctrl-C only");
                if let Err(e) = ctrl_c.await {
                    tracing::error!(error = %e, "Failed to listen for Ctrl-C");
                }
            }
        }
    }
}

#[cfg(not(unix))]
pub fn shutdown_signal() -> impl std::future::Future<Output = ()> {
    let ctrl_c = tokio::signal::ctrl_c();
    async move {
        if let Err(e) = ctrl_c.await {
            tracing::error!(error = %e, "Failed to listen for Ctrl-C");
        }
    }
}

/// Watches for a shutdown signal and, when one arrives, terminates the
/// running `restic` children before exiting.
///
/// Returns only once the SIGTERM handler is installed, so a signal that
/// arrives right after this call cannot slip through unhandled. Ctrl-C is
/// registered when the watcher's select is first polled, which happens
/// immediately afterwards; until then Ctrl-C keeps its default
/// disposition, and no `restic` child exists yet anyway.
pub fn watch_for_shutdown() {
    let (ready_tx, ready_rx) = mpsc::channel();

    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(e) => {
                tracing::error!(error = %e, "Failed to build the shutdown-signal runtime");
                let _ = ready_tx.send(());
                return;
            }
        };

        runtime.block_on(async move {
            // Creating the signal future installs the handlers; only then is
            // it safe to tell the caller that shutdowns are handled.
            let signal = shutdown_signal();
            let _ = ready_tx.send(());
            signal.await;
        });

        request_shutdown();
        crate::restic_child::terminate_all();

        // Give the main thread a moment to unwind and report the
        // interruption itself, so the exit is orderly rather than a
        // truncated error message.
        std::thread::sleep(MAIN_THREAD_GRACE);
        exit_interrupted();
    });

    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(()) => {}
        Err(e) => {
            tracing::error!(error = %e, "Shutdown signal handlers were not installed in time")
        }
    }
}
