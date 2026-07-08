use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse YAML: {0}")]
    ParseError(#[from] serde_yaml::Error),

    #[error("Config file not found: {0}")]
    NotFound(String),

    #[error("Invalid configuration: {0}")]
    Invalid(String),

    #[error("Missing required field: {0}")]
    MissingField(String),
}

#[derive(Error, Debug)]
pub enum SecretsError {
    #[error("Failed to read secrets file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse YAML: {0}")]
    ParseError(#[from] serde_yaml::Error),

    #[error("Secrets file not found: {0}")]
    NotFound(String),

    #[error("Missing secret key: {0}")]
    MissingKey(String),
}

#[derive(Error, Debug)]
pub enum ResticError {
    #[error("Restic command failed: {0}")]
    CommandFailed(String),

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
