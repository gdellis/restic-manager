use restic_manager::config::{Config, Hook, NotificationConfig, RetentionPolicy};
use restic_manager::secrets::Secrets;

#[test]
fn test_config_deserialize() {
    let yaml = r#"
repositories:
  backup1:
    repo: /srv/backup1
    password_key: backup1_pass
jobs:
  daily:
    repository: backup1
    paths:
      - /home
"#;
    let config: Config = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(config.repositories.len(), 1);
    assert_eq!(config.jobs.len(), 1);
    assert_eq!(config.repositories["backup1"].repo, "/srv/backup1");
    assert_eq!(config.jobs["daily"].repository, "backup1");
}

#[test]
fn test_config_validate_job_refs_nonexistent_repo() {
    let yaml = r#"
repositories:
  backup1:
    repo: /srv/backup1
    password_key: pass1
jobs:
  daily:
    repository: nonexistent
    paths:
      - /home
"#;
    let config: Config = serde_yaml::from_str(yaml).unwrap();
    assert!(!config.repositories.contains_key("nonexistent"));
    assert_eq!(config.jobs["daily"].repository, "nonexistent");
}

#[test]
fn test_retention_policy_deserialization() {
    let yaml = r#"
keep_daily: 7
keep_weekly: 4
keep_monthly: 6
keep_last: 3
"#;
    let retention: RetentionPolicy = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(retention.keep_daily, Some(7));
    assert_eq!(retention.keep_weekly, Some(4));
    assert_eq!(retention.keep_monthly, Some(6));
    assert_eq!(retention.keep_last, Some(3));
}

#[test]
fn test_notification_config_defaults() {
    let yaml = r#"
on_failure: true
"#;
    let notif: NotificationConfig = serde_yaml::from_str(yaml).unwrap();
    assert!(notif.on_failure);
    assert!(!notif.on_success);
}

#[test]
fn test_secrets_deserialize() {
    let yaml = r#"
backup1_pass: secret123
telegram:
  bot_token: mytoken
  chat_id: mychat
"#;
    let secrets: Secrets = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(secrets.get("backup1_pass"), Some("secret123"));
    assert!(secrets.telegram.is_some());
    let telegram = secrets.telegram.as_ref().unwrap();
    assert_eq!(telegram.bot_token.as_deref(), Some("mytoken"));
}

#[test]
fn test_secrets_get_returns_none_for_missing_key() {
    let secrets: Secrets = serde_yaml::from_str("{}").unwrap();
    assert_eq!(secrets.get("nonexistent"), None);
}

#[test]
fn test_job_with_retention() {
    let yaml = r#"
repositories:
  test:
    repo: /srv/test
    password_key: test_pass
jobs:
  test_job:
    repository: test
    paths:
      - /data
    retention:
      keep_daily: 5
"#;
    let config: Config = serde_yaml::from_str(yaml).unwrap();
    let job = config.jobs.get("test_job").unwrap();
    assert!(job.retention.is_some());
}

#[test]
fn test_hook_command_deserialize() {
    let yaml = r#"
type: Command
command: /usr/local/bin/backup-hook
args:
  - "--notify"
"#;
    let hook: Hook = serde_yaml::from_str(yaml).unwrap();
    match hook {
        Hook::Command { command, args } => {
            assert_eq!(command, "/usr/local/bin/backup-hook");
            assert_eq!(args, vec!["--notify"]);
        }
        _ => panic!("Expected Command hook"),
    }
}

#[test]
fn test_hook_wait_deserialize() {
    let yaml = r#"
type: Wait
seconds: 30
"#;
    let hook: Hook = serde_yaml::from_str(yaml).unwrap();
    match hook {
        Hook::Wait { seconds } => {
            assert_eq!(seconds, 30);
        }
        _ => panic!("Expected Wait hook"),
    }
}

#[test]
fn test_retention_policy_all_fields() {
    let yaml = r#"
keep_daily: 7
keep_weekly: 4
keep_monthly: 6
keep_yearly: 1
keep_hourly: 12
keep_last: 5
"#;
    let retention: RetentionPolicy = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(retention.keep_daily, Some(7));
    assert_eq!(retention.keep_weekly, Some(4));
    assert_eq!(retention.keep_monthly, Some(6));
    assert_eq!(retention.keep_yearly, Some(1));
    assert_eq!(retention.keep_hourly, Some(12));
    assert_eq!(retention.keep_last, Some(5));
}

#[test]
fn test_config_empty() {
    let yaml = "{}";
    let config: Config = serde_yaml::from_str(yaml).unwrap();
    assert!(config.repositories.is_empty());
    assert!(config.jobs.is_empty());
}

#[test]
fn test_secrets_empty_telegram() {
    let yaml = "{}";
    let secrets: Secrets = serde_yaml::from_str(yaml).unwrap();
    assert!(secrets.telegram.is_none());
}
