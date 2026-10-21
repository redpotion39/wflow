//! Secret storage.
//!
//! Tokens and API keys are kept out of `config.json` and written to a
//! dedicated `secrets.json` file with `0600` permissions (on Unix). The file is
//! written atomically so a crash cannot truncate credentials.
//!
//! For a stronger guarantee, the same abstraction can be backed by the OS
//! keyring (Windows Credential Manager / Secret Service) without touching the
//! callers.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SECRETS_FILE: &str = "secrets.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secrets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub groq_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevenlabs_api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseten_api_key: Option<String>,
}

impl Secrets {
    /// Secrets live next to the configuration file.
    pub fn path_for(config_path: &Path) -> PathBuf {
        config_path
            .parent()
            .map(|p| p.join(SECRETS_FILE))
            .unwrap_or_else(|| PathBuf::from(SECRETS_FILE))
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|err| {
                tracing::warn!("Ignoring malformed secrets file {:?}: {}", path, err);
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Persists secrets with restrictive permissions and atomic replacement.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            crate::paths::ensure_dir(parent.to_path_buf());
        }
        let json = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");

        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)
                .with_context(|| format!("Failed to create secrets file {:?}", tmp))?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(&tmp, json.as_bytes())?;
        }

        std::fs::rename(&tmp, path)
            .with_context(|| format!("Failed to replace secrets file {:?}", path))?;
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.auth_token.is_none()
            && self.groq_api_key.is_none()
            && self.elevenlabs_api_key.is_none()
            && self.baseten_api_key.is_none()
    }
}

/// Reads an environment variable, returning `None` for empty values.
pub fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        let secrets = Secrets {
            auth_token: Some("jwt".into()),
            groq_api_key: Some("gsk".into()),
            ..Default::default()
        };
        secrets.save(&path).unwrap();
        assert_eq!(Secrets::load(&path), secrets);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "secrets must not be world readable");
        }
    }

    #[test]
    fn malformed_file_falls_back_to_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(Secrets::load(&path).is_empty());
    }
}
