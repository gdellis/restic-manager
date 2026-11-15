use crate::cli_log::write_command_output;
use crate::config::{ResolvedConfig, RetentionPolicy};
use crate::errors::{AppError, ResticError};
use crate::restic_child;
use serde::{Deserialize, Serialize};
use std::process::Command;
use tracing::{info, warn};

/// A single restic snapshot, parsed from `restic snapshots --json`.
///
/// Field names mirror the JSON keys restic emits, so the struct is populated
/// directly by serde in `SnapshotManager::list`. `id` and `time` are required:
/// if restic renames or retypes them, deserialization fails instead of
/// silently yielding empty strings. `tags`, `paths`, and `short_id` default
/// when absent (restic omits `tags` for untagged snapshots) but still error
/// on a type change. Extra fields restic adds over time are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Full snapshot ID assigned by restic.
    pub id: String,
    /// Timestamp of the snapshot in restic's RFC3339 string form.
    pub time: String,
    /// Hostname of the machine that produced the snapshot, if restic captured it.
    pub hostname: Option<String>,
    /// Tags attached to the snapshot at backup time.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Paths the snapshot covers.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Short form of the snapshot ID, suitable for display and `restore` commands.
    #[serde(default)]
    pub short_id: String,
}

/// The result of listing a job's snapshots, wrapped so callers can extend it
/// (e.g. with paging metadata) without breaking the public signature.
#[derive(Debug)]
pub struct SnapshotList {
    /// All snapshots returned by restic, in the order restic reported them.
    pub snapshots: Vec<Snapshot>,
}

/// One policy group from `restic forget --json`. restic emits one group per
/// `--group-by` key (default `host,paths`); only the `remove` list matters
/// here. Each entry is the same object `snapshots --json` emits, so it is
/// parsed with the same [`Snapshot`] struct and the same required fields.
///
/// `remove` is required so a schema rename surfaces as a parse error rather
/// than an empty result, but restic marshals the nil remove slice as `null`
/// when nothing is removed, so `null` is accepted and treated as empty.
/// The other keys restic emits (`keep`, `reasons`, `host`, ...) are ignored.
#[derive(Debug, Deserialize)]
struct ForgetGroup {
    #[serde(deserialize_with = "null_as_empty_vec")]
    remove: Vec<Snapshot>,
}

/// Accepts `null` as an empty list but still rejects a missing key: unlike
/// a plain `Option` field, `deserialize_with` disables serde's implicit
/// missing-field default (see [`ForgetGroup`]).
fn null_as_empty_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<Vec<T>>::deserialize(deserializer).map(Option::unwrap_or_default)
}

pub struct SnapshotManager;

