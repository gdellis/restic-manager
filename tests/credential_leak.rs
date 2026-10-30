//! Regression test for the credential leak through the CLI's error output.
//!
//! Restic echoes the repository URL - including the password, when the
//! URL carries one - in its stderr. That stderr reaches the user
//! through three channels:
//!
//! 1. the error the CLI prints when a command fails,
//! 2. the `log_cli_output` file written to disk, and
//! 3. the Telegram chat, when the failure happens inside the daemon.
//!
//! All three must redact the credentials. This test drives the real
//! binary against an unreachable repository whose URL embeds a
//! password, and asserts the password appears in none of them.
//!
//! The repository URL is a real `sftp:` URL pointing at a closed port,
//! so restic fails fast and its stderr carries the URL back verbatim.
//! `restic` must be on PATH (see AGENTS.md).
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The password embedded in the repository URL below. It must never
/// appear in any output the user can see.
const SECRET: &str = "SECRETPASS";

/// A repository URL that embeds credentials, exactly the shape restic
/// echoes back in its stderr.
fn credential_repo() -> String {
    format!("sftp:user:{SECRET}@127.0.0.1:1/nope")
}

/// Writes a scratch config tree and returns the `XDG_CONFIG_HOME` that
/// points at it.
///
/// The `log_cli_output` path is set so the CLI writes its captured
/// restic output to a file inside the same tree - that file is the
/// on-disk channel this test also checks.
fn scratch_config(test_name: &str) -> (PathBuf, PathBuf) {
    let tmp = std::env::temp_dir().join(format!(
        "restic-manager-credleak-{}-{}",
        test_name,
        std::process::id()
    ));
    let config_dir = tmp.join("restic-manager");
    std::fs::create_dir_all(&config_dir).unwrap();

    let log_path = tmp.join("cli-output.log");
    let config = format!(
        "repositories:\n  \
         test:\n    \
         repo: {}\n    \
         password_key: test_pass\n    \
         log_cli_output: {}\n",
        credential_repo(),
        log_path.display()
    );
    std::fs::write(config_dir.join("config.yaml"), config).unwrap();
    std::fs::write(config_dir.join("secrets.yaml"), "test_pass: dummy\n").unwrap();

    (tmp, log_path)
}

/// Runs `restic-manager check test` against the scratch config and
/// returns its combined stdout+stderr.
fn run_check(tmp: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_restic-manager"))
        .arg("check")
        .arg("test")
        .env("XDG_CONFIG_HOME", tmp)
        .output()
        .unwrap();

    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn cli_error_output_does_not_leak_repository_password() {
    let (tmp, _log_path) = scratch_config("cli-error");

    let output = run_check(&tmp);

    assert!(
        !output.contains(SECRET),
        "the repository password reached the CLI's error output:\n{output}"
    );
    // The rest of the URL must survive, or the message is useless.
    assert!(
        output.contains("127.0.0.1"),
        "the error should still name the host:\n{output}"
    );

    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn cli_output_log_file_does_not_leak_repository_password() {
    let (tmp, log_path) = scratch_config("cli-log");

    // The check must fail for the log file to be written at all.
    let _ = run_check(&tmp);

    let log = std::fs::read_to_string(&log_path)
        .unwrap_or_else(|e| panic!("expected the CLI output log at {}: {e}", log_path.display()));

    assert!(
        !log.contains(SECRET),
        "the repository password reached the CLI output log:\n{log}"
    );
    assert!(
        log.contains("127.0.0.1"),
        "the log should still name the host:\n{log}"
    );

    std::fs::remove_dir_all(&tmp).ok();
}
