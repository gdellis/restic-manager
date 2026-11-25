use crate::errors::AppError;
use std::path::{Component, Path, PathBuf};

pub fn default_exclude_content() -> &'static str {
    r#"# Restic exclude patterns
# https://restic.readthedocs.io/en/latest/040_backup.html#excluding-files

# Cache directories
.cache
.local/share/Trash

# Package managers
node_modules
.npm
.bun

# Version control
.git

# Virtual environments
.venv
venv

# Python
__pycache__
*.pyc

# Rust
.cargo/registry
.rustup

# IDE
.vscode/extensions
.vscode/configurations

# OS caches
Library/Caches
AppData/Local/Cache
"#
}

pub fn config_dir() -> Result<PathBuf, std::io::Error> {
    if let Some(dir) = std::env::var_os("RESTIC_MANAGER_CONFIG_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let dir = dirs::config_dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "Cannot find config directory")
    })?;
    Ok(dir.join("restic-manager"))
}

pub fn exclude_file_path() -> Result<PathBuf, std::io::Error> {
    Ok(config_dir()?.join("excludes.txt"))
}

/// Creates a directory and its parents with mode `0700` on Unix.
///
/// `0700` matches the permission warning in `secrets.rs`: the directory
/// holds `secrets.yaml` (plaintext passwords) and should not be readable
/// or writable by group or others. If the directory already existed with
/// looser permissions, it is chmod'd to 0700; this is intentional — the
/// directory is reserved for restic-manager secrets.
fn create_dir_all_secure(dir: &std::path::Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        use std::os::unix::fs::PermissionsExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        // Ensure the requested mode survives the process umask and any
        // pre-existing directory with looser permissions.
        let mut perms = std::fs::metadata(dir)?.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(dir, perms)?;
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

pub fn ensure_default_exclude_file() -> Result<PathBuf, std::io::Error> {
    ensure_default_exclude_file_in(&exclude_file_path()?)
}

fn ensure_default_exclude_file_in(path: &std::path::Path) -> Result<PathBuf, std::io::Error> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            create_dir_all_secure(parent)?;
        }
        std::fs::write(path, default_exclude_content())?;
    }
    Ok(path.to_path_buf())
}

/// Checks that a job name can be used verbatim as a file name inside
/// `excludes.d/`.
///
/// The job name comes from the config file, and `Path::join` does not
/// sanitise it: a job called `../../escaped` would write its
/// `exclude_patterns` outside the config directory. The rules are the
/// same on every platform so a config validates identically across the
/// build matrix: `\` and `:` are rejected even on Unix, where they are
/// legal in file names, because on Windows they are a separator and a
/// drive prefix (`C:name` joined onto a directory *replaces* it).
pub fn validate_job_file_name(job_name: &str) -> Result<(), String> {
    if job_name.trim().is_empty() {
        return Err("job name is empty".to_string());
    }
    if job_name == "." || job_name == ".." {
        return Err(format!(
            "job name {:?} is not usable as a file name under excludes.d/",
            job_name
        ));
    }
    if job_name.contains('\0') {
        return Err(format!(
            "job name {:?} must not contain a NUL byte",
            job_name
        ));
    }
    if job_name.contains(['/', '\\', ':']) {
        return Err(format!(
            "job name {:?} is used as a file name under excludes.d/ and must not contain \
             '/', '\\' or ':'",
            job_name
        ));
    }
    // Backstop for any platform-specific prefix the character filter
    // above does not know about.
    let mut components = Path::new(job_name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Ok(()),
        _ => Err(format!(
            "job name {:?} is not a plain file name (it contains a path prefix or separator)",
            job_name
        )),
    }
}