impl SnapshotManager {
    pub fn list(config: &ResolvedConfig, job_name: &str) -> Result<SnapshotList, AppError> {
        let (_job, repo, password) = config.resolve_job(job_name)?;

        let mut cmd = Command::new("restic");
        cmd.args(["snapshots", "--json", "--repo", &repo.repo])
            .env("RESTIC_PASSWORD", password);
        let output = restic_child::capture_output(&mut cmd).map_err(ResticError::from_io)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(
                ResticError::CommandFailed(crate::redact::RedactedText::new(&stderr)).into(),
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Self::parse_snapshots(stdout.trim())
    }

    fn parse_snapshots(output: &str) -> Result<SnapshotList, AppError> {
        if output.is_empty() {
            return Ok(SnapshotList { snapshots: vec![] });
        }

        let snapshots: Vec<Snapshot> = serde_json::from_str(output)
            .map_err(|e| AppError::Other(format!("Failed to parse snapshots JSON: {}", e)))?;

        Ok(SnapshotList { snapshots })
    }

    pub fn forget(
        config: &ResolvedConfig,
        job_name: &str,
        dry_run: bool,
    ) -> Result<Vec<String>, AppError> {
        let (job, repo, password) = config.resolve_job(job_name)?;

        let retention = job.retention.as_ref().ok_or_else(|| {
            AppError::Other(format!(
                "No retention policy defined for job '{}'",
                job_name
            ))
        })?;

        let mut args = vec![
            "forget".to_string(),
            "--json".to_string(),
            "--repo".to_string(),
            repo.repo.clone(),
        ];

        Self::apply_retention_args(&mut args, retention);

        if dry_run {
            args.push("--dry-run".to_string());
        }

        info!(
            job = job_name,
            dry_run = dry_run,
            "Applying retention policy"
        );

        let mut cmd = Command::new("restic");
        cmd.args(&args).env("RESTIC_PASSWORD", password);
        let output = restic_child::capture_output(&mut cmd).map_err(ResticError::from_io)?;

        write_command_output(repo.log_cli_output.as_deref(), &output);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(
                ResticError::CommandFailed(crate::redact::RedactedText::new(&stderr)).into(),
            );
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Self::parse_forget_output(stdout.trim())
    }

    fn apply_retention_args(args: &mut Vec<String>, retention: &RetentionPolicy) {
        if let Some(n) = retention.keep_daily {
            args.push("--keep-daily".to_string());
            args.push(n.to_string());
        }
        if let Some(n) = retention.keep_weekly {
            args.push("--keep-weekly".to_string());
            args.push(n.to_string());
        }
        if let Some(n) = retention.keep_monthly {
            args.push("--keep-monthly".to_string());
            args.push(n.to_string());
        }
        if let Some(n) = retention.keep_yearly {
            args.push("--keep-yearly".to_string());
            args.push(n.to_string());
        }
        if let Some(n) = retention.keep_hourly {
            args.push("--keep-hourly".to_string());
            args.push(n.to_string());
        }
        if let Some(n) = retention.keep_last {
            args.push("--keep-last".to_string());
            args.push(n.to_string());
        }
    }

    fn parse_forget_output(output: &str) -> Result<Vec<String>, AppError> {
        if output.is_empty() {
            return Ok(vec![]);
        }

        let groups: Vec<ForgetGroup> = serde_json::from_str(output)
            .map_err(|e| AppError::Other(format!("Failed to parse forget JSON: {}", e)))?;

        Ok(groups
            .into_iter()
            .flat_map(|group| group.remove)
            .map(|snap| snap.id)
            .collect())
    }

    pub fn prune(config: &ResolvedConfig, job_name: &str) -> Result<(), AppError> {
        let (_job, repo, password) = config.resolve_job(job_name)?;

        info!(job = job_name, "Pruning repository");

        let mut cmd = Command::new("restic");
        cmd.args(["prune", "--repo", &repo.repo])
            .env("RESTIC_PASSWORD", password);
        let output = restic_child::capture_output(&mut cmd).map_err(ResticError::from_io)?;

        write_command_output(repo.log_cli_output.as_deref(), &output);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(
                ResticError::CommandFailed(crate::redact::RedactedText::new(&stderr)).into(),
            );
        }

        Ok(())
    }

    pub fn apply_retention(
        config: &ResolvedConfig,
        job_name: &str,
        dry_run: bool,
    ) -> Result<Vec<String>, AppError> {
        let removed = Self::forget(config, job_name, dry_run)?;

        if !dry_run && !removed.is_empty() {
            warn!(
                job = job_name,
                count = removed.len(),
                "Removed snapshots, running prune"
            );
            Self::prune(config, job_name)?;
        }

        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Job, Repository as RepoConfig};
    use crate::secrets::Secrets;

