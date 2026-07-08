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
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

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