fn write_job_excludes_in(
    dir: &Path,
    job_name: &str,
    patterns: &[String],
) -> Result<PathBuf, std::io::Error> {
    validate_job_file_name(job_name)
        .map_err(|msg| std::io::Error::new(std::io::ErrorKind::InvalidInput, msg))?;
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.txt", job_name));
    let content: String = patterns.iter().map(|p| format!("{}\n", p)).collect();
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Picks the exclude file for a backup of `job`.
///
/// Explicit configuration is honoured or fails loudly, never silently
/// replaced: a configured `exclude_file` that does not exist is an error,
/// and so is failing to materialise `exclude_patterns`. Only when the job
/// configures neither does this fall back to the shared default file
/// (`excludes.txt` in the config directory), if present.
pub fn resolve_exclude_file(
    job: &crate::config::Job,
    job_name: &str,
) -> Result<Option<PathBuf>, AppError> {
    resolve_exclude_file_in(&config_dir()?, job, job_name)
}

fn resolve_exclude_file_in(
    config_dir: &Path,
    job: &crate::config::Job,
    job_name: &str,
) -> Result<Option<PathBuf>, AppError> {
    if let Some(ref file_path) = job.exclude_file {
        let path = PathBuf::from(file_path);
        if !path.is_file() {
            return Err(AppError::Other(format!(
                "Job '{}' is configured with exclude_file '{}', but that file does not exist \
                 (or is not a regular file)",
                job_name,
                path.display()
            )));
        }
        return Ok(Some(path));
    }

    if let Some(patterns) = &job.exclude_patterns {
        if !patterns.is_empty() {
            let path = write_job_excludes_in(&config_dir.join("excludes.d"), job_name, patterns)
                .map_err(|e| {
                    AppError::Other(format!(
                        "Job '{}': cannot write exclude_patterns file: {}",
                        job_name, e
                    ))
                })?;
            return Ok(Some(path));
        }
    }

    Ok(Some(config_dir.join("excludes.txt")).filter(|p| p.is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_exclude_content_not_empty() {
        assert!(!default_exclude_content().is_empty());
        assert!(default_exclude_content().contains(".cache"));
        assert!(default_exclude_content().contains(".git"));
    }

    /// M9: when ensure_default_exclude_file creates the config directory,
    /// it must be mode 0700 on Unix so secrets.yaml created alongside it
    /// starts in a private directory.
    #[test]
    fn test_default_exclude_file_creates_secure_config_dir() {
        let scratch = Scratch::new("secure-dir");
        let path = scratch.config_dir().join("excludes.txt");
        ensure_default_exclude_file_in(&path).unwrap();
        assert!(path.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(scratch.config_dir())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(
                mode, 0o700,
                "config dir must be created with mode 0700, got {:o}",
                mode
            );
        }
    }

    #[test]
    fn test_config_dir_returns_path() {
        let result = config_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        let suffix = std::path::MAIN_SEPARATOR_STR.to_string() + "restic-manager";
        assert!(
            path.to_str().unwrap().ends_with(&suffix),
            "expected path to end with '{}', got {:?}",
            suffix,
            path
        );
    }

    /// A fresh scratch directory per test, removed on drop (including when
    /// an assertion panics), so tests never share state, never touch the
    /// real config directory, and never need the process-global
    /// RESTIC_MANAGER_CONFIG_DIR (which would race between tests).
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "restic-manager-test-exclude-{}-{}-{}",
                tag,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        /// A config directory nested two levels below the scratch root, so a
        /// `../../escaped` from `<config>/excludes.d/` lands inside the
        /// scratch root (and is cleaned up) rather than in the shared temp dir.
        fn config_dir(&self) -> PathBuf {
            let dir = self.0.join("cfg").join("restic-manager");
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn make_job(
        exclude_file: Option<&str>,
        exclude_patterns: Option<Vec<&str>>,
    ) -> crate::config::Job {
        crate::config::Job {
            repository: "test".to_string(),
            paths: vec!["/tmp".into()],
            exclude_file: exclude_file.map(str::to_string),
            exclude_patterns: exclude_patterns.map(|p| p.into_iter().map(str::to_string).collect()),
            ..Default::default()
        }
    }

    #[test]
    fn test_validate_job_file_name_accepts_plain_names() {
        for name in ["daily", "home-docs", "job_01", "photos.2026", ".hidden"] {
            assert!(validate_job_file_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn test_validate_job_file_name_rejects_traversal_and_separators() {
        for name in [
            "",
            "   ",
            ".",
            "..",
            "../../escaped",
            "a/b",
            "a\\b",
            "/abs",
            "nul\0byte",
            "C:foo",
            "C:",
            "a:b",
        ] {
            assert!(validate_job_file_name(name).is_err(), "{name:?}");
        }
    }

    /// H13 regression: a job named with traversal segments must not be
    /// able to write outside excludes.d/.
    #[test]
    fn test_write_job_excludes_rejects_traversal_job_name() {
        let scratch = Scratch::new("traversal");
        let excludes_d = scratch.config_dir().join("excludes.d");

        let err = write_job_excludes_in(&excludes_d, "../../escaped", &["*.tmp".to_string()])
            .expect_err("traversal job name must be rejected");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);

        // Nothing was written anywhere: not the escaped path, and not a
        // sanitised one either (the whole job is refused).
        assert!(!scratch.0.join("cfg").join("escaped.txt").exists());
        assert!(!excludes_d.exists());
    }

    #[test]
    fn test_write_job_excludes_writes_patterns_for_plain_name() {
        let scratch = Scratch::new("plain");
        let excludes_d = scratch.config_dir().join("excludes.d");

        let path = write_job_excludes_in(
            &excludes_d,
            "daily",
            &["*.tmp".to_string(), "node_modules".to_string()],
        )
        .unwrap();

        assert_eq!(path, excludes_d.join("daily.txt"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "*.tmp\nnode_modules\n"
        );
    }

    /// M1: an explicitly configured exclude_file that is missing must be
    /// an error, not a silent fall-through to patterns or the default.
    #[test]
    fn test_resolve_missing_configured_exclude_file_is_an_error() {
        let scratch = Scratch::new("missing");
        let root = scratch.config_dir();
        // Both fallbacks are available, to prove neither is used.
        std::fs::write(root.join("excludes.txt"), ".cache\n").unwrap();
        let missing = root.join("does-not-exist.txt");
        let job = make_job(Some(missing.to_str().unwrap()), Some(vec!["*.tmp"]));

        let err = resolve_exclude_file_in(&root, &job, "daily")
            .expect_err("missing exclude_file must error");
        let msg = err.to_string();
        assert!(msg.contains("daily"), "{msg}");
        assert!(msg.contains("exclude_file"), "{msg}");
        assert!(msg.contains("does-not-exist.txt"), "{msg}");
        assert!(
            !root.join("excludes.d").exists(),
            "patterns must not be written"
        );
    }

    #[test]
    fn test_resolve_configured_exclude_file_wins_when_present() {
        let scratch = Scratch::new("present");
        let root = scratch.config_dir();
        let explicit = root.join("mine.txt");
        std::fs::write(&explicit, "*.log\n").unwrap();
        std::fs::write(root.join("excludes.txt"), ".cache\n").unwrap();
        let job = make_job(Some(explicit.to_str().unwrap()), Some(vec!["*.tmp"]));

        let resolved = resolve_exclude_file_in(&root, &job, "daily").unwrap();
        assert_eq!(resolved, Some(explicit));
        assert!(!root.join("excludes.d").exists());
    }

    #[test]
    fn test_resolve_patterns_are_written_under_excludes_d() {
        let scratch = Scratch::new("patterns");
        let root = scratch.config_dir();
        let job = make_job(None, Some(vec!["*.tmp"]));

        let resolved = resolve_exclude_file_in(&root, &job, "daily").unwrap();
        assert_eq!(resolved, Some(root.join("excludes.d").join("daily.txt")));
    }

    #[test]
    fn test_resolve_patterns_with_traversal_job_name_is_an_error() {
        let scratch = Scratch::new("patterns-traversal");
        let root = scratch.config_dir();
        std::fs::write(root.join("excludes.txt"), ".cache\n").unwrap();
        let job = make_job(None, Some(vec!["*.tmp"]));

        let err = resolve_exclude_file_in(&root, &job, "../../escaped")
            .expect_err("must not fall back to the default file");
        assert!(err.to_string().contains("exclude_patterns"), "{err}");
        assert!(!scratch.0.join("cfg").join("escaped.txt").exists());
        assert!(!root.join("excludes.d").exists());
    }

    #[test]
    fn test_resolve_falls_back_to_default_only_when_nothing_configured() {
        let scratch = Scratch::new("default");
        let root = scratch.config_dir();
        let job = make_job(None, None);

        assert_eq!(resolve_exclude_file_in(&root, &job, "daily").unwrap(), None);

        std::fs::write(root.join("excludes.txt"), ".cache\n").unwrap();
        assert_eq!(
            resolve_exclude_file_in(&root, &job, "daily").unwrap(),
            Some(root.join("excludes.txt"))
        );

        // Empty pattern list counts as "nothing configured".
        let job = make_job(None, Some(vec![]));
        assert_eq!(
            resolve_exclude_file_in(&root, &job, "daily").unwrap(),
            Some(root.join("excludes.txt"))
        );
    }
}
