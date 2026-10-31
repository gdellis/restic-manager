//! Regression tests for the TUI's data loading.
//!
//! The TUI used to spawn `restic-manager` subprocesses to read its data.
//! Two defects came out of that:
//!
//! - H2: the config was loaded strictly *before* the TUI was dispatched,
//!   so a single repository with a missing secret stopped the dashboard
//!   from starting at all - one broken repository hid every other job.
//! - H3: the TUI shared the terminal with the child's stderr, so a
//!   warning raised while loading spliced into the rendered widgets.
//!
//! These tests cover the library-level contract that fixed both.

use restic_manager::config::ResolvedConfig;
use std::io::Write;
use std::sync::Mutex;

/// Serializes tests that mutate `XDG_CONFIG_HOME` so they don't race.
static CONFIG_DIR_LOCK: Mutex<()> = Mutex::new(());

/// Writes a config with two repositories - one whose secret exists and one
/// whose secret is missing - and returns the config directory.
fn write_config_with_one_broken_repo(config_dir: &std::path::Path) {
    let config_yaml = r#"
repositories:
  good:
    repo: /backup/good
    password_key: good-password
  broken:
    repo: /backup/broken
    password_key: missing-password
jobs:
  documents:
    repository: good
    paths:
      - /home
  photos:
    repository: broken
    paths:
      - /photos
"#;
    let mut config_file = std::fs::File::create(config_dir.join("config.yaml")).unwrap();
    config_file.write_all(config_yaml.as_bytes()).unwrap();

    let mut secrets_file = std::fs::File::create(config_dir.join("secrets.yaml")).unwrap();
    secrets_file.write_all(b"good-password: hunter2\n").unwrap();
}

/// Runs `f` with `XDG_CONFIG_HOME` pointed at a temporary directory
/// containing the config written by `write_config_with_one_broken_repo`.
fn with_broken_repo_config<T>(f: impl FnOnce() -> T) -> T {
    let _guard = CONFIG_DIR_LOCK.lock().unwrap();
    let tmp = std::env::temp_dir().join(format!(
        "restic-manager-test-partial-config-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let config_dir = tmp.join("restic-manager");
    std::fs::create_dir_all(&config_dir).unwrap();
    write_config_with_one_broken_repo(&config_dir);

    let old_xdg = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("XDG_CONFIG_HOME", &tmp);
    let result = f();
    match old_xdg {
        Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    std::fs::remove_dir_all(&tmp).ok();
    result
}

/// The strict loader still refuses a config with a missing secret: the
/// one-shot CLI commands need every repository to be usable.
#[test]
fn test_strict_load_still_fails_with_one_broken_repo() {
    with_broken_repo_config(|| {
        let result = ResolvedConfig::load();
        let err = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("Expected ResolvedConfig::load to fail for a missing secret"),
        };
        assert!(
            err.contains("missing-password"),
            "error should name the missing secret key, got: {err}"
        );
    });
}

/// The lenient loader succeeds and keeps the healthy repository usable, so
/// one broken repository cannot hide every other job from the TUI.
#[test]
fn test_partial_load_keeps_healthy_repo_with_one_broken_repo() {
    with_broken_repo_config(|| {
        let (resolved, problems) = ResolvedConfig::load_partial().unwrap();

        // Both jobs are visible.
        assert_eq!(resolved.config.list_jobs().len(), 2);
        assert_eq!(resolved.config.list_repositories().len(), 2);

        // The healthy repository still resolves to a password.
        let (_job, _repo, password) = resolved.resolve_job("documents").unwrap();
        assert_eq!(password, "hunter2");

        // The broken one is reported, not silently dropped.
        assert!(
            problems
                .iter()
                .any(|p| p.contains("broken") && p.contains("missing-password")),
            "problems should name the broken repository, got: {problems:?}"
        );
    });
}

/// The TUI must not abort when the config has a broken repository: it
/// loads leniently and reports the problem in the UI instead.
///
/// This is the H2 regression. Before the fix, `cli_run` loaded the config
/// strictly before dispatching to the TUI, so `restic-manager tui` exited
/// with a config error and never started the dashboard.
#[cfg(target_os = "linux")]
#[test]
fn test_tui_starts_with_one_broken_repo() {
    use std::process::{Command, Stdio};

    let _guard = CONFIG_DIR_LOCK.lock().unwrap();
    let tmp = std::env::temp_dir().join(format!(
        "restic-manager-test-tui-broken-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let config_dir = tmp.join("restic-manager");
    std::fs::create_dir_all(&config_dir).unwrap();
    write_config_with_one_broken_repo(&config_dir);

    // The TUI needs a terminal, which a test does not have, so the point
    // is not that it renders: it is that it gets far enough to try. A
    // strict config load fails before the terminal is set up and exits
    // non-zero with a config error; a lenient load reaches the terminal
    // setup and fails there instead, with a different error.
    let output = Command::new(env!("CARGO_BIN_EXE_restic-manager"))
        .arg("tui")
        .env("XDG_CONFIG_HOME", &tmp)
        .env("TERM", "dumb")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("missing-password"),
        "the TUI must not fail on the broken repository, got: {stderr}"
    );

    std::fs::remove_dir_all(&tmp).ok();
}
