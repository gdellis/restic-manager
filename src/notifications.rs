use crate::config::{NotificationConfig, ResolvedConfig};
use crate::errors::{AppError, NotificationError};
use reqwest::Client;
use std::sync::Mutex;
use std::time::Duration;
use tracing::{info, warn};

pub struct Notifications {
    client: Client,
    bot_token: Option<String>,
    chat_id: Option<String>,
    rate_limiter: Mutex<RateLimiter>,
}

struct RateLimiter {
    last_failure_notification: Option<std::time::Instant>,
    last_success_notification: Option<std::time::Instant>,
}

const MIN_NOTIFICATION_INTERVAL: Duration = Duration::from_secs(300);

/// Telegram's hard limit on a sendMessage `text` field.
const TELEGRAM_MAX_MESSAGE_LEN: usize = 4096;

impl Notifications {
    pub fn new(config: &ResolvedConfig) -> Self {
        let telegram = config.secrets.telegram_config();

        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| Client::new()),
            bot_token: telegram.and_then(|t| t.bot_token.clone()),
            chat_id: telegram.and_then(|t| t.chat_id.clone()),
            rate_limiter: Mutex::new(RateLimiter {
                last_failure_notification: None,
                last_success_notification: None,
            }),
        }
    }

    pub fn is_configured(&self) -> bool {
        self.bot_token.is_some() && self.chat_id.is_some()
    }

    pub async fn send_failure(&self, job_name: &str, error: &str) -> Result<(), AppError> {
        if !self.is_configured() {
            return Ok(());
        }

        if self.is_rate_limited(true) {
            warn!(job = job_name, "Failure notification rate limited");
            return Ok(());
        }

        let message = format!("❌ Backup Failed: {}\n\nError: {}", job_name, error);

        self.send_telegram(&message).await?;
        self.record_notification_sent(true);
        info!(job = job_name, "Failure notification sent");
        Ok(())
    }

    pub async fn send_partial(
        &self,
        job_name: &str,
        snapshot_id: Option<&str>,
        error: &str,
    ) -> Result<(), AppError> {
        if !self.is_configured() {
            return Ok(());
        }

        // Shares the failure rate-limit slot so a partial backup and a hard
        // failure in the same window don't double alert volume.
        if self.is_rate_limited(true) {
            warn!(job = job_name, "Partial notification rate limited");
            return Ok(());
        }

        let snap = snapshot_id.unwrap_or("none");
        let message = format!(
            "⚠️ Backup Partial: {}\n\nSnapshot: {}\nSome files could not be read: {}",
            job_name, snap, error
        );

        self.send_telegram(&message).await?;
        self.record_notification_sent(true);
        info!(job = job_name, "Partial notification sent");
        Ok(())
    }

    pub async fn send_success(
        &self,
        job_name: &str,
        snapshot_id: Option<&str>,
    ) -> Result<(), AppError> {
        if !self.is_configured() {
            return Ok(());
        }

        if self.is_rate_limited(false) {
            return Ok(());
        }

        let message = if let Some(snap) = snapshot_id {
            format!("✅ Backup Success: {}\n\nSnapshot: {}", job_name, snap)
        } else {
            format!("✅ Backup Success: {}", job_name)
        };

        self.send_telegram(&message).await?;
        self.record_notification_sent(false);
        info!(job = job_name, "Success notification sent");
        Ok(())
    }

    /// Returns true if a notification of this kind was sent within
    /// `MIN_NOTIFICATION_INTERVAL`. Read-only - callers must call
    /// `record_notification_sent` themselves after a successful send, so a
    /// failed send doesn't consume the rate-limit window.
    fn is_rate_limited(&self, is_failure: bool) -> bool {
        let limiter = self.rate_limiter.lock().unwrap();
        let last_notification = if is_failure {
            limiter.last_failure_notification
        } else {
            limiter.last_success_notification
        };

        matches!(last_notification, Some(last) if std::time::Instant::now().duration_since(last) < MIN_NOTIFICATION_INTERVAL)
    }

    fn record_notification_sent(&self, is_failure: bool) {
        let mut limiter = self.rate_limiter.lock().unwrap();
        let now = std::time::Instant::now();
        if is_failure {
            limiter.last_failure_notification = Some(now);
        } else {
            limiter.last_success_notification = Some(now);
        }
    }

    /// Telegram's Markdown parser treats `_`, `*`, `` ` ``, `[` as special
    /// characters; job names and (especially) restic error text are
    /// arbitrary and not under our control, so unbalanced delimiters can
    /// mangle the rendered message or make Telegram reject it outright. We
    /// send plain text (no `parse_mode`) instead of trying to escape
    /// arbitrary content. This also truncates to Telegram's 4096-char
    /// message limit so an oversized message doesn't get rejected wholesale.
    fn truncate_for_telegram(message: &str) -> String {
        if message.len() <= TELEGRAM_MAX_MESSAGE_LEN {
            return message.to_string();
        }
        let suffix = "\n… (truncated)";
        let mut end = TELEGRAM_MAX_MESSAGE_LEN.saturating_sub(suffix.len());
        while end > 0 && !message.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}{}", &message[..end], suffix)
    }

    async fn send_telegram(&self, message: &str) -> Result<(), AppError> {
        let bot_token = self
            .bot_token
            .as_ref()
            .ok_or(NotificationError::NotConfigured)?;
        let chat_id = self
            .chat_id
            .as_ref()
            .ok_or(NotificationError::NotConfigured)?;

        let url = format!("https://api.telegram.org/bot{}/sendMessage", bot_token);
        let message = Self::truncate_for_telegram(message);

        let params = [("chat_id", chat_id.as_str()), ("text", message.as_str())];

        let response = self
            .client
            .post(&url)
            .form(&params)
            .send()
            .await
            .map_err(NotificationError::RequestFailed)?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(NotificationError::SendFailed(format!("{}: {}", status, body)).into());
        }

        Ok(())
    }
}

