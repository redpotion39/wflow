//! Cross-platform application paths.
//!
//! All persistent state lives in a single per-user directory so that secrets,
//! configuration, logs and the SQLite database can be backed up or wiped
//! together.

use std::path::PathBuf;

/// Returns the per-user configuration directory, creating it if needed.
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(base).join("Wflow")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".config"))
            })
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("wflow")
    }
}

/// Directory holding the SQLite history database.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        config_dir()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let base = std::env::var("XDG_DATA_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".local").join("share"))
            })
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("wflow")
    }
}

/// Directory for rotating diagnostic logs.
pub fn log_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        config_dir().join("logs")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let base = std::env::var("XDG_STATE_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| PathBuf::from(h).join(".local").join("state"))
            })
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("wflow").join("logs")
    }
}

/// Ensures a directory exists, returning it for convenient chaining.
pub fn ensure_dir(path: PathBuf) -> PathBuf {
    let _ = std::fs::create_dir_all(&path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
    }
    path
}
