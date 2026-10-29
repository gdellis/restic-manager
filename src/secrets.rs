use crate::errors::{SecretsError, SecretsError::NotFound};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TelegramConfig {
    pub bot_token: Option<String>,
    pub chat_id: Option<String>,
}

// Unlike the config.rs structs, this intentionally does NOT set
// #[serde(deny_unknown_fields)]: serde doesn't support combining
// deny_unknown_fields with #[serde(flatten)] (a known limitation - it's a
// compile error), and `values` needs to stay flattened to accept arbitrary
// user-defined keys.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Secrets {
    #[serde(flatten)]
    pub values: HashMap<String, String>,
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
}

impl Secrets {
    pub fn load() -> Result<Self, SecretsError> {
        let path = Self::path()?;
        if !path.exists() {
            return Err(NotFound(format!(
                "Secrets file not found at {}",
                path.display()
            )));
        }
        Self::warn_if_insecure_permissions(&path);
        let content = std::fs::read_to_string(&path)?;
        let secrets: Secrets = serde_yaml::from_str(&content)?;
        secrets.warn_if_telegram_incomplete();
        Ok(secrets)
    }

    pub fn load_optional() -> Result<Option<Self>, SecretsError> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(None);
        }
        Self::warn_if_insecure_permissions(&path);
        let content = std::fs::read_to_string(&path)?;
        let secrets: Secrets = serde_yaml::from_str(&content)?;
        secrets.warn_if_telegram_incomplete();
        Ok(Some(secrets))
    }

    fn path() -> Result<PathBuf, SecretsError> {
        Ok(crate::exclude::config_dir()?.join("secrets.yaml"))
    }

    /// A Unix permission `mode` (as returned by `PermissionsExt::mode()`) is
    /// insecure for a secrets file if it grants any access to group or
    /// others - i.e. it's more permissive than `0600`. This only inspects
    /// POSIX mode bits; it doesn't see ACLs or other access-control
    /// mechanisms that could grant broader access despite a strict mode.
    #[cfg(unix)]
    fn is_insecure_permissions(mode: u32) -> bool {
        mode & 0o077 != 0
    }

    /// A directory's mode is insecure for holding a secrets file if it's
    /// writable by group or others - that would let another user on the
    /// system replace, rename, or symlink-swap the secrets file itself,
    /// regardless of how strict the file's own permissions are.
    #[cfg(unix)]
    fn is_insecure_directory_permissions(mode: u32) -> bool {
        mode & 0o022 != 0
    }

    /// Warns (does not fail or modify anything) if the secrets file, or its
    /// parent directory, is more permissive than it should be, since the
    /// file holds repository passwords and the Telegram bot token in
    /// plaintext.
    #[cfg(unix)]
    fn warn_if_insecure_permissions(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        if let Ok(metadata) = std::fs::metadata(path) {
            let mode = metadata.permissions().mode() & 0o777;
            if Self::is_insecure_permissions(mode) {
                warn!(
                    path = %path.display(),
                    mode = format!("{:o}", mode),
                    "secrets file's POSIX permission bits are more permissive than 0600 \
                     (readable/writable by group or others); it contains plaintext repository \
                     passwords and should be chmod 600"
                );
            }
        }

        if let Some(parent) = path.parent() {
            if let Ok(metadata) = std::fs::metadata(parent) {
                let mode = metadata.permissions().mode() & 0o777;
                if Self::is_insecure_directory_permissions(mode) {
                    warn!(
                        path = %parent.display(),
                        mode = format!("{:o}", mode),
                        "secrets file's parent directory is writable by group or others; \
                         another user on this system could replace, rename, or symlink-swap \
                         the secrets file - consider chmod 700"
                    );
                }
            }
        }
    }

    #[cfg(not(unix))]
    fn warn_if_insecure_permissions(_path: &Path) {}

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|s| s.as_str())
    }

    pub fn telegram_config(&self) -> Option<&TelegramConfig> {
        self.telegram.as_ref()
    }

    /// Returns a warning message if a `telegram` block is present but
    /// carries no bot token.
    ///
    /// `TelegramConfig` can't use `deny_unknown_fields`: serde doesn't
    /// allow combining it with `flatten`, which `Secrets` needs for the
    /// arbitrary user-defined secret keys. The practical consequence is
    /// that a misspelled key (`bot_tokenn` instead of `bot_token`) is
    /// silently dropped: the config validates, the daemon starts, and
    /// alerting is quietly disabled with no indication why. Surfacing a
    /// warning when a telegram block exists without a token closes that
    /// gap.
    pub fn telegram_incomplete_warning(&self) -> Option<String> {
        let telegram = self.telegram.as_ref()?;
        if telegram
            .bot_token
            .as_deref()
            .map(str::trim)
            .is_some_and(|t| !t.is_empty())
        {
            return None;
        }
        Some(
            "secrets file contains a 'telegram' block without a usable 'bot_token'; \
             Telegram notifications are disabled (check for a misspelled key such as \
             'bot_tokenn')"
                .to_string(),
        )
    }

    /// Warn (without failing) if the secrets file has a telegram block
    /// but no bot token.
    fn warn_if_telegram_incomplete(&self) {
        if let Some(message) = self.telegram_incomplete_warning() {
            warn!("{}", message);
        }
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn test_misspelled_bot_token_is_reported_as_incomplete() {
        // Regression test: `TelegramConfig` can't use deny_unknown_fields
        // (serde forbids combining it with flatten, which `Secrets`
        // needs), so a misspelled key is silently dropped. Without a
        // warning the user gets a config that validates, a daemon that
        // runs, and no alerts at all.
        let yaml = "telegram:\n  bot_tokenn: \"123456:ABC\"\n  chat_id: \"42\"\n";
        let secrets: Secrets = serde_yaml::from_str(yaml).expect("secrets must parse");

        // Precondition: the typo is silently dropped by the parser.
        assert!(secrets.telegram.is_some());
        assert!(secrets.telegram_config().unwrap().bot_token.is_none());

        let warning = secrets
            .telegram_incomplete_warning()
            .expect("a telegram block without a bot token must be reported");
        assert!(
            warning.contains("bot_token"),
            "the warning should name the missing field: {warning}"
        );
    }

    #[test]
    fn test_complete_telegram_block_is_not_reported() {
        let yaml = "telegram:\n  bot_token: \"123456:ABC\"\n  chat_id: \"42\"\n";
        let secrets: Secrets = serde_yaml::from_str(yaml).expect("secrets must parse");
        assert!(secrets.telegram_incomplete_warning().is_none());
    }

    #[test]
    fn test_no_telegram_block_is_not_reported() {
        let yaml = "some-secret: value\n";
        let secrets: Secrets = serde_yaml::from_str(yaml).expect("secrets must parse");
        assert!(secrets.telegram_incomplete_warning().is_none());
    }

    #[test]
    fn test_empty_bot_token_is_reported_as_incomplete() {
        let yaml = "telegram:\n  bot_token: \"\"\n  chat_id: \"42\"\n";
        let secrets: Secrets = serde_yaml::from_str(yaml).expect("secrets must parse");
        assert!(secrets.telegram_incomplete_warning().is_some());
    }

    #[test]
    fn test_is_insecure_permissions_0600_is_secure() {
        assert!(!Secrets::is_insecure_permissions(0o600));
    }

    #[test]
    fn test_is_insecure_permissions_0400_is_secure() {
        assert!(!Secrets::is_insecure_permissions(0o400));
    }

    #[test]
    fn test_is_insecure_permissions_group_readable_is_insecure() {
        assert!(Secrets::is_insecure_permissions(0o640));
    }

    #[test]
    fn test_is_insecure_permissions_world_readable_is_insecure() {
        assert!(Secrets::is_insecure_permissions(0o644));
    }

    #[test]
    fn test_is_insecure_permissions_0777_is_insecure() {
        assert!(Secrets::is_insecure_permissions(0o777));
    }

    #[test]
    fn test_is_insecure_directory_permissions_0700_is_secure() {
        assert!(!Secrets::is_insecure_directory_permissions(0o700));
    }

    #[test]
    fn test_is_insecure_directory_permissions_group_readable_only_is_secure() {
        // Group/other read+execute (0o755) doesn't grant write access, so
        // it can't be used to replace the file - only the write bits matter.
        assert!(!Secrets::is_insecure_directory_permissions(0o755));
    }

    #[test]
    fn test_is_insecure_directory_permissions_group_writable_is_insecure() {
        assert!(Secrets::is_insecure_directory_permissions(0o770));
    }

    #[test]
    fn test_is_insecure_directory_permissions_world_writable_is_insecure() {
        assert!(Secrets::is_insecure_directory_permissions(0o777));
    }

    #[test]
    fn test_warn_if_insecure_permissions_does_not_panic_on_missing_file() {
        // Should silently do nothing for a path that doesn't exist, not
        // error or panic.
        Secrets::warn_if_insecure_permissions(std::path::Path::new(
            "/nonexistent/path/for/testing/secrets.yaml",
        ));
    }

    #[test]
    fn test_warn_if_insecure_permissions_handles_secure_and_insecure_files() {
        let dir = std::env::temp_dir().join(format!(
            "restic-manager-test-secrets-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secrets.yaml");
        std::fs::write(&path, "test: value").unwrap();

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        Secrets::warn_if_insecure_permissions(&path); // should not warn, just shouldn't panic

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        Secrets::warn_if_insecure_permissions(&path); // should warn, just shouldn't panic

        std::fs::remove_dir_all(&dir).ok();
    }
}
