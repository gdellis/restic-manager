use std::path::Path;
use std::process::Output;
use tracing::warn;

const MAX_CLI_LOG_ENTRY_LEN: usize = 1_048_576;

/// Append a run's captured stdout and stderr to `log_path`, creating
/// parent directories as needed. Each call adds one `=== <timestamp> ===`
/// block so multiple runs don't blur together.
///
/// `stdout_lines` is the list of stdout lines the caller has already
/// collected; `stderr` is the optional stderr text. Failures are logged
/// but never propagated, since the original restic invocation already
/// finished and a logging failure shouldn't shadow its result.
pub fn write_cli_output_log(log_path: &Path, stdout_lines: &[String], stderr: Option<&str>) {
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut entry = format!(
        "=== {} ===\n{}\n",
        chrono::Local::now(),
        stdout_lines.join("\n")
    );
    if let Some(stderr) = stderr.filter(|s| !s.is_empty()) {
        entry.push_str("--- stderr ---\n");
        entry.push_str(stderr);
        entry.push('\n');
    }
    let entry = truncate_cli_log_entry(&entry);

    use std::io::Write;
    if let Err(e) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .and_then(|mut f| f.write_all(entry.as_bytes()))
    {
        warn!(
            "Failed to write CLI output log to {}: {}",
            log_path.display(),
            e
        );
    }
}

/// Convenience: split a captured `std::process::Output` into the
/// `Vec<String>` / `Option<&str>` shape `write_cli_output_log` wants,
/// then call it. No-op when `log_path` is `None`. Used by `repository`
/// and `snapshot` so each module doesn't have to re-implement the split.
pub fn write_command_output(log_path: Option<&Path>, output: &Output) {
    let Some(path) = log_path else { return };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<String> = stdout.lines().map(String::from).collect();
    let stderr_str = if stderr.is_empty() {
        None
    } else {
        Some(stderr.into_owned())
    };
    write_cli_output_log(path, &lines, stderr_str.as_deref());
}

fn truncate_cli_log_entry(entry: &str) -> String {
    if entry.len() <= MAX_CLI_LOG_ENTRY_LEN {
        return entry.to_string();
    }
    let suffix = "\n... (truncated)\n";
    let mut end = MAX_CLI_LOG_ENTRY_LEN.saturating_sub(suffix.len());
    while end > 0 && !entry.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &entry[..end], suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "restic-manager-test-cli-log-{}-{}-{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn truncate_under_limit_is_unchanged() {
        let entry = "a".repeat(100);
        assert_eq!(truncate_cli_log_entry(&entry), entry);
    }

    #[test]
    fn truncate_over_limit_is_truncated() {
        let entry = "a".repeat(MAX_CLI_LOG_ENTRY_LEN + 500);
        let result = truncate_cli_log_entry(&entry);
        assert!(result.len() <= MAX_CLI_LOG_ENTRY_LEN);
        assert!(result.ends_with("(truncated)\n"));
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let entry = "é".repeat(MAX_CLI_LOG_ENTRY_LEN);
        let result = truncate_cli_log_entry(&entry);
        assert!(result.len() <= MAX_CLI_LOG_ENTRY_LEN);
    }

    #[test]
    fn write_cli_output_log_appends_across_calls() {
        let dir = unique_temp_path("append");
        let log_path = dir.join("nested").join("backup.log");

        write_cli_output_log(&log_path, &["first run line".to_string()], None);
        write_cli_output_log(
            &log_path,
            &["second run line".to_string()],
            Some("a stderr warning"),
        );

        let contents = std::fs::read_to_string(&log_path).unwrap();
        assert!(contents.contains("first run line"));
        assert!(contents.contains("second run line"));
        assert!(contents.contains("a stderr warning"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
