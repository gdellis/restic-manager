//! Tracking of the `restic` child processes this manager spawns.
//!
//! `restic` holds a lock on the repository while it runs, so a `restic`
//! process that outlives its manager keeps the repository locked and the
//! next run fails with "repository is already locked". Every `restic`
//! invocation therefore goes through a tracked child, and the shutdown
//! paths terminate the live ones before the manager exits.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// How often a waiting child is polled for completion.
///
/// The wait polls rather than blocking in `Child::wait` so a concurrent
/// shutdown can take the child lock and terminate the process: a blocking
/// wait would hold the lock for the whole run, and the shutdown would have
/// to wait for the run to finish on its own.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// The `restic` children this manager currently has running.
///
/// Weak references, so the registry never keeps a child alive: it only lets
/// the shutdown paths find the ones that are still running.
static LIVE_CHILDREN: Mutex<Vec<Weak<TrackedChild>>> = Mutex::new(Vec::new());

/// A running `restic` child, shared between its owner and the shutdown path
/// that may need to terminate it.
struct TrackedChild {
    child: Mutex<Child>,
    /// Set once the child has been reaped, so a late termination attempt
    /// cannot signal a recycled process id.
    reaped: AtomicBool,
}

impl TrackedChild {
    /// Terminates the child, ignoring failures.
    ///
    /// A child that has already been reaped is left alone: its process id
    /// may have been reused by an unrelated process, and killing that would
    /// be worse than doing nothing.
    fn terminate(&self) {
        let mut child = match self.child.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if self.reaped.load(Ordering::SeqCst) {
            return;
        }
        let _ = child.kill();
    }
}

impl Drop for TrackedChild {
    fn drop(&mut self) {
        // The owner went away without waiting for the child - an early
        // return or a panic. Do not leave a `restic` behind holding the
        // repository lock.
        self.terminate();
    }
}

/// A `restic` child process the manager keeps track of.
pub struct ResticChild {
    tracked: Arc<TrackedChild>,
}

impl ResticChild {
    /// Spawns `cmd` and registers the child, so that a shutdown can
    /// terminate it instead of leaving it behind.
    pub fn spawn(cmd: &mut Command) -> std::io::Result<Self> {
        let child = cmd.spawn()?;
        let tracked = Arc::new(TrackedChild {
            child: Mutex::new(child),
            reaped: AtomicBool::new(false),
        });
        register(Arc::downgrade(&tracked));
        Ok(Self { tracked })
    }

    /// Takes the child's stdout pipe, if one was configured.
    pub fn take_stdout(&self) -> Option<std::process::ChildStdout> {
        self.lock().stdout.take()
    }

    /// Takes the child's stderr pipe, if one was configured.
    pub fn take_stderr(&self) -> Option<std::process::ChildStderr> {
        self.lock().stderr.take()
    }

    /// Waits for the child to exit and returns its exit status.
    pub fn wait(&self) -> std::io::Result<ExitStatus> {
        loop {
            {
                let mut child = self.lock();
                if let Some(status) = child.try_wait()? {
                    self.tracked.reaped.store(true, Ordering::SeqCst);
                    return Ok(status);
                }
            }
            std::thread::sleep(WAIT_POLL_INTERVAL);
        }
    }

    /// Terminates the child.
    pub fn terminate(&self) {
        self.tracked.terminate();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Child> {
        match self.tracked.child.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Registers a child so [`terminate_all`] can find it.
fn register(handle: Weak<TrackedChild>) {
    let mut live = match LIVE_CHILDREN.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    // Drop entries whose owner is gone, so the registry does not grow
    // without bound over the daemon's lifetime.
    live.retain(|entry| entry.strong_count() > 0);
    live.push(handle);
}

/// Terminates every `restic` child this manager currently has running.
///
/// Called from the shutdown paths that would otherwise leave a `restic`
/// process behind holding the repository lock.
pub fn terminate_all() {
    let handles: Vec<Weak<TrackedChild>> = {
        let live = match LIVE_CHILDREN.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        live.clone()
    };

    for handle in handles {
        if let Some(tracked) = handle.upgrade() {
            tracked.terminate();
        }
    }
}

/// Runs `cmd` to completion and captures its stdout and stderr.
///
/// Equivalent to [`Command::output`], except the child is tracked, so a
/// shutdown can terminate it instead of leaving it running against the
/// repository.
pub fn capture_output(cmd: &mut Command) -> std::io::Result<Output> {
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let child = ResticChild::spawn(cmd)?;
    let stdout = child.take_stdout();
    let stderr = child.take_stderr();

    // Drain stderr on a dedicated thread while stdout is read here. If we
    // read stdout to completion before touching stderr, restic can fill the
    // ~64KB stderr pipe buffer and block on write() while we are blocked
    // reading stdout -> deadlock.
    let stderr_thread = stderr.map(|mut pipe| {
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let _ = pipe.read_to_end(&mut buffer);
            buffer
        })
    });

    let mut stdout_buffer = Vec::new();
    if let Some(mut pipe) = stdout {
        pipe.read_to_end(&mut stdout_buffer)?;
    }

    let status = child.wait()?;
    let stderr_buffer = match stderr_thread {
        Some(handle) => handle.join().unwrap_or_else(|_| Vec::new()),
        None => Vec::new(),
    };

    Ok(Output {
        status,
        stdout: stdout_buffer,
        stderr: stderr_buffer,
    })
}
