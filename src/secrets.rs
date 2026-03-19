use crate::errors::{SecretsError, SecretsError::NotFound};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

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
        let content = std::fs::read_to_string(&path)?;
        let secrets: Secrets = serde_yaml::from_str(&content)?;
        Ok(secrets)
    }

    pub fn load_optional() -> Result<Option<Self>, SecretsError> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)?;
        let secrets: Secrets = serde_yaml::from_str(&content)?;
        Ok(Some(secrets))
    }

    fn path() -> Result<PathBuf, SecretsError> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| SecretsError::NotFound("Cannot find config directory".into()))?;
        Ok(config_dir.join("restic-manager").join("secrets.yaml"))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(|s| s.as_str())
    }

    pub fn telegram_config(&self) -> Option<&TelegramConfig> {
        self.telegram.as_ref()
    }
}
