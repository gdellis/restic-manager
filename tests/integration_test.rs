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
        Hook::Command {
            command,
            args,
            continue_on_error,
        } => {
            assert_eq!(command, "/usr/local/bin/backup-hook");
            assert_eq!(args, vec!["--notify"]);
            assert!(!continue_on_error);
        }
        _ => panic!("Expected Command hook"),
    }
}

#[test]
fn test_hook_command_continue_on_error_deserialize() {
    let yaml = r#"
type: Command
command: /usr/local/bin/notify-hook
args: []
continue_on_error: true
"#;
    let hook: Hook = serde_yaml::from_str(yaml).unwrap();
    match hook {
        Hook::Command {
            continue_on_error, ..
        } => {
            assert!(continue_on_error);
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

/// Regression test for the config.yaml example in README.md's Quick Start
/// section: parses the exact YAML shown there and asserts every field
/// actually landed where the doc claims, so a future field rename can't
/// silently make the example wrong again (serde ignores unknown keys by
/// default, so a typo'd field name would otherwise fail silently, not
/// with a deserialization error).
#[test]
fn test_readme_quick_start_config_example_is_valid() {
    let yaml = r#"
repositories:
  local:
    repo: /backup/my-repo
    password_key: restic-password

jobs:
  documents:
    repository: local
    paths:
      - /home/user/documents
    exclude_patterns:
      - "*.tmp"
      - ".cache/**"
    schedule: "0 2 * * *"  # 2 AM daily
    retention:
      keep_daily: 7
      keep_weekly: 4
      keep_monthly: 6
    notifications:
      on_failure: true
      on_success: false
    pre_backup:
      - type: Command
        command: /usr/local/bin/db-dump.sh
        args: []
        continue_on_error: false  # abort the backup if this hook fails (default)
"#;
    let config: Config = serde_yaml::from_str(yaml).unwrap();

    let repo = &config.repositories["local"];
    assert_eq!(repo.repo, "/backup/my-repo");
    assert_eq!(repo.password_key, "restic-password");

    let job = &config.jobs["documents"];
    assert_eq!(job.repository, "local");
    assert_eq!(
        job.paths,
        vec![std::path::PathBuf::from("/home/user/documents")]
    );
    assert_eq!(
        job.exclude_patterns,
        Some(vec!["*.tmp".to_string(), ".cache/**".to_string()])
    );
    assert_eq!(job.schedule.as_deref(), Some("0 2 * * *"));

    let retention = job.retention.as_ref().unwrap();
    assert_eq!(retention.keep_daily, Some(7));
    assert_eq!(retention.keep_weekly, Some(4));
    assert_eq!(retention.keep_monthly, Some(6));

    assert!(job.notifications.on_failure);
    assert!(!job.notifications.on_success);

    assert_eq!(job.pre_backup.len(), 1);
    match &job.pre_backup[0] {
        Hook::Command {
            command,
            args,
            continue_on_error,
        } => {
            assert_eq!(command, "/usr/local/bin/db-dump.sh");
            assert!(args.is_empty());
            assert!(!continue_on_error);
        }
        _ => panic!("Expected Command hook"),
    }
}

/// Regression test for the secrets.yaml example in README.md's Quick Start.
#[test]
fn test_readme_quick_start_secrets_example_is_valid() {
    let yaml = r#"
restic-password: your-secret-password
telegram:
  bot_token: your-bot-token
  chat_id: your-chat-id
"#;
    let secrets: Secrets = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(secrets.get("restic-password"), Some("your-secret-password"));
    let telegram = secrets.telegram.unwrap();
    assert_eq!(telegram.bot_token.as_deref(), Some("your-bot-token"));
    assert_eq!(telegram.chat_id.as_deref(), Some("your-chat-id"));
}
