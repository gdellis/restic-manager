//! Shared restic runner: every `restic` invocation goes through `run_restic()`
//! (FIX_PLAN.md PR-1.1), which captures the child's output through #112's
//! tracked-child seam (`restic_child::capture_output`, so pidfile/SIGTERM
//! tracking still sees every child) and maps restic's documented exit codes
//! onto typed `ResticError` variants. The captured `Output` is returned to
//! callers on the known partial-success codes (`forget`/`backup` exit 3) so
//! they can parse stdout and continue.
//!
//! `log_path` preserves each call site's pre-existing `log_cli_output` wiring:
//! call sites that logged before PR-1.1 pass `Some(path)` (written in the
//! shared `cli_log::write_command_output` format), the others pass `None`.
//!
//! Hard bounds (later PRs own these): no watchdog/timeout kill and no bounded
//! output capture (PR-1.2); no credential redaction (PR-6.1).

use crate::cli_log::write_command_output;
use crate::errors::ResticError;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};

/// Runs `restic <args...>` with `RESTIC_PASSWORD` set, returning the captured
/// `Output` on any exit code (including the known partial-success ones).
///
/// Exit codes are classified into typed errors:
///
/// - 10 -> `ResticError::NotInitialized`
/// - 11 -> `ResticError::Locked`
/// - 12 -> `ResticError::InvalidPassword`
/// - 3  -> `ResticError::Partial` (partial success: e.g. `forget` removed only
///   some snapshots, or `backup` completed with some files unreadable). The
///   captured output rides along so callers can parse it and continue.
/// - anything else -> `ResticError::CommandFailed`
///
/// When stderr is empty the error text falls back to the exit status plus a
/// tail of stdout. `CommandFailed`'s message always carries the exit status.
pub fn run_restic<S: AsRef<OsStr>>(
    args: &[S],
    password: &str,
    log_path: Option<&Path>,
) -> Result<Output, ResticError> {
    let mut cmd = Command::new("restic");
    cmd.args(args).env("RESTIC_PASSWORD", password);
    let output = crate::restic_child::capture_output(&mut cmd).map_err(ResticError::from_io)?;

    write_command_output(log_path, &output);

    map_exit(output.status.code(), output)
}

/// Maps a restic exit code onto a typed error. Code 3 (partial success) and
/// code 0 return the captured `Output` to the caller.
fn map_exit(code: Option<i32>, output: Output) -> Result<Output, ResticError> {
    match code {
        Some(0) => Ok(output),
        Some(3) => Err(ResticError::Partial {
            detail: exit_status_text(code),
            output,
        }),
        Some(10) => Err(ResticError::NotInitialized),
        Some(11) => Err(ResticError::Locked),
        Some(12) => Err(ResticError::InvalidPassword),
        _ => Err(command_failed(code, &output)),
    }
}

/// Builds the generic failure error: stderr when it carries a message,
/// otherwise the exit status plus a tail of stdout.
fn command_failed(code: Option<i32>, output: &Output) -> ResticError {
    let status = exit_status_text(code);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let text = stderr.trim();
    if text.is_empty() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        ResticError::CommandFailed(format!(
            "{} (stdout tail: {})",
            status,
            output_tail(&stdout)
        ))
    } else {
        ResticError::CommandFailed(format!("{}: {}", status, text))
    }
}

/// Human-readable exit status, e.g. `exit status 10` or `signal 9 (SIGKILL)`.
fn exit_status_text(code: Option<i32>) -> String {
    match code {
        Some(code) => format!("exit status {}", code),
        None => "unknown exit status (process terminated by signal)".to_string(),
    }
}

/// Last 200 chars of stdout (or its tail), normalized to one line.
fn output_tail(stdout: &str) -> String {
    let trimmed = stdout.trim();
    let tail: String = if trimmed.chars().count() > 200 {
        trimmed
            .chars()
            .skip(trimmed.chars().count() - 200)
            .collect()
    } else {
        trimmed.to_string()
    };
    tail.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Output;

    fn fake_output(code: i32, stdout: &str, stderr: &str) -> Output {
        #[allow(unused_imports)]
        use std::os::unix::process::ExitStatusExt;
        use std::process::ExitStatus;
        let status = ExitStatus::from_raw(code << 8);
        Output {
            status,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn map_exit_zero_returns_output() {
        let output = fake_output(0, "[]", "");
        assert!(map_exit(Some(0), output).is_ok());
    }

    #[test]
    fn map_exit_ten_is_not_initialized() {
        let output = fake_output(10, "", "");
        assert!(matches!(
            map_exit(Some(10), output),
            Err(ResticError::NotInitialized)
        ));
    }

    #[test]
    fn map_exit_eleven_is_locked() {
        let output = fake_output(11, "", "");
        assert!(matches!(
            map_exit(Some(11), output),
            Err(ResticError::Locked)
        ));
    }

    #[test]
    fn map_exit_twelve_is_invalid_password() {
        let output = fake_output(12, "", "");
        assert!(matches!(
            map_exit(Some(12), output),
            Err(ResticError::InvalidPassword)
        ));
    }

    #[test]
    fn map_exit_three_is_partial_with_output() {
        let output = fake_output(3, "[{\"remove\":[]}]", "");
        match map_exit(Some(3), output) {
            Err(ResticError::Partial { detail, output }) => {
                assert_eq!(detail, "exit status 3");
                assert_eq!(String::from_utf8_lossy(&output.stdout), "[{\"remove\":[]}]");
            }
            other => panic!("expected Partial, got {:?}", other.map(|_| "ok")),
        }
    }

    #[test]
    fn map_exit_other_is_command_failed_with_status() {
        let output = fake_output(2, "", "boom");
        match map_exit(Some(2), output) {
            Err(ResticError::CommandFailed(msg)) => {
                assert!(msg.contains("exit status 2"), "msg: {}", msg);
                assert!(msg.contains("boom"), "msg: {}", msg);
            }
            other => panic!("expected CommandFailed, got {:?}", other.map(|_| "ok")),
        }
    }

    #[test]
    fn command_failed_empty_stderr_carries_exit_and_stdout_tail() {
        let output = fake_output(1, "failed: repository not reachable", "");
        match map_exit(Some(1), output) {
            Err(ResticError::CommandFailed(msg)) => {
                assert!(msg.contains("exit status 1"), "msg: {}", msg);
                assert!(
                    msg.contains("failed: repository not reachable"),
                    "msg: {}",
                    msg
                );
            }
            other => panic!("expected CommandFailed, got {:?}", other.map(|_| "ok")),
        }
    }

    #[test]
    fn output_tail_is_bounded() {
        let long = "x".repeat(300);
        let tail = output_tail(&long);
        assert_eq!(tail.chars().count(), 200);
        assert_eq!(output_tail("short"), "short");
        assert_eq!(output_tail("  a   b  "), "a b");
    }
}