    fn test_config() -> ResolvedConfig {
        let mut repositories = std::collections::BTreeMap::new();
        repositories.insert(
            "test".to_string(),
            RepoConfig {
                repo: "/tmp/test-repo".to_string(),
                password_key: "test-password".to_string(),
                log_cli_output: None,
            },
        );

        let mut jobs = std::collections::BTreeMap::new();
        jobs.insert(
            "test-job".to_string(),
            Job {
                repository: "test".to_string(),
                paths: vec!["/tmp".into()],
                exclude_file: None,
                exclude_patterns: None,
                schedule: None,
                retention: Some(RetentionPolicy::default()),
                notifications: Default::default(),
                pre_backup: vec![],
                post_backup: vec![],
            },
        );

        let mut secrets_values = std::collections::HashMap::new();
        secrets_values.insert("test-password".to_string(), "test-secret".to_string());

        let config = Config { repositories, jobs };

        let secrets = Secrets {
            values: secrets_values,
            telegram: None,
        };

        ResolvedConfig { config, secrets }
    }

    #[test]
    fn test_retention_args() {
        let mut args = vec![];
        let retention = RetentionPolicy::default();
        SnapshotManager::apply_retention_args(&mut args, &retention);
        assert!(args.contains(&"--keep-daily".to_string()));
        assert!(args.contains(&"7".to_string()));
        assert!(args.contains(&"--keep-weekly".to_string()));
        assert!(args.contains(&"4".to_string()));
        assert!(args.contains(&"--keep-last".to_string()));
        assert!(args.contains(&"3".to_string()));
    }

