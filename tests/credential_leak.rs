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
//! All three must redact the credentials.
//!
//! The test does not need a real `restic`: it puts a stub `restic` on
//! PATH that reproduces the stderr real restic emits when a repository
//! with an embedded password is unreachable. That keeps the test
//! hermetic - no network, no restic install, which CI does not do
//! either - while still driving the real binary end to end.
//!
//! Unix-only: the stub is a shell script.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The password embedded in the repository URL below. It must never
/// appear in any output the user can see.
const SECRET: &str = "SECRETPASS";

/// The host part of the repository URL. It must survive redaction, so
/// the error message stays useful.
const HOST: &str = "127.0.0.1";

/// A repository URL that embeds credentials, exactly the shape restic
/// echoes back in its stderr.
fn credential_repo() -> String {
    format!("sftp:user:{SECRET}@{HOST}:1/nope")
}

/// Writes a stub `restic` that fails the way real restic fails when a
/// repository with embedded credentials is unreachable, and returns the
/// directory holding it.
///
/// The stub echoes the `--repo` URL it was given, so the credential in
/// the output comes from the URL under test rather than a hardcoded
/// string: if the URL shape changes, the test follows it.
fn write_restic_stub(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    let stub = dir.join("restic");
    std::fs::write(
        &stub,
        r#"#!/bin/sh
# Stand in for restic: fail the way it fails when a repository with an
# embedded password is unreachable, echoing the repository URL verbatim.
repo=""
prev=""
for arg in "$@"; do
    if [ "$prev" = "--repo" ]; then
        repo="$arg"
    fi
    prev="$arg"
done
echo "Fatal: unable to open repository at ${repo}: unable to start the sftp session, error: error receiving version packet from server: server unexpectedly closed connection: unexpected EOF" >&2
exit 1
"#,
    )
    .unwrap();

    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&stub, perms).unwrap();
}

/// Writes a scratch config tree and returns the `XDG_CONFIG_HOME` that
/// points at it, plus the directory holding the `restic` stub.
///
/// The `log_cli_output` path is set so the CLI writes its captured
/// restic output to a file inside the same tree - that file is the
/// on-disk channel this test also checks.
fn scratch_config(test_name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let tmp = std::env::temp_dir().join(format!(
        "restic-manager-credleak-{}-{}",
        test_name,
        std::process::id()
    ));
    let config_dir = tmp.join("restic-manager");
    std::fs::create_dir_all(&config_dir).unwrap();

    let stub_dir = tmp.join("stub-bin");
    write_restic_stub(&stub_dir);

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

    (tmp, stub_dir, log_path)
}

/// Runs `restic-manager check test` against the scratch config, with the
/// `restic` stub first on PATH, and returns its combined stdout+stderr.
fn run_check(tmp: &Path, stub_dir: &Path) -> String {
    // Put the stub ahead of any real restic on PATH so the test behaves
    // the same way whether or not restic is installed.
    let path = match std::env::join_paths(std::iter::once(stub_dir.to_path_buf()).chain(
        std::env::split_paths(&std::env::var("PATH").unwrap_or_default()),
    )) {
        Ok(p) => p,
        Err(_) => std::env::var("PATH").unwrap_or_default().into(),
    };

    let output = Command::new(env!("CARGO_BIN_EXE_restic-manager"))
        .arg("check")
        .arg("test")
        .env("XDG_CONFIG_HOME", tmp)
        .env("PATH", path)
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
    let (tmp, stub_dir, _log_path) = scratch_config("cli-error");

    let output = run_check(&tmp, &stub_dir);

    assert!(
        !output.contains(SECRET),
        "the repository password reached the CLI's error output:\n{output}"
    );
    // The rest of the URL must survive, or the message is useless.
    assert!(
        output.contains(HOST),
        "the error should still name the host:\n{output}"
    );

    std::fs::remove_dir_all(&tmp).ok();
}

#[test]
fn cli_output_log_file_does_not_leak_repository_password() {
    let (tmp, stub_dir, log_path) = scratch_config("cli-log");

    // The check must fail for the log file to be written at all.
    let _ = run_check(&tmp, &stub_dir);

    let log = std::fs::read_to_string(&log_path)
        .unwrap_or_else(|e| panic!("expected the CLI output log at {}: {e}", log_path.display()));

    assert!(
        !log.contains(SECRET),
        "the repository password reached the CLI output log:\n{log}"
    );
    assert!(
        log.contains(HOST),
        "the log should still name the host:\n{log}"
    );

    std::fs::remove_dir_all(&tmp).ok();
}
