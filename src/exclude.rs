use std::path::PathBuf;

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
    let dir = dirs::config_dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "Cannot find config directory")
    })?;
    Ok(dir.join("restic-manager"))
}

pub fn exclude_file_path() -> Result<PathBuf, std::io::Error> {
    Ok(config_dir()?.join("excludes.txt"))
}

pub fn exclude_d_dir() -> Result<PathBuf, std::io::Error> {
    Ok(config_dir()?.join("excludes.d"))
}

pub fn ensure_default_exclude_file() -> Result<PathBuf, std::io::Error> {
    let path = exclude_file_path()?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, default_exclude_content())?;
    }
    Ok(path)
}

pub fn write_job_excludes(job_name: &str, patterns: &[String]) -> Result<PathBuf, std::io::Error> {
    let dir = exclude_d_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.txt", job_name));
    let content: String = patterns.iter().map(|p| format!("{}\n", p)).collect();
    std::fs::write(&path, content)?;
    Ok(path)
}

pub fn resolve_exclude_file(job: &crate::config::Job, job_name: &str) -> Option<PathBuf> {
    if let Some(ref file_path) = job.exclude_file {
        let path = PathBuf::from(file_path);
        if path.exists() {
            return Some(path);
        }
    }

    if let Some(patterns) = &job.exclude_patterns {
        if !patterns.is_empty() {
            if let Ok(path) = write_job_excludes(job_name, patterns) {
                return Some(path);
            }
        }
    }

    exclude_file_path().ok().filter(|p| p.exists())
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

    #[test]
    fn test_config_dir_returns_path() {
        let result = config_dir();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.to_str().unwrap().ends_with("restic-manager"));
    }
}
