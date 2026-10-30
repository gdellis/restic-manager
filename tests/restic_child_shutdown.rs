//! End-to-end tests for the `restic` child-lifecycle contract.
//!
//! A `restic` child must never outlive the manager that spawned it.
//! `restic` holds a lock on the repository while it runs, so an orphaned
//! `restic` keeps the repository locked and the next run fails with
//! "repository is already locked".
//!
//! Both shutdown paths are covered:
//! - The one-shot `run` command, which has to terminate the child when it
//!   receives SIGTERM.
//! - The daemon's force-exit (a second signal during the drain), which
//!   abandons the in-flight backup.
//!
//! The tests put a `restic` shim first on PATH. The shim records its PID
//! and then sleeps, so the child stays in flight long enough to be
//! signalled, and the test can assert on whether it survived.
//!
//! Linux-only: they inspect /proc and deliver signals.
#![cfg(target_os = "linux")]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long to wait for the shim to record its PID.
const SHIM_START_TIMEOUT: Duration = Duration::from_secs(30);

/// How long to wait for the manager to exit after it was signalled.
const MANAGER_EXIT_TIMEOUT: Duration = Duration::from_secs(15);

/// How long to wait for the shim to disappear from /proc after the
/// manager exited.
const SHIM_EXIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Writes a `restic` replacement that records its PID and then sleeps.
///
/// `exec sleep` keeps the recorded PID valid: the shell process is
/// replaced in place, so the PID in the file is the PID of the process
/// that stays alive until it is killed.
fn write_restic_shim(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let shim = dir.join("restic");
    let mut file = std::fs::File::create(&shim).unwrap();
    file.write_all(b"#!/bin/sh\necho $$ > \"$RESTIC_SHIM_PIDFILE\"\nexec sleep 300\n")
        .unwrap();
    drop(file);
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Writes a scratch config directory and returns it as XDG_CONFIG_HOME.
///
/// The job points at a nonexistent repository: the `restic` shim stands in
/// for the real binary, so the repository is never actually touched.
fn scratch_config(test_name: &str) -> PathBuf {
    let tmp = std::env::temp_dir().join(format!(
        "restic-manager-{}-{}",
        test_name,
        std::process::id()
    ));
    let config_dir = tmp.join("restic-manager");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.yaml"),
        r#"
repositories:
  test:
    repo: /nonexistent/repo
    password_key: test_pass
jobs:
  backup:
    repository: test
    paths:
      - /nonexistent/path
"#,
    )
    .unwrap();
    std::fs::write(config_dir.join("secrets.yaml"), "test_pass: dummy\n").unwrap();
    tmp
}

/// Whether a process with this PID still exists.
fn process_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// Waits for a process to disappear, returning whether it did.
fn wait_for_process_to_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !process_exists(pid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Waits for the shim to record its PID, returning that PID.
fn wait_for_shim_pid(pid_file: &Path, timeout: Duration) -> u32 {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(text) = std::fs::read_to_string(pid_file) {
            if let Ok(pid) = text.trim().parse::<u32>() {
                if pid > 0 {
                    return pid;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "the restic shim never started; no PID file at {}",
            pid_file.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Sends SIGTERM to a process.
fn sigterm(pid: u32) {
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .unwrap();
    assert!(status.success(), "failed to send SIGTERM to {pid}");
}

/// Waits for a child to exit, killing it and panicking on timeout.
fn wait_for_exit(child: &mut Child, timeout: Duration, context: &str) -> i32 {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code().unwrap_or(-1);
        }
        if Instant::now() >= deadline {
            child.kill().ok();
            child.wait().ok();
            panic!("{context} did not exit within {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The environment every test needs: the scratch config home, the shim
/// first on PATH, and the file the shim records its PID in.
fn test_env(tmp: &Path, shim_dir: &Path, pid_file: &Path) -> Vec<(String, String)> {
    let path = format!(
        "{}:{}",
        shim_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    vec![
        ("XDG_CONFIG_HOME".to_string(), tmp.display().to_string()),
        ("PATH".to_string(), path),
        (
            "RESTIC_SHIM_PIDFILE".to_string(),
            pid_file.display().to_string(),
        ),
    ]
}

#[test]
fn one_shot_run_terminates_restic_child_on_sigterm() {
    let tmp = scratch_config("oneshot-sigterm");
    let shim_dir = tmp.join("shim");
    let pid_file = tmp.join("restic.pid");
    write_restic_shim(&shim_dir);

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_restic-manager"));
    cmd.arg("run")
        .arg("backup")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, value) in test_env(&tmp, &shim_dir, &pid_file) {
        cmd.env(key, value);
    }
    let mut manager = cmd.spawn().unwrap();

    // The shim records its PID before sleeping, so once the file exists
    // the manager has a live `restic` child.
    let shim_pid = wait_for_shim_pid(&pid_file, SHIM_START_TIMEOUT);
    assert!(
        process_exists(shim_pid),
        "the restic shim was not running when it recorded its PID"
    );

    // SIGTERM is what `systemctl stop` sends.
    sigterm(manager.id());

    let code = wait_for_exit(&mut manager, MANAGER_EXIT_TIMEOUT, "the one-shot run");
    assert_eq!(
        code, 130,
        "expected the interrupted exit code 130; got {code}"
    );

    assert!(
        wait_for_process_to_exit(shim_pid, SHIM_EXIT_TIMEOUT),
        "the restic child {shim_pid} outlived the manager and is still running"
    );

    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn daemon_force_exit_terminates_restic_child() {
    let tmp = scratch_config("daemon-force-exit");
    let config_dir = tmp.join("restic-manager");
    // An every-minute schedule plus a 1-second scheduler tick (test-only
    // env override) triggers the job right after startup.
    std::fs::write(
        config_dir.join("config.yaml"),
        r#"
repositories:
  test:
    repo: /nonexistent/repo
    password_key: test_pass
jobs:
  backup:
    repository: test
    paths:
      - /nonexistent/path
    schedule: "* * * * *"
"#,
    )
    .unwrap();
    let shim_dir = tmp.join("shim");
    let pid_file = tmp.join("restic.pid");
    write_restic_shim(&shim_dir);

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_restic-manager"));
    cmd.arg("daemon")
        .env("RESTIC_MANAGER_TEST_TICK_SECS", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (key, value) in test_env(&tmp, &shim_dir, &pid_file) {
        cmd.env(key, value);
    }
    let mut daemon = cmd.spawn().unwrap();

    // The daemon's backup spawns the shim, which then sleeps until it is
    // killed. The PID file appears only once that child is live.
    let shim_pid = wait_for_shim_pid(&pid_file, SHIM_START_TIMEOUT);
    assert!(
        process_exists(shim_pid),
        "the restic shim was not running when it recorded its PID"
    );

    // First signal: the daemon starts draining the in-flight backup.
    sigterm(daemon.id());
    // Give the drain a moment to arm its force-exit handler before the
    // second signal arrives.
    std::thread::sleep(Duration::from_millis(500));

    // Second signal: force-exit, abandoning the in-flight backup. The
    // restic child must not survive it.
    sigterm(daemon.id());

    let code = wait_for_exit(&mut daemon, MANAGER_EXIT_TIMEOUT, "the daemon");
    assert_eq!(code, 130, "expected the force-exit code 130; got {code}");

    assert!(
        wait_for_process_to_exit(shim_pid, SHIM_EXIT_TIMEOUT),
        "the restic child {shim_pid} outlived the daemon and is still running"
    );

    std::fs::remove_dir_all(&tmp).ok();
}
