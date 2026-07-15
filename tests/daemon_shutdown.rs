//! End-to-end tests for the daemon's shutdown contract:
//! - SIGTERM (what `systemctl stop` sends) must break the scheduler loop
//!   and exit cleanly, not kill the process outright.
//! - A second signal during the drain must force-exit with code 130
//!   instead of waiting for in-flight backups.
//!
//! Linux-only: they point the daemon at a scratch config via
//! XDG_CONFIG_HOME, which `dirs::config_dir()` only honors on Linux.
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

struct Daemon {
    child: Child,
    stderr: Arc<Mutex<String>>,
    reader: Option<JoinHandle<()>>,
}

impl Daemon {
    /// Spawns the daemon against `tmp` as XDG_CONFIG_HOME, collecting its
    /// stderr on a reader thread so tests can both watch for lines while
    /// it runs and assert on the full output after it exits.
    fn spawn(tmp: &Path, extra_env: &[(&str, &str)]) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_restic-manager"));
        cmd.arg("daemon")
            .env("XDG_CONFIG_HOME", tmp)
            .env("RUST_LOG", "info")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().unwrap();

        let stderr = Arc::new(Mutex::new(String::new()));
        let reader = {
            let stderr = Arc::clone(&stderr);
            let pipe = child.stderr.take().unwrap();
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines() {
                    let Ok(line) = line else { break };
                    let mut buf = stderr.lock().unwrap();
                    buf.push_str(&line);
                    buf.push('\n');
                }
            })
        };

        Self {
            child,
            stderr,
            reader: Some(reader),
        }
    }

    /// Waits until `needle` appears on stderr, panicking (after killing
    /// the daemon) if the daemon exits or the deadline passes first.
    fn wait_for_log(&mut self, needle: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            if self.stderr.lock().unwrap().contains(needle) {
                return;
            }
            if self.child.try_wait().unwrap().is_some() || Instant::now() >= deadline {
                self.kill();
                panic!(
                    "daemon did not log {:?}; stderr:\n{}",
                    needle,
                    self.stderr.lock().unwrap()
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Sends SIGTERM to the daemon.
    fn sigterm(&self) {
        let status = Command::new("kill")
            .arg(self.child.id().to_string())
            .status()
            .unwrap();
        assert!(status.success(), "failed to send SIGTERM");
    }

    /// Waits for the daemon to exit, killing it and panicking if it is
    /// still running when the deadline passes.
    fn wait_for_exit(&mut self, timeout: Duration) -> ExitStatus {
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                self.kill();
                panic!("daemon did not exit within {:?}", timeout);
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
        status
    }

    fn kill(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
    }

    fn stderr(&self) -> String {
        self.stderr.lock().unwrap().clone()
    }
}

fn scratch_config(test_name: &str, config_yaml: &str, secrets_yaml: Option<&str>) -> PathBuf {
    let tmp = std::env::temp_dir().join(format!(
        "restic-manager-{}-{}",
        test_name,
        std::process::id()
    ));
    let config_dir = tmp.join("restic-manager");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(config_dir.join("config.yaml"), config_yaml).unwrap();
    if let Some(secrets) = secrets_yaml {
        std::fs::write(config_dir.join("secrets.yaml"), secrets).unwrap();
    }
    tmp
}

#[test]
fn daemon_exits_gracefully_on_sigterm() {
    // One scheduled job is required: with no schedules the daemon exits
    // immediately instead of idling. The 2099 date means it never fires.
    let tmp = scratch_config(
        "sigterm",
        r#"
repositories:
  test:
    repo: /nonexistent/repo
    password_key: test_pass
jobs:
  idle:
    repository: test
    paths:
      - /nonexistent/path
    schedule: "0 0 0 1 1 ? 2099"
"#,
        None,
    );

    let mut daemon = Daemon::spawn(&tmp, &[]);

    // The daemon logs "Scheduler started" only after its shutdown-signal
    // handlers are installed, so waiting for that line (instead of a
    // fixed sleep) makes it safe to signal even on a slow CI runner.
    daemon.wait_for_log("Scheduler started", Duration::from_secs(30));

    daemon.sigterm();
    let status = daemon.wait_for_exit(Duration::from_secs(10));
    let stderr = daemon.stderr();

    assert!(
        status.success(),
        "daemon exited with {:?}; stderr:\n{}",
        status,
        stderr
    );
    assert!(
        stderr.contains("Shutdown signal received"),
        "expected graceful shutdown log on stderr; got:\n{}",
        stderr
    );

    // Cleanup only after the assertions so a failure leaves the scratch
    // config behind for debugging.
    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn daemon_force_exits_on_second_sigterm_during_drain() {
    // An every-minute schedule plus a 1-second scheduler tick (test-only
    // env override) triggers the job right after startup, and the Wait
    // pre-backup hook keeps it in flight far longer than the test runs.
    // The secrets file is needed so the job resolves its repository
    // password and actually reaches the hook.
    let tmp = scratch_config(
        "force-exit",
        r#"
repositories:
  test:
    repo: /nonexistent/repo
    password_key: test_pass
jobs:
  slow:
    repository: test
    paths:
      - /nonexistent/path
    schedule: "* * * * *"
    pre_backup:
      - type: Wait
        seconds: 300
"#,
        Some("test_pass: dummy\n"),
    );

    let mut daemon = Daemon::spawn(&tmp, &[("RESTIC_MANAGER_TICK_SECS", "1")]);

    daemon.wait_for_log("Starting scheduled backup", Duration::from_secs(30));

    // First signal: enters the drain, which now waits on the in-flight
    // Wait hook.
    daemon.sigterm();
    daemon.wait_for_log("Shutdown signal received", Duration::from_secs(10));
    // Tiny grace so the drain's force-exit signal stream (registered just
    // after that log line) is armed before the second signal.
    std::thread::sleep(Duration::from_millis(300));

    // Second signal: force-exit with the conventional "interrupted" code
    // instead of waiting out the 300s hook.
    daemon.sigterm();
    let status = daemon.wait_for_exit(Duration::from_secs(10));
    let stderr = daemon.stderr();

    assert_eq!(
        status.code(),
        Some(130),
        "expected force-exit code 130; got {:?}; stderr:\n{}",
        status,
        stderr
    );
    assert!(
        stderr.contains("Second shutdown signal received"),
        "expected force-exit log on stderr; got:\n{}",
        stderr
    );

    std::fs::remove_dir_all(&tmp).ok();
}
