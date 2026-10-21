//! Single-instance guard.
//!
//! Starting a second copy would register a duplicate global keyboard hook and a
//! duplicate tray icon, so the second process exits immediately.

use anyhow::{Context, Result};
use fs2::FileExt;
use std::fs::File;
use std::path::{Path, PathBuf};

pub struct InstanceGuard {
    _file: File,
    path: PathBuf,
}

impl InstanceGuard {
    /// Tries to take an exclusive lock. Returns `Ok(None)` when another instance
    /// already holds it.
    pub fn acquire(dir: &Path) -> Result<Option<Self>> {
        crate::paths::ensure_dir(dir.to_path_buf());
        let path = dir.join("wflow.lock");
        let file = File::create(&path)
            .with_context(|| format!("Failed to create instance lock {:?}", path))?;

        match file.try_lock_exclusive() {
            Ok(()) => {
                let _ = std::fs::write(&path, std::process::id().to_string());
                Ok(Some(Self { _file: file, path }))
            }
            Err(_) => Ok(None),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_instance_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let first = InstanceGuard::acquire(dir.path()).unwrap();
        assert!(first.is_some());
        let second = InstanceGuard::acquire(dir.path()).unwrap();
        assert!(second.is_none(), "second instance must be rejected");
        drop(first);
    }
}
