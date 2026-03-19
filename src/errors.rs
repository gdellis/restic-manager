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

    #[error("Repository not initialized")]
    NotInitialized,

    #[error("Invalid password")]
    InvalidPassword,

    #[error("Repository locked")]
    Locked,

    #[error("Snapshot not found: {0}")]
    SnapshotNotFound(String),
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
