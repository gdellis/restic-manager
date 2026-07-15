//! End-to-end test for the daemon's graceful shutdown: SIGTERM (what
//! `systemctl stop` sends) must break the scheduler loop and exit cleanly,
//! not kill the process outright.
//!
//! Linux-only: it points the daemon at a scratch config via
//! XDG_CONFIG_HOME, which `dirs::config_dir()` only honors on Linux.
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[test]
fn daemon_exits_gracefully_on_sigterm() {
    let tmp = std::env::temp_dir().join(format!("restic-manager-sigterm-{}", std::process::id()));
    let config_dir = tmp.join("restic-manager");
    std::fs::create_dir_all(&config_dir).unwrap();

    // One scheduled job is required: with no schedules the daemon exits
    // immediately instead of idling. The 2099 date means it never fires.
    std::fs::write(
        config_dir.join("config.yaml"),
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
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_restic-manager"))
        .arg("daemon")
        .env("XDG_CONFIG_HOME", &tmp)
        .env("RUST_LOG", "info")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // Collect stderr on a reader thread so we can both watch for the
    // startup line while the daemon runs and assert on the full output
    // after it exits.
    let collected = Arc::new(Mutex::new(String::new()));
    let reader = {
        let collected = Arc::clone(&collected);
        let stderr = child.stderr.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                let mut buf = collected.lock().unwrap();
                buf.push_str(&line);
                buf.push('\n');
            }
        })
    };

    // The daemon logs "Scheduler started" only after its shutdown-signal
    // handlers are installed, so waiting for that line (instead of a
    // fixed sleep) makes it safe to signal even on a slow CI runner.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if collected.lock().unwrap().contains("Scheduler started") {
            break;
        }
        if child.try_wait().unwrap().is_some() || Instant::now() >= deadline {
            child.kill().ok();
            child.wait().ok();
            reader.join().unwrap();
            panic!(
                "daemon did not reach 'Scheduler started'; stderr:\n{}",
                collected.lock().unwrap()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let sigterm = Command::new("kill")
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(sigterm.success(), "failed to send SIGTERM");

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("daemon did not exit within 10s of SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    reader.join().unwrap();
    let stderr = collected.lock().unwrap().clone();

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
