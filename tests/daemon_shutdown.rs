//! End-to-end test for the daemon's graceful shutdown: SIGTERM (what
//! `systemctl stop` sends) must break the scheduler loop and exit cleanly,
//! not kill the process outright.
//!
//! Linux-only: it points the daemon at a scratch config via
//! XDG_CONFIG_HOME, which `dirs::config_dir()` only honors on Linux.
#![cfg(target_os = "linux")]

use std::io::Read;
use std::process::{Command, Stdio};
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

    // Give the daemon time to install its signal handlers and start the
    // scheduler loop before we signal it.
    std::thread::sleep(Duration::from_secs(2));

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

    let mut stderr = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();

    std::fs::remove_dir_all(&tmp).ok();

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
}