    #[test]
    fn test_retention_args_empty() {
        let mut args = vec![];
        let retention = RetentionPolicy {
            keep_daily: None,
            keep_weekly: None,
            keep_monthly: None,
            keep_yearly: None,
            keep_hourly: None,
            keep_last: None,
        };
        SnapshotManager::apply_retention_args(&mut args, &retention);
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_snapshots_empty() {
        let result = SnapshotManager::parse_snapshots("");
        assert!(result.is_ok());
        let list = result.unwrap();
        assert!(list.snapshots.is_empty());
    }

    #[test]
    fn test_parse_forget_output_empty() {
        let result = SnapshotManager::parse_forget_output("");
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_job_not_found() {
        let config = test_config();
        let result = SnapshotManager::list(&config, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_forget_no_retention_policy() {
        let mut repositories = std::collections::BTreeMap::new();
        repositories.insert(
            "test".to_string(),
            RepoConfig {
                repo: "/tmp/test-repo".to_string(),
                password_key: "test-password".to_string(),
                log_cli_output: None,
            },
        );

        let mut jobs = std::collections::BTreeMap::new();
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

        let mut secrets_values = std::collections::HashMap::new();
        secrets_values.insert("test-password".to_string(), "test-secret".to_string());

        let config = ResolvedConfig {
            config: Config { repositories, jobs },
            secrets: Secrets {
                values: secrets_values,
                telegram: None,
            },
        };

        let result = SnapshotManager::forget(&config, "test-job", false);
        assert!(result.is_err());
    }

    // Verbatim `restic snapshots --json` outputs captured against a real local
    // repository with restic 0.16.4 (same approach as the RESTIC_SUMMARY_*
    // fixtures in src/backup.rs). The first fixture filters to two tagged
    // snapshots (`restic snapshots --json a434fba8 01cb76e5`); the second is an
    // untagged snapshot, which restic emits with no "tags" key at all.
    const RESTIC_SNAPSHOTS_TWO_TAGGED: &str = r#"[{"time":"2026-11-15T10:31:23.218486319-07:00","tree":"5f2ac0f467ca8ad9a4751fc4dee71d447b8493d1c9e164503469f830c581cf50","paths":["/data/projects/restic-manager/.omo/session-work/e2e/data/one.txt"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"tags":["daily"],"program_version":"restic 0.16.4","id":"a434fba8777181895db6a3d34a4a2895e40eb0bd07c8ed99ad333626ce6084d7","short_id":"a434fba8"},{"time":"2026-11-15T10:31:23.90424118-07:00","tree":"d62647d95d8c919a9796c8ae6ad3b1d2124477fad54608f22c9f8dc63f65f0ef","paths":["/data/projects/restic-manager/.omo/session-work/e2e/data/one.txt","/data/projects/restic-manager/.omo/session-work/e2e/data/two.txt"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"tags":["daily"],"program_version":"restic 0.16.4","id":"01cb76e5e47afd9bcc61d9d732f3d22e4c43959a64e3d504c8190890c0f789b6","short_id":"01cb76e5"}]"#;

    const RESTIC_SNAPSHOTS_NO_TAGS_KEY: &str = r#"[{"time":"2026-11-15T10:31:27.18471964-07:00","tree":"5e8e2ad3e485d761e00273960240daff3436976f761a08874ae13975c0851466","paths":["/data/projects/restic-manager/.omo/session-work/e2e/data/three.txt"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"program_version":"restic 0.16.4","id":"7ff282fbf777b56e601d265733dbd3c97142334a9c2400c9fc4d15f20ac9c95d","short_id":"7ff282fb"}]"#;

    #[test]
    fn test_parse_snapshots_real_output_two_tagged() {
        let list = SnapshotManager::parse_snapshots(RESTIC_SNAPSHOTS_TWO_TAGGED).unwrap();
        assert_eq!(list.snapshots.len(), 2);
        let first = &list.snapshots[0];
        assert_eq!(
            first.id,
            "a434fba8777181895db6a3d34a4a2895e40eb0bd07c8ed99ad333626ce6084d7"
        );
        assert_eq!(first.short_id, "a434fba8");
        assert_eq!(first.time, "2026-11-15T10:31:23.218486319-07:00");
        assert_eq!(first.hostname.as_deref(), Some("baggins-zorin"));
        assert_eq!(first.tags, vec!["daily"]);
        assert_eq!(
            first.paths,
            vec!["/data/projects/restic-manager/.omo/session-work/e2e/data/one.txt"]
        );
        assert_eq!(list.snapshots[1].paths.len(), 2);
    }

    #[test]
    fn test_parse_snapshots_real_output_no_tags_key() {
        // restic omits the "tags" key entirely for untagged snapshots; this
        // must parse with an empty tag list, not fail on a missing field.
        let list = SnapshotManager::parse_snapshots(RESTIC_SNAPSHOTS_NO_TAGS_KEY).unwrap();
        assert_eq!(list.snapshots.len(), 1);
        assert_eq!(list.snapshots[0].short_id, "7ff282fb");
        assert!(list.snapshots[0].tags.is_empty());
    }

    #[test]
    fn test_parse_snapshots_missing_id_errors() {
        // Schema change: restic renames "id" — must surface as a parse error,
        // not a snapshot with an empty id.
        let json =
            r#"[{"snapshot_id":"abc123","short_id":"abc123","time":"2026-01-01T00:00:00Z"}]"#;
        assert!(SnapshotManager::parse_snapshots(json).is_err());
    }

    #[test]
    fn test_parse_snapshots_missing_time_errors() {
        let json = r#"[{"id":"abc123","short_id":"abc123"}]"#;
        assert!(SnapshotManager::parse_snapshots(json).is_err());
    }

    #[test]
    fn test_parse_snapshots_mistyped_field_errors() {
        // Schema change: "time" becomes an object — must error, not default.
        let json = r#"[{"id":"abc123","time":{"secs":0}}]"#;
        assert!(SnapshotManager::parse_snapshots(json).is_err());
    }

    #[test]
    fn test_parse_snapshots_unknown_extra_fields_ok() {
        // restic adds fields over time; unknown keys must be ignored.
        let json = r#"[{"id":"abc123","time":"2026-01-01T00:00:00Z","brand_new_field":42}]"#;
        let list = SnapshotManager::parse_snapshots(json).unwrap();
        assert_eq!(list.snapshots.len(), 1);
        assert_eq!(list.snapshots[0].id, "abc123");
        assert!(list.snapshots[0].hostname.is_none());
    }

    #[test]
    fn test_parse_snapshots_malformed_json_errors() {
        assert!(SnapshotManager::parse_snapshots("not json").is_err());
    }

    // Verbatim `restic forget --json` outputs captured against a real local
    // repository with restic 0.16.4 (five snapshots of one path, all tagged
    // "daily", so restic produces a single policy group). The first fixture is
    // `restic forget --json --keep-last 2` (three removed); the second is
    // `--keep-last 10` on the remaining two, where restic marshals the empty
    // remove list as `null`, not `[]`. `--dry-run` emits the identical shape.
    const RESTIC_FORGET_THREE_REMOVED: &str = r#"[{"tags":null,"host":"baggins-zorin","paths":["/tmp/opencode/i134-data"],"keep":[{"time":"2026-11-15T14:23:35.537689527-07:00","parent":"bdd4f2deceb0c0c67f65e6472d02fe2666799d6b487b56e03f91abdd304f8296","tree":"bb1a1a61c76ae83aab6e37d7f03dc6debf7db25c127b6f70534bc9ae545813c6","paths":["/tmp/opencode/i134-data"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"tags":["daily"],"program_version":"restic 0.16.4","id":"bfe7610ce22dbf78fc27daac53de88e36437b901c6cfc8402d7e3203dc0d2e59","short_id":"bfe7610c"},{"time":"2026-11-15T14:23:34.802798069-07:00","parent":"3cfcc0c4eea09911041ef813dd6d9b76e70a419abbf3a693d6cf7a9741ea427c","tree":"fb80a4aab048f801fbca006adae6cb275b1d0b71b4b49746547a18056259d7ab","paths":["/tmp/opencode/i134-data"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"tags":["daily"],"program_version":"restic 0.16.4","id":"bdd4f2deceb0c0c67f65e6472d02fe2666799d6b487b56e03f91abdd304f8296","short_id":"bdd4f2de"}],"remove":[{"time":"2026-11-15T14:23:34.070005663-07:00","parent":"5be56177839b6133b21d3fe2e112d1121542019a7dbec8b9460ff263861c896e","tree":"d0550082df31054bd5c48a3ddc17af063ac88e3eca6b652dd959e6538b41b765","paths":["/tmp/opencode/i134-data"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"tags":["daily"],"program_version":"restic 0.16.4","id":"3cfcc0c4eea09911041ef813dd6d9b76e70a419abbf3a693d6cf7a9741ea427c","short_id":"3cfcc0c4"},{"time":"2026-11-15T14:23:33.34174476-07:00","parent":"c380b0e868e927fdb47db4078a9c9609044bb38940facb7edb0686e0ad3c2660","tree":"6cce98981de1b45c10425751c2497e63b4e6a8ffe4fef035777d28495852fe95","paths":["/tmp/opencode/i134-data"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"tags":["daily"],"program_version":"restic 0.16.4","id":"5be56177839b6133b21d3fe2e112d1121542019a7dbec8b9460ff263861c896e","short_id":"5be56177"},{"time":"2026-11-15T14:23:32.611896407-07:00","tree":"19527dd2d22a48f3c70c66ec1ff0b594465f57779133f88e5fca1013d2dd82c9","paths":["/tmp/opencode/i134-data"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"tags":["daily"],"program_version":"restic 0.16.4","id":"c380b0e868e927fdb47db4078a9c9609044bb38940facb7edb0686e0ad3c2660","short_id":"c380b0e8"}],"reasons":[{"snapshot":{"time":"2026-11-15T14:23:35.537689527-07:00","parent":"bdd4f2deceb0c0c67f65e6472d02fe2666799d6b487b56e03f91abdd304f8296","tree":"bb1a1a61c76ae83aab6e37d7f03dc6debf7db25c127b6f70534bc9ae545813c6","paths":["/tmp/opencode/i134-data"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"tags":["daily"],"program_version":"restic 0.16.4"},"matches":["last snapshot"],"counters":{"last":1}},{"snapshot":{"time":"2026-11-15T14:23:34.802798069-07:00","parent":"3cfcc0c4eea09911041ef813dd6d9b76e70a419abbf3a693d6cf7a9741ea427c","tree":"fb80a4aab048f801fbca006adae6cb275b1d0b71b4b49746547a18056259d7ab","paths":["/tmp/opencode/i134-data"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"tags":["daily"],"program_version":"restic 0.16.4"},"matches":["last snapshot"],"counters":{}}]}]"#;

    const RESTIC_FORGET_REMOVE_NULL: &str = r#"[{"tags":null,"host":"baggins-zorin","paths":["/tmp/opencode/i134-data"],"keep":[{"time":"2026-11-15T14:23:35.537689527-07:00","parent":"bdd4f2deceb0c0c67f65e6472d02fe2666799d6b487b56e03f91abdd304f8296","tree":"bb1a1a61c76ae83aab6e37d7f03dc6debf7db25c127b6f70534bc9ae545813c6","paths":["/tmp/opencode/i134-data"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"tags":["daily"],"program_version":"restic 0.16.4","id":"bfe7610ce22dbf78fc27daac53de88e36437b901c6cfc8402d7e3203dc0d2e59","short_id":"bfe7610c"},{"time":"2026-11-15T14:23:34.802798069-07:00","parent":"3cfcc0c4eea09911041ef813dd6d9b76e70a419abbf3a693d6cf7a9741ea427c","tree":"fb80a4aab048f801fbca006adae6cb275b1d0b71b4b49746547a18056259d7ab","paths":["/tmp/opencode/i134-data"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"tags":["daily"],"program_version":"restic 0.16.4","id":"bdd4f2deceb0c0c67f65e6472d02fe2666799d6b487b56e03f91abdd304f8296","short_id":"bdd4f2de"}],"remove":null,"reasons":[{"snapshot":{"time":"2026-11-15T14:23:35.537689527-07:00","parent":"bdd4f2deceb0c0c67f65e6472d02fe2666799d6b487b56e03f91abdd304f8296","tree":"bb1a1a61c76ae83aab6e37d7f03dc6debf7db25c127b6f70534bc9ae545813c6","paths":["/tmp/opencode/i134-data"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"tags":["daily"],"program_version":"restic 0.16.4"},"matches":["last snapshot"],"counters":{"last":9}},{"snapshot":{"time":"2026-11-15T14:23:34.802798069-07:00","parent":"3cfcc0c4eea09911041ef813dd6d9b76e70a419abbf3a693d6cf7a9741ea427c","tree":"fb80a4aab048f801fbca006adae6cb275b1d0b71b4b49746547a18056259d7ab","paths":["/tmp/opencode/i134-data"],"hostname":"baggins-zorin","username":"glenn","uid":1000,"gid":1000,"tags":["daily"],"program_version":"restic 0.16.4"},"matches":["last snapshot"],"counters":{"last":8}}]}]"#;

    #[test]
    fn test_parse_forget_output_real_output_three_removed() {
        let removed = SnapshotManager::parse_forget_output(RESTIC_FORGET_THREE_REMOVED).unwrap();
        assert_eq!(
            removed,
            vec![
                "3cfcc0c4eea09911041ef813dd6d9b76e70a419abbf3a693d6cf7a9741ea427c",
                "5be56177839b6133b21d3fe2e112d1121542019a7dbec8b9460ff263861c896e",
                "c380b0e868e927fdb47db4078a9c9609044bb38940facb7edb0686e0ad3c2660",
            ]
        );
    }

    #[test]
    fn test_parse_forget_output_real_output_remove_null() {
        // Nothing to remove: restic emits "remove":null. Must parse as empty,
        // not fail on a null where an array was expected.
        let removed = SnapshotManager::parse_forget_output(RESTIC_FORGET_REMOVE_NULL).unwrap();
        assert!(removed.is_empty());
    }

    #[test]
    fn test_parse_forget_output_multiple_groups() {
        let output = r#"[
            {"tags":null,"host":"h","paths":["/x"],"keep":[],"remove":[{"id":"snap1","short_id":"abc","time":"2026-01-01T00:00:00Z"}],"reasons":[]},
            {"tags":null,"host":"h","paths":["/y"],"keep":[],"remove":[{"id":"snap2","short_id":"def","time":"2026-01-01T00:00:00Z"},{"id":"snap3","short_id":"ghi","time":"2026-01-01T00:00:00Z"}],"reasons":[]}
        ]"#;
        let removed = SnapshotManager::parse_forget_output(output).unwrap();
        assert_eq!(removed, vec!["snap1", "snap2", "snap3"]);
    }

    #[test]
    fn test_parse_forget_output_empty_remove_array() {
        let output = r#"[{"tags":null,"host":"h","paths":["/x"],"keep":[{"id":"kept1"}],"remove":[],"reasons":[]}]"#;
        let removed = SnapshotManager::parse_forget_output(output).unwrap();
        assert!(removed.is_empty());
    }

    #[test]
    fn test_parse_forget_output_missing_remove_key_errors() {
        // Schema change: restic renames "remove" — must surface as a parse
        // error, not silently report zero removed snapshots.
        let output = r#"[{"tags":null,"host":"h","paths":["/x"],"keep":[],"removed":[{"id":"snap1","time":"2026-01-01T00:00:00Z"}],"reasons":[]}]"#;
        assert!(SnapshotManager::parse_forget_output(output).is_err());
    }

    #[test]
    fn test_parse_forget_output_missing_id_errors() {
        // Schema change: a removed snapshot without "id" — must error, not be
        // skipped and undercount what was removed.
        let output = r#"[{"remove":[{"snapshot_id":"snap1","time":"2026-01-01T00:00:00Z"}]}]"#;
        assert!(SnapshotManager::parse_forget_output(output).is_err());
    }

    #[test]
    fn test_parse_forget_output_mistyped_remove_errors() {
        // Schema change: "remove" becomes a count — must error, not default.
        let output = r#"[{"remove":3}]"#;
        assert!(SnapshotManager::parse_forget_output(output).is_err());
    }

    #[test]
    fn test_parse_forget_output_unknown_extra_fields_ok() {
        let output = r#"[{"remove":[{"id":"snap1","time":"2026-01-01T00:00:00Z","brand_new_field":42}],"brand_new_group_field":true}]"#;
        let removed = SnapshotManager::parse_forget_output(output).unwrap();
        assert_eq!(removed, vec!["snap1"]);
    }

    #[test]
    fn test_parse_forget_output_malformed_json_errors() {
        let result = SnapshotManager::parse_forget_output("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_forget_missing_password() {
        let mut repositories = std::collections::BTreeMap::new();
        repositories.insert(
            "test".to_string(),
            RepoConfig {
                repo: "/tmp/test-repo".to_string(),
                password_key: "nonexistent-key".to_string(),
                log_cli_output: None,
            },
        );

        let mut jobs = std::collections::BTreeMap::new();
        jobs.insert(
            "test-job".to_string(),
            Job {
                repository: "test".to_string(),
                paths: vec!["/tmp".into()],
                exclude_file: None,
                exclude_patterns: None,
                schedule: None,
                retention: Some(RetentionPolicy::default()),
                notifications: Default::default(),
                pre_backup: vec![],
                post_backup: vec![],
            },
        );

        let config = ResolvedConfig {
            config: Config { repositories, jobs },
            secrets: Secrets {
                values: std::collections::HashMap::new(),
                telegram: None,
            },
        };

        let result = SnapshotManager::forget(&config, "test-job", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_prune_missing_job() {
        let config = test_config();
        let result = SnapshotManager::prune(&config, "nonexistent");
        assert!(result.is_err());
    }
}
