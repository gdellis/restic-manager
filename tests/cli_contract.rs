//! CLI-level contract tests: they run the real `restic-manager` binary
//! against a scratch config and, where restic would be invoked, a stub
//! `restic` on PATH. Each test locks behavior that is externally
//! observable (exit codes, message wording) and names the planned PR
//! (see FIX_PLAN.md) that deliberately changes it, so the contract can
//! only flip as a conscious decision.
//!
//! Linux-only: the tests point the binary at a scratch config via
//! XDG_CONFIG_HOME, which `dirs::config_dir()` only honors on Linux
//! (same caveat as tests/daemon_shutdown.rs).
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::{Command, Output};

/// Exclusive-creates a unique scratch home with a `restic-manager/`
/// config dir and an empty `bin/` dir for the stub restic. `create_dir`
/// (not `create_dir_all`) fails if the path is already taken - by a
/// stale dir from a crashed run, or a symlink planted at that path by
/// another user - instead of writing through it. Callers clean up with
/// `remove_dir_all` after asserting.
fn scratch_home(label: &str) -> std::path::PathBuf {
    let tmp = std::env::temp_dir().join(format!(
        "restic-manager-test-cli-{}-{}-{}",
        label,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&tmp).expect("scratch dir path should not already exist");
    std::fs::create_dir(tmp.join("restic-manager")).unwrap();
    std::fs::create_dir(tmp.join("bin")).unwrap();
    tmp
}

/// Writes a minimal valid config.yaml + secrets.yaml into `config_dir`.
fn write_config_and_secrets(config_dir: &Path) {
    let config_yaml = r#"
repositories:
  test:
    repo: /nonexistent/repo
    password_key: test_pass
jobs:
  daily:
    repository: test
    paths:
      - /nonexistent/path
"#;
    std::fs::write(config_dir.join("config.yaml"), config_yaml).unwrap();
    std::fs::write(config_dir.join("secrets.yaml"), "test_pass: dummy\n").unwrap();
}

/// Writes a stub `restic` executable (a `/bin/sh` script) into `bin_dir`,
/// so the CLI's restic invocations can be driven without a real restic
/// install or a real repository.
fn write_stub_restic(bin_dir: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    let stub = bin_dir.join("restic");
    std::fs::write(&stub, script).unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Runs the built `restic-manager` binary against `home` as its
/// XDG_CONFIG_HOME, with `bin_dir` (which may contain the stub restic)
/// prepended to PATH so the child's `Command::new("restic")` resolves to
/// the stub.
fn run_cli(home: &Path, bin_dir: &Path, args: &[&str]) -> Output {
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new(env!("CARGO_BIN_EXE_restic-manager"))
        .args(args)
        .env("XDG_CONFIG_HOME", home)
        .env("PATH", path)
        .output()
        .expect("failed to spawn restic-manager")
}

/// CONTRACT LOCK (flips in PR-4.1): when restic exits 3 (the backup
/// completed but some source files were unreadable), the CLI currently
/// exits 0 even though it prints a "partial" message - an automated
/// caller watching only the exit code cannot tell a clean backup from a
/// partial one. PR-4.1 makes partial runs exit 3 and must update this
/// test's exit-code assertion deliberately.
#[test]
fn run_partial_backup_currently_exits_zero() {
    let tmp = scratch_home("partial-exit");
    write_config_and_secrets(&tmp.join("restic-manager"));
    write_stub_restic(
        &tmp.join("bin"),
        r#"#!/bin/sh
echo '{"message_type":"summary","summary":{"files_new":1,"files_changed":2,"files_unmodified":3,"dirs_new":0,"dirs_changed":0,"dirs_unmodified":0,"data_blobs":0,"tree_blobs":0,"data_added":0,"duration":0.1},"snapshot_id":"deadbeef"}'
echo "unable to read some files" >&2
exit 3
"#,
    );

    let output = run_cli(&tmp, &tmp.join("bin"), &["run", "daily"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(0),
        "CONTRACT: partial backups exit 0 today (flips to 3 in PR-4.1); stdout:\n{}\nstderr:\n{}",
        stdout,
        stderr
    );
    assert!(
        stdout.contains("Backup completed with errors (partial)"),
        "stdout should report the partial result; got:\n{}",
        stdout
    );
    assert!(
        stdout.contains("Snapshot ID: deadbeef"),
        "stdout should report the parsed snapshot ID; got:\n{}",
        stdout
    );

    // Cleanup only after the assertions so a failure leaves the scratch
    // dir behind for debugging.
    std::fs::remove_dir_all(&tmp).ok();
}

/// CONTRACT LOCK (flips in PR-7.2): `init-exclude` prints
/// "Created default exclude file at: ..." on every run, including the
/// second run below where the file already existed and nothing was
/// created - a known wording defect. PR-7.2 replaces that with honest
/// wording and must update the second-run assertion deliberately.
/// The non-clobbering half of the contract lives in
/// `init_exclude_never_clobbers_existing_file` below and must NOT flip.
#[test]
fn init_exclude_reports_created_even_when_file_already_exists() {
    let tmp = scratch_home("init-exclude");
    let config_dir = tmp.join("restic-manager");
    // A valid config + secrets is required even for init-exclude: the CLI
    // loads the full ResolvedConfig before dispatching any subcommand.
    // That requirement is itself a known quirk (planned for the bootstrap
    // PR), but it is not the contract under test here.
    write_config_and_secrets(&config_dir);
    let bin_dir = tmp.join("bin");

    let first = run_cli(&tmp, &bin_dir, &["init-exclude"]);
    let first_stdout = String::from_utf8_lossy(&first.stdout);
    assert_eq!(
        first.status.code(),
        Some(0),
        "first init-exclude run failed; stdout:\n{}",
        first_stdout
    );
    assert!(
        first_stdout.contains("Created default exclude file at: "),
        "stdout should report the created exclude file; got:\n{}",
        first_stdout
    );

    // Customize the file, then re-run: the message must still say
    // "Created ..." even though nothing was created. That the content
    // survives is asserted by init_exclude_never_clobbers_existing_file.
    let exclude_file = config_dir.join("excludes.txt");
    std::fs::write(&exclude_file, "# my custom excludes\n").unwrap();

    let second = run_cli(&tmp, &bin_dir, &["init-exclude"]);
    let second_stdout = String::from_utf8_lossy(&second.stdout);
    assert_eq!(
        second.status.code(),
        Some(0),
        "second init-exclude run failed; stdout:\n{}",
        second_stdout
    );
    assert!(
        second_stdout.contains("Created default exclude file at: "),
        "CONTRACT: the misleading 'Created' wording is locked deliberately (flips in PR-7.2); got:\n{}",
        second_stdout
    );

    // Cleanup only after the assertions so a failure leaves the scratch
    // dir behind for debugging.
    std::fs::remove_dir_all(&tmp).ok();
}

/// INVARIANT (never flips): `init-exclude` must not clobber an existing
/// exclude file - a customized file keeps its content on re-run. This is
/// correct today and must stay correct no matter how the wording contract
/// in `init_exclude_reports_created_even_when_file_already_exists` flips
/// in PR-7.2; it lives in its own test so that flip cannot edit it away.
#[test]
fn init_exclude_never_clobbers_existing_file() {
    let tmp = scratch_home("init-exclude-no-clobber");
    let config_dir = tmp.join("restic-manager");
    write_config_and_secrets(&config_dir);
    let bin_dir = tmp.join("bin");

    let first = run_cli(&tmp, &bin_dir, &["init-exclude"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "first init-exclude run failed; stdout:\n{}",
        String::from_utf8_lossy(&first.stdout)
    );

    let exclude_file = config_dir.join("excludes.txt");
    std::fs::write(&exclude_file, "# my custom excludes\n").unwrap();

    let second = run_cli(&tmp, &bin_dir, &["init-exclude"]);
    assert_eq!(
        second.status.code(),
        Some(0),
        "second init-exclude run failed; stdout:\n{}",
        String::from_utf8_lossy(&second.stdout)
    );

    let content = std::fs::read_to_string(&exclude_file).unwrap();
    assert!(
        content.contains("# my custom excludes"),
        "init-exclude must not clobber an existing exclude file; got:\n{}",
        content
    );

    std::fs::remove_dir_all(&tmp).ok();
}
