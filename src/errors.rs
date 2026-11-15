use crate::redact::RedactedText;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file '{}': {source}", path.display())]
    IoError {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse YAML '{}': {source}", path.display())]
    ParseError {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("Config file not found: {0}")]
    NotFound(String),

    #[error("Invalid configuration: {0}")]
    Invalid(String),

    #[error("Missing required field: {0}")]
    MissingField(String),
}

#[derive(Error, Debug)]
pub enum SecretsError {
    #[error("Failed to read secrets file '{}': {source}", path.display())]
    IoError {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse YAML '{}': {source}", path.display())]
    ParseError {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("Secrets file not found: {0}")]
    NotFound(String),

    #[error("Missing secret key: {0}")]
    MissingKey(String),
}

#[derive(Error, Debug)]
pub enum ResticError {
    /// Restic echoes the repository URL verbatim in its stderr, and that
    /// URL can carry credentials (`sftp:user:password@host:/path`). The
    /// payload is a [`RedactedText`], so the credentials are stripped
    /// before the error exists - every caller is covered, including the
    /// ones that only wanted a nicer message.
    #[error("Restic command failed: {0}")]
    CommandFailed(RedactedText),

    #[error("Restic not found in PATH")]
    NotFound,

    #[error("Failed to run restic: {0}")]
    Io(#[source] std::io::Error),

    #[error("Repository not initialized")]
    NotInitialized,

    #[error("Invalid password")]
    InvalidPassword,

    #[error("Repository locked")]
    Locked,

    #[error("Snapshot not found: {0}")]
    SnapshotNotFound(String),
}

impl ResticError {
    /// Classifies an `io::Error` from spawning or waiting on a `restic`
    /// child process: only a genuine "binary not found" error (ENOENT)
    /// becomes `NotFound`, everything else (permission denied, resource
    /// exhaustion, interrupted syscalls, etc.) is preserved as `Io` so the
    /// real cause isn't hidden behind a misleading "not found" message.
    pub fn from_io(err: std::io::Error) -> Self {
        if err.kind() == std::io::ErrorKind::NotFound {
            ResticError::NotFound
        } else {
            ResticError::Io(err)
        }
    }
}

#[derive(Error, Debug)]
pub enum NotificationError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("Telegram not configured")]
    NotConfigured,

    #[error("Failed to send message: {0}")]
    SendFailed(String),
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Secrets error: {0}")]
    Secrets(#[from] SecretsError),

    #[error("Restic error: {0}")]
    Restic(#[from] ResticError),

    #[error("Notification error: {0}")]
    Notification(#[from] NotificationError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error, ErrorKind};
    use std::path::PathBuf;

    /// M8: config IO and parse errors must include the file path so the
    /// user knows which file failed.
    #[test]
    fn test_config_error_includes_path() {
        let path = PathBuf::from("/some/config.yaml");
        let io_err = ConfigError::IoError {
            path: path.clone(),
            source: Error::new(ErrorKind::PermissionDenied, "denied"),
        };
        let msg = io_err.to_string();
        assert!(msg.contains("/some/config.yaml"), "{}", msg);
        assert!(msg.contains("denied"), "{}", msg);

        let parse_err = ConfigError::ParseError {
            path: path.clone(),
            source: serde_yaml::from_str::<i32>(":").unwrap_err(),
        };
        let msg = parse_err.to_string();
        assert!(msg.contains("/some/config.yaml"), "{}", msg);
    }

    /// M8: secrets IO and parse errors must include the file path.
    #[test]
    fn test_secrets_error_includes_path() {
        let path = PathBuf::from("/some/secrets.yaml");
        let io_err = SecretsError::IoError {
            path: path.clone(),
            source: Error::new(ErrorKind::NotFound, "not found"),
        };
        let msg = io_err.to_string();
        assert!(msg.contains("/some/secrets.yaml"), "{}", msg);
        assert!(msg.contains("not found"), "{}", msg);

        let parse_err = SecretsError::ParseError {
            path,
            source: serde_yaml::from_str::<i32>("{").unwrap_err(),
        };
        let msg = parse_err.to_string();
        assert!(msg.contains("/some/secrets.yaml"), "{}", msg);
    }

    #[test]
    fn test_from_io_not_found_becomes_not_found_variant() {
        let err = Error::new(ErrorKind::NotFound, "No such file or directory");
        assert!(matches!(ResticError::from_io(err), ResticError::NotFound));
    }

    #[test]
    fn test_from_io_permission_denied_preserves_error() {
        let err = Error::new(ErrorKind::PermissionDenied, "Permission denied");
        match ResticError::from_io(err) {
            ResticError::Io(inner) => assert_eq!(inner.kind(), ErrorKind::PermissionDenied),
            other => panic!("expected ResticError::Io, got {other:?}"),
        }
    }

    #[test]
    fn test_from_io_other_kind_preserves_error() {
        let err = Error::new(ErrorKind::Interrupted, "signal received");
        match ResticError::from_io(err) {
            ResticError::Io(inner) => assert_eq!(inner.kind(), ErrorKind::Interrupted),
            other => panic!("expected ResticError::Io, got {other:?}"),
        }
    }
}