pub struct NotificationManager {
    notifications: Notifications,
    job_config: NotificationConfig,
}

impl NotificationManager {
    pub fn new(config: &ResolvedConfig, job_config: NotificationConfig) -> Self {
        Self {
            notifications: Notifications::new(config),
            job_config,
        }
    }

    pub async fn notify_failure(&self, job_name: &str, error: &str) -> Result<(), AppError> {
        if self.job_config.on_failure {
            self.notifications.send_failure(job_name, error).await?;
        }
        Ok(())
    }

    pub async fn notify_success(
        &self,
        job_name: &str,
        snapshot_id: Option<&str>,
    ) -> Result<(), AppError> {
        if self.job_config.on_success {
            self.notifications
                .send_success(job_name, snapshot_id)
                .await?;
        }
        Ok(())
    }

    /// Partial backups (some files unreadable) are routed through the
    /// on_failure toggle rather than on_success: on_failure defaults to
    /// true and on_success defaults to false, so routing through
    /// on_success would silently produce zero notification for a partial
    /// backup under the default config - exactly the failure-adjacent case
    /// where alerting matters most.
    pub async fn notify_partial(
        &self,
        job_name: &str,
        snapshot_id: Option<&str>,
        error: &str,
    ) -> Result<(), AppError> {
        if self.job_config.on_failure {
            self.notifications
                .send_partial(job_name, snapshot_id, error)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_for_telegram_short_message_unchanged() {
        let message = "short message";
        assert_eq!(Notifications::truncate_for_telegram(message), message);
    }

    #[test]
    fn test_truncate_for_telegram_exact_limit_unchanged() {
        let message = "a".repeat(TELEGRAM_MAX_MESSAGE_LEN);
        assert_eq!(Notifications::truncate_for_telegram(&message), message);
    }

    #[test]
    fn test_truncate_for_telegram_over_limit_is_truncated() {
        let message = "a".repeat(TELEGRAM_MAX_MESSAGE_LEN + 500);
        let result = Notifications::truncate_for_telegram(&message);
        assert!(result.len() <= TELEGRAM_MAX_MESSAGE_LEN);
        assert!(result.ends_with("(truncated)"));
    }

    #[test]
    fn test_truncate_for_telegram_respects_char_boundaries() {
        // Multi-byte UTF-8 characters right at the truncation boundary
        // must not cause a panic from slicing mid-character.
        let message = "é".repeat(TELEGRAM_MAX_MESSAGE_LEN);
        let result = Notifications::truncate_for_telegram(&message);
        assert!(result.len() <= TELEGRAM_MAX_MESSAGE_LEN);
    }

    fn test_config() -> ResolvedConfig {
        use crate::config::{Config, Job, Repository as RepoConfig};
        use crate::secrets::Secrets;

        let mut repositories = std::collections::HashMap::new();
        repositories.insert(
            "test".to_string(),
            RepoConfig {
                repo: "/tmp/test-repo".to_string(),
                password_key: "test-password".to_string(),
                log_cli_output: None,
            },
        );

        let mut jobs = std::collections::HashMap::new();
        jobs.insert(
            "test-job".to_string(),
            Job {
                repository: "test".to_string(),
                paths: vec!["/tmp".into()],
                exclude_file: None,
                exclude_patterns: None,
                schedule: None,
                retention: None,
                notifications: Default::default(),
                pre_backup: vec![],
                post_backup: vec![],
            },
        );

        let secrets_values = std::collections::HashMap::new();

        ResolvedConfig {
            config: Config { repositories, jobs },
            secrets: Secrets {
                values: secrets_values,
                telegram: None,
            },
        }
    }

    #[test]
    fn test_not_configured() {
        let config = test_config();
        let notifications = Notifications::new(&config);
        assert!(!notifications.is_configured());
    }

    #[tokio::test]
    async fn test_send_failure_not_configured() {
        let config = test_config();
        let notifications = Notifications::new(&config);
        let result = notifications.send_failure("test-job", "test error").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_success_not_configured() {
        let config = test_config();
        let notifications = Notifications::new(&config);
        let result = notifications
            .send_success("test-job", Some("snap123"))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_notification_manager_respects_on_failure_flag() {
        let config = test_config();
        let job_config = NotificationConfig {
            on_failure: false,
            on_success: false,
        };
        let manager = NotificationManager::new(&config, job_config);
        let result = manager.notify_failure("test-job", "error").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_notification_manager_respects_on_success_flag() {
        let config = test_config();
        let job_config = NotificationConfig {
            on_failure: false,
            on_success: false,
        };
        let manager = NotificationManager::new(&config, job_config);
        let result = manager.notify_success("test-job", Some("snap123")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_notification_manager_notify_failure_flag_enabled() {
        let config = test_config();
        let job_config = NotificationConfig {
            on_failure: true,
            on_success: false,
        };
        let manager = NotificationManager::new(&config, job_config);
        let result = manager.notify_failure("test-job", "error").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_notification_manager_notify_success_flag_enabled() {
        let config = test_config();
        let job_config = NotificationConfig {
            on_failure: false,
            on_success: true,
        };
        let manager = NotificationManager::new(&config, job_config);
        let result = manager.notify_success("test-job", Some("snap123")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_failure_configured_does_not_panic_from_spawned_task() {
        // Regression test for #26: send_telegram used to spin up a nested
        // tokio::runtime::Runtime and call block_on() on it, which panics
        // when invoked from within an already-running runtime - exactly
        // how the scheduler calls this, via tokio::spawn. Reproduce that
        // calling shape here with telegram actually configured, so the
        // call reaches send_telegram instead of short-circuiting on
        // is_configured() == false like the other tests in this file.
        use crate::secrets::{Secrets, TelegramConfig};
        let mut config = test_config();
        config.secrets = Secrets {
            values: std::collections::HashMap::new(),
            telegram: Some(TelegramConfig {
                bot_token: Some("test-token".to_string()),
                chat_id: Some("test-chat".to_string()),
            }),
        };

        let handle = tokio::spawn(async move {
            let notifications = Notifications::new(&config);
            // The fake token means the actual HTTP call will fail or be
            // rejected; we only care that awaiting it from inside this
            // spawned task completes without panicking, not what the
            // Telegram API actually returns.
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                notifications.send_failure("test-job", "test error"),
            )
            .await;
        });

        assert!(handle.await.is_ok(), "notification task should not panic");
    }

    #[test]
    fn test_is_configured_with_empty_bot_token() {
        use crate::secrets::{Secrets, TelegramConfig};
        let mut config = test_config();
        config.secrets = Secrets {
            values: std::collections::HashMap::new(),
            telegram: Some(TelegramConfig {
                bot_token: None,
                chat_id: Some("123".to_string()),
            }),
        };
        let notifications = Notifications::new(&config);
        assert!(!notifications.is_configured());
    }

    #[test]
    fn test_is_configured_with_empty_chat_id() {
        use crate::secrets::{Secrets, TelegramConfig};
        let mut config = test_config();
        config.secrets = Secrets {
            values: std::collections::HashMap::new(),
            telegram: Some(TelegramConfig {
                bot_token: Some("token".to_string()),
                chat_id: None,
            }),
        };
        let notifications = Notifications::new(&config);
        assert!(!notifications.is_configured());
    }

    #[test]
    fn test_is_configured_with_both_none() {
        use crate::secrets::{Secrets, TelegramConfig};
        let mut config = test_config();
        config.secrets = Secrets {
            values: std::collections::HashMap::new(),
            telegram: Some(TelegramConfig {
                bot_token: None,
                chat_id: None,
            }),
        };
        let notifications = Notifications::new(&config);
        assert!(!notifications.is_configured());
    }

    #[test]
    fn test_is_configured_full() {
        use crate::secrets::{Secrets, TelegramConfig};
        let mut config = test_config();
        config.secrets = Secrets {
            values: std::collections::HashMap::new(),
            telegram: Some(TelegramConfig {
                bot_token: Some("token".to_string()),
                chat_id: Some("chat123".to_string()),
            }),
        };
        let notifications = Notifications::new(&config);
        assert!(notifications.is_configured());
    }

    #[test]
    fn test_is_rate_limited_does_not_mutate_state() {
        // Regression test for #40: is_rate_limited must be read-only.
        // Previously, checking the rate limit had the side effect of
        // recording "we just notified" even if the caller never actually
        // sent anything - so a failed send would silently poison the
        // window for every real failure afterward.
        let config = test_config();
        let notifications = Notifications::new(&config);
        assert!(!notifications.is_rate_limited(true));
        assert!(
            !notifications.is_rate_limited(true),
            "checking the rate limit alone must not consume it"
        );
    }

    #[test]
    fn test_record_notification_sent_then_rate_limited() {
        let config = test_config();
        let notifications = Notifications::new(&config);
        assert!(!notifications.is_rate_limited(true));
        notifications.record_notification_sent(true);
        assert!(notifications.is_rate_limited(true));
    }

    #[test]
    fn test_failure_and_success_rate_limits_are_independent() {
        let config = test_config();
        let notifications = Notifications::new(&config);
        notifications.record_notification_sent(true);
        assert!(notifications.is_rate_limited(true));
        assert!(!notifications.is_rate_limited(false));
    }

    #[tokio::test]
    async fn test_send_partial_not_configured() {
        let config = test_config();
        let notifications = Notifications::new(&config);
        let result = notifications
            .send_partial("test-job", Some("snap123"), "3 files unreadable")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_notify_partial_respects_on_failure_flag() {
        let config = test_config();
        let job_config = NotificationConfig {
            on_failure: false,
            on_success: false,
        };
        let manager = NotificationManager::new(&config, job_config);
        let result = manager
            .notify_partial("test-job", Some("snap123"), "err")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_notify_partial_enabled_via_on_failure() {
        let config = test_config();
        let job_config = NotificationConfig {
            on_failure: true,
            on_success: false,
        };
        let manager = NotificationManager::new(&config, job_config);
        let result = manager
            .notify_partial("test-job", Some("snap123"), "err")
            .await;
        assert!(result.is_ok());
    }
}
