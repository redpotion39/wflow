//! In-app self-update: download, verify, install and restart.
//!
//! The update feed is read-only, but the release archive contains the raw
//! binary, so the app can replace itself without a package manager:
//!
//! 1. download the release archive (and the checksum manifest),
//! 2. verify the SHA-256 of the archive against the manifest,
//! 3. unpack the binary and ask it for its version as a sanity check,
//! 4. replace the running executable (atomically, via a rename),
//! 5. restart the application.
//!
//! When the executable lives in a directory the user cannot write to
//! (`/usr/bin/wflow` for a package install), the replacement is retried
//! through `pkexec`, which shows the usual desktop authentication prompt.

use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::update::UpdateInfo;

/// How far the installation has progressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstallStage {
    /// Nothing is running.
    #[default]
    Idle,
    /// Downloading the release archive.
    Downloading,
    /// Checking the SHA-256 checksum.
    Verifying,
    /// Unpacking and replacing the executable.
    Installing,
    /// The new version is in place; the app is about to restart.
    Restarting,
    /// Finished successfully.
    Done,
    /// Something went wrong; the message explains what.
    Failed,
}

/// Progress of the running installation, shared with the GUI.
#[derive(Debug, Clone, Default)]
pub struct InstallState {
    pub stage: InstallStage,
    /// Download progress in percent (0-100).
    pub percent: u8,
    /// Human readable status line.
    pub message: String,
}

impl InstallState {
    pub fn is_running(&self) -> bool {
        !matches!(
            self.stage,
            InstallStage::Idle | InstallStage::Done | InstallStage::Failed
        )
    }
}

/// Shared handle to the installation progress.
pub type SharedInstallState = Arc<Mutex<InstallState>>;

/// Updates the shared progress state.
fn publish(state: &SharedInstallState, stage: InstallStage, percent: u8, message: &str) {
    if let Ok(mut guard) = state.lock() {
        guard.stage = stage;
        guard.percent = percent;
        guard.message = message.to_string();
    }
}

/// Where the downloaded archives are kept.
fn staging_dir() -> PathBuf {
    crate::paths::ensure_dir(crate::paths::data_dir().join("updates"))
}

/// Downloads `url` into `dest`, reporting progress as a percentage.
///
/// A dedicated client is used instead of the shared one: the shared client's
/// connection pool is bound to the runtime that created it, and the download
/// needs a timeout far above the 30 seconds of the API client.
async fn download(url: &str, dest: &Path, state: &SharedInstallState) -> Result<u64> {
    let client = reqwest::Client::builder()
        .user_agent(crate::http::USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(600))
        .build()
        .context("Could not create the download client")?;
    let mut response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("Could not download {url}"))?;

    if !response.status().is_success() {
        bail!("Download failed: HTTP {}", response.status());
    }

    let total = response.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(dest)
        .await
        .with_context(|| format!("Could not create {}", dest.display()))?;

    let mut written: u64 = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .with_context(|| format!("Download of {url} was interrupted"))?
    {
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
        written += chunk.len() as u64;
        let percent = if total > 0 {
            ((written as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as u8
        } else {
            0
        };
        if let Ok(mut guard) = state.lock() {
            guard.percent = percent;
        }
    }
    tokio::io::AsyncWriteExt::flush(&mut file).await?;
    Ok(written)
}

/// Parses a `sha256sum` style manifest into `file name -> digest`.
fn parse_checksums(text: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(digest), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if digest.len() != 64 || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        // The manifest lists the names as they appear in the archive
        // ("*./wflow-0.3.5.tar.gz"), so the "./" prefix has to go.
        let name = name.trim_start_matches('*').trim_start_matches("./");
        map.insert(name.to_ascii_lowercase(), digest.to_ascii_lowercase());
    }
    map
}

/// Computes the SHA-256 of a file.
fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};

    let mut file =
        std::fs::File::open(path).with_context(|| format!("Could not open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Extracts the application binary from a `.tar.gz` release archive.
///
/// Returns the path of the extracted file.
#[cfg(not(target_os = "windows"))]
fn extract_binary(archive: &Path, dest_dir: &Path) -> Result<PathBuf> {
    let file = std::fs::File::open(archive)
        .with_context(|| format!("Could not open {}", archive.display()))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);

    for entry in tar
        .entries()
        .context("Could not read the release archive")?
    {
        let mut entry = entry.context("Could not read a release archive entry")?;
        let path = entry
            .path()
            .context("Invalid release archive entry")?
            .to_path_buf();
        let is_binary = path
            .file_name()
            .map(|name| name == "wflow" || name == "wflow.exe")
            .unwrap_or(false);
        if !is_binary || !entry.header().entry_type().is_file() {
            continue;
        }
        let target = dest_dir.join("wflow.new");
        let mut out = std::fs::File::create(&target)
            .with_context(|| format!("Could not create {}", target.display()))?;
        std::io::copy(&mut entry, &mut out).context("Could not extract the application binary")?;
        // Flush to disk and close the file before it is executed: an open
        // write handle makes the kernel refuse to run it.
        out.sync_all().ok();
        drop(out);
        make_executable(&target)?;
        return Ok(target);
    }
    bail!("The release archive does not contain the wflow binary")
}

/// Marks a file as executable.
///
/// Only the self-replacing installation needs this; Windows runs the Inno
/// Setup installer instead.
#[cfg(not(target_os = "windows"))]
fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Asks a binary for its version, e.g. `wflow 0.3.6`.
///
/// Executing a file that was written a moment ago can fail with `ETXTBSY`
/// ("Text file busy"): while another thread forks, the child inherits the open
/// write handle of the file, and the kernel then refuses to run it. The
/// handle is gone as soon as the child execs, so a short retry is enough.
#[cfg(not(target_os = "windows"))]
fn binary_version(path: &Path) -> Result<String> {
    const ATTEMPTS: usize = 10;

    let mut last_error = None;
    for attempt in 0..ATTEMPTS {
        match std::process::Command::new(path).arg("--version").output() {
            Ok(output) => {
                if !output.status.success() {
                    bail!("The downloaded binary does not run");
                }
                return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
            }
            Err(err) if err.raw_os_error() == Some(26) => {
                // `ETXTBSY`: the file is still held open by a forked child.
                last_error = Some(err);
                std::thread::sleep(Duration::from_millis(20 * (attempt as u64 + 1)));
            }
            Err(err) => {
                return Err(err).with_context(|| format!("Could not run {}", path.display()));
            }
        }
    }

    Err(anyhow!(
        "Could not run {}: {}",
        path.display(),
        last_error
            .map(|err| err.to_string())
            .unwrap_or_else(|| "Text file busy".to_string())
    ))
}

/// Replaces `target` with `source`, keeping a backup of the old file.
///
/// The replacement happens through a rename so the running process keeps its
/// open file handle and the new binary appears atomically.
#[cfg(not(target_os = "windows"))]
fn replace_executable(target: &Path, source: &Path, backup: &Path) -> Result<()> {
    let _ = std::fs::remove_file(backup);
    if let Some(parent) = backup.parent() {
        crate::paths::ensure_dir(parent.to_path_buf());
    }
    // The backup is best effort: a read-only directory must not block the
    // update itself.
    let _ = std::fs::copy(target, backup);

    // The staged file lives next to the target (a rename across file systems
    // would fail) and has a name of its own, so it can never collide with
    // the source file.
    let file_name = target
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "wflow".to_string());
    let staged = target.with_file_name(format!(".{file_name}.{}.new", std::process::id()));
    std::fs::copy(source, &staged)
        .with_context(|| format!("Could not stage the new binary at {}", staged.display()))?;
    make_executable(&staged)?;
    std::fs::rename(&staged, target).with_context(|| {
        format!(
            "Could not replace {} with the new version",
            target.display()
        )
    })?;
    Ok(())
}

/// Replaces `target` with `source` using the desktop privilege prompt.
#[cfg(not(target_os = "windows"))]
fn replace_executable_privileged(target: &Path, source: &Path) -> Result<()> {
    let status = std::process::Command::new("pkexec")
        .arg("install")
        .arg("-m")
        .arg("0755")
        .arg(source)
        .arg(target)
        .status()
        .context("Could not run pkexec (is polkit installed?)")?;
    if !status.success() {
        bail!("The privileged replacement was refused or failed");
    }
    Ok(())
}

/// The `wflow` a user install would leave in `~/.local/bin`.
///
/// Such a copy shadows `/usr/bin/wflow` in `PATH`, so an update has to refresh
/// it as well. `None` is returned when there is no home directory, when the
/// copy does not exist, or when the running binary already *is* that copy.
#[cfg(not(target_os = "windows"))]
fn user_local_copy(home: Option<&Path>, running: &Path) -> Option<PathBuf> {
    let local = home?.join(".local/bin/wflow");

    // Nothing to do when the copy is missing or is not a regular file.
    let metadata = std::fs::symlink_metadata(&local).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let same = match (
        std::fs::canonicalize(&local),
        std::fs::canonicalize(running),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        return None;
    }
    Some(local)
}

/// The home directory of the user running the app.
#[cfg(not(target_os = "windows"))]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Copies the freshly installed binary over a second location.
///
/// The copy is written next to the target and moved into place, so the
/// replacement is atomic and works even when the existing file is owned by
/// root but its directory is writable by the user.
#[cfg(not(target_os = "windows"))]
fn sync_copy(installed: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent directory", destination.display()))?;

    let file_name = destination
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "wflow".to_string());
    let staged = parent.join(format!(".{file_name}.{}.new", std::process::id()));

    std::fs::copy(installed, &staged)
        .with_context(|| format!("Could not write {}", staged.display()))?;
    make_executable(&staged)?;

    if let Err(err) = std::fs::rename(&staged, destination) {
        // Do not leave the staged copy behind if the move is refused.
        let _ = std::fs::remove_file(&staged);
        return Err(err).with_context(|| format!("Could not replace {}", destination.display()));
    }
    Ok(())
}

/// Schedules a restart of the application.
///
/// A small shell helper waits until the current process is gone and then
/// starts the (new) executable, so the single-instance lock is free again.
#[cfg(not(target_os = "windows"))]
fn schedule_restart(executable: &Path) -> Result<()> {
    let pid = std::process::id();
    let script = format!(
        "while kill -0 {pid} 2>/dev/null; do sleep 0.2; done; exec {}",
        shell_quote(&executable.to_string_lossy())
    );

    let mut command = std::process::Command::new("sh");
    command
        .arg("-c")
        .arg(&script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group, so the helper is not killed together with
        // the app and does not receive the terminal's Ctrl+C.
        command.process_group(0);
    }

    command.spawn().context("Could not schedule the restart")?;
    Ok(())
}

/// Quotes a string for use in a POSIX shell.
#[cfg(not(target_os = "windows"))]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Picks the asset that should be downloaded for the current platform.
///
/// Returns `None` when the release has nothing that can be installed
/// automatically (for example a macOS-only release).
#[cfg(not(target_os = "windows"))]
pub fn install_asset(info: &UpdateInfo) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        info.archive_url.clone()
    }
    #[cfg(not(target_os = "linux"))]
    {
        info.archive_url.clone()
    }
}

/// The file name of an asset, used to look it up in the checksum manifest.
fn asset_name(url: &str) -> String {
    url.rsplit('/').next().unwrap_or(url).to_ascii_lowercase()
}

/// Installs an update and restarts the application.
///
/// The caller is responsible for showing the progress in `state` and for
/// quitting the app once this function returns successfully. When `restart`
/// is false the new build is installed but the app is left running, which is
/// what the command line installer uses.
pub async fn install_update(
    info: &UpdateInfo,
    state: &SharedInstallState,
    restart: bool,
) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        let _ = restart;
        install_windows(info, state).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        install_self_replacing(info, state, restart).await
    }
}

/// Windows cannot replace a running `.exe`, so the Inno Setup installer is
/// downloaded and launched instead. It restarts the app on its own.
#[cfg(target_os = "windows")]
async fn install_windows(info: &UpdateInfo, state: &SharedInstallState) -> Result<()> {
    let asset_url = info
        .windows_setup_url
        .clone()
        .ok_or_else(|| anyhow!("This release has no Windows installer"))?;
    let asset_name = asset_name(&asset_url);

    let dir = staging_dir();
    let installer_path = dir.join(&asset_name);

    publish(state, InstallStage::Downloading, 0, "กำลังดาวน์โหลดอัปเดต...");
    download(&asset_url, &installer_path, state).await?;

    publish(
        state,
        InstallStage::Verifying,
        100,
        "กำลังตรวจสอบไฟล์อัปเดต...",
    );
    verify_checksum(info, &asset_name, &installer_path, state, &dir).await?;

    publish(state, InstallStage::Installing, 100, "กำลังติดตั้งอัปเดต...");
    // `/VERYSILENT` keeps the user in the flow; the installer closes the
    // running app and starts the new version itself.
    std::process::Command::new(&installer_path)
        .arg("/VERYSILENT")
        .arg("/SUPPRESSMSGBOXES")
        .arg("/NORESTART")
        .spawn()
        .context("Could not start the installer")?;

    publish(
        state,
        InstallStage::Done,
        100,
        &format!("ติดตั้ง v{} เรียบร้อยแล้ว", info.latest),
    );
    Ok(())
}

/// Verifies a downloaded file against the release checksum manifest.
async fn verify_checksum(
    info: &UpdateInfo,
    asset_name: &str,
    file: &Path,
    state: &SharedInstallState,
    staging: &Path,
) -> Result<()> {
    let checksums_url = info
        .checksums_url
        .as_ref()
        .ok_or_else(|| anyhow!("The release does not publish a checksum manifest"))?;
    let manifest_path = staging.join("SHA256SUMS.txt");
    download(checksums_url, &manifest_path, state).await?;
    let manifest = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("Could not read {}", manifest_path.display()))?;
    let expected = parse_checksums(&manifest)
        .get(asset_name)
        .cloned()
        .ok_or_else(|| anyhow!("The release does not publish a checksum for {asset_name}"))?;
    let actual = sha256_file(file)?;
    if actual != expected {
        bail!("The downloaded file is corrupted (checksum mismatch)");
    }
    Ok(())
}

/// Replaces the running binary with the downloaded one and restarts the app.
#[cfg(not(target_os = "windows"))]
async fn install_self_replacing(
    info: &UpdateInfo,
    state: &SharedInstallState,
    restart: bool,
) -> Result<()> {
    let executable = std::env::current_exe().context("Could not locate the running binary")?;
    let staging = staging_dir();
    let home = home_dir();
    install_into(info, state, restart, &executable, &staging, home.as_deref()).await
}

/// The actual installation: download, verify, unpack and replace `target`.
///
/// `staging` is the directory the downloads are kept in and `home` is the
/// user's home directory. Both are parameters so that the tests work on their
/// own copies and can never touch the real installation.
#[cfg(not(target_os = "windows"))]
async fn install_into(
    info: &UpdateInfo,
    state: &SharedInstallState,
    restart: bool,
    target: &Path,
    staging: &Path,
    home: Option<&Path>,
) -> Result<()> {
    let asset_url = install_asset(info)
        .ok_or_else(|| anyhow!("This release has no archive for this platform"))?;
    let asset_name = asset_name(&asset_url);

    let dir = crate::paths::ensure_dir(staging.to_path_buf());
    let archive_path = dir.join(&asset_name);

    // --- download -------------------------------------------------------
    publish(state, InstallStage::Downloading, 0, "กำลังดาวน์โหลดอัปเดต...");
    download(&asset_url, &archive_path, state).await?;

    // --- verify ---------------------------------------------------------
    publish(
        state,
        InstallStage::Verifying,
        100,
        "กำลังตรวจสอบไฟล์อัปเดต...",
    );
    verify_checksum(info, &asset_name, &archive_path, state, &dir).await?;

    // --- install --------------------------------------------------------
    publish(state, InstallStage::Installing, 100, "กำลังติดตั้งอัปเดต...");

    let new_binary = extract_binary(&archive_path, &dir)?;

    let reported = binary_version(&new_binary)?;
    if !reported.contains(&info.latest.to_string()) {
        bail!(
            "The downloaded build reports version \"{reported}\" instead of {}",
            info.latest
        );
    }

    let backup = dir.join(format!("wflow-{}.bak", info.current));

    let replaced = replace_executable(target, &new_binary, &backup);
    if let Err(err) = replaced {
        // A package install puts the binary in a root-owned directory, so the
        // replacement has to be done with elevated rights.
        tracing::warn!("Direct replacement failed ({err:#}), trying pkexec");
        replace_executable_privileged(target, &new_binary)?;
    }

    // The new binary must really be in place.
    let installed = binary_version(target)?;
    if !installed.contains(&info.latest.to_string()) {
        bail!("The new version was not installed correctly");
    }

    // A copy in `~/.local/bin` takes precedence over `/usr/bin` in `PATH`, so
    // a stale one would keep the user on the old version for ever. This is
    // best effort: a failure here must not fail the update itself.
    if let Some(local) = user_local_copy(home, target) {
        match sync_copy(target, &local) {
            Ok(()) => tracing::info!("Refreshed the user-local copy at {}", local.display()),
            Err(err) => tracing::warn!("Could not refresh {}: {err:#}", local.display()),
        }
    }

    // --- restart --------------------------------------------------------
    if restart {
        publish(state, InstallStage::Restarting, 100, "กำลังเริ่มโปรแกรมใหม่...");
        schedule_restart(target)?;
    }

    publish(
        state,
        InstallStage::Done,
        100,
        &format!("ติดตั้ง v{} เรียบร้อยแล้ว", info.latest),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_checksum_manifest() {
        let tar_digest = "a".repeat(64);
        let deb_digest = "b".repeat(64);
        let manifest = format!(
            "{tar_digest}  ./wflow-0.3.6-linux-x86_64.tar.gz\n\
             {deb_digest}  *wflow_0.3.6_amd64.deb\n\
             not-a-hash  file\n"
        );
        let map = parse_checksums(&manifest);
        assert_eq!(
            map.get("wflow-0.3.6-linux-x86_64.tar.gz")
                .map(String::as_str),
            Some(tar_digest.as_str())
        );
        assert_eq!(
            map.get("wflow_0.3.6_amd64.deb").map(String::as_str),
            Some(deb_digest.as_str())
        );
        assert!(!map.contains_key("file"));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn extracts_the_binary_from_a_release_archive() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("release.tar.gz");

        // Build a release-shaped archive.
        {
            let file = std::fs::File::create(&archive).unwrap();
            let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
            let mut builder = tar::Builder::new(encoder);
            let payload = b"#!/bin/sh\necho test\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "./wflow", &payload[..])
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        }

        let extracted = extract_binary(&archive, dir.path()).unwrap();
        assert!(extracted.exists());
        let contents = std::fs::read(&extracted).unwrap();
        assert_eq!(contents, b"#!/bin/sh\necho test\n");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn replaces_a_binary_and_keeps_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("wflow");
        let source = dir.path().join("wflow.new");
        let backup = dir.path().join("backup/wflow.bak");

        std::fs::write(&target, b"old").unwrap();
        std::fs::write(&source, b"new").unwrap();

        replace_executable(&target, &source, &backup).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(std::fs::read(&backup).unwrap(), b"old");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn quotes_shell_arguments() {
        assert_eq!(shell_quote("/tmp/a b"), "'/tmp/a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    /// A stale copy in `~/.local/bin` is found and refreshed, so that it can
    /// no longer shadow the packaged binary in `PATH`.
    #[test]
    #[cfg(not(target_os = "windows"))]
    fn refreshes_a_user_local_copy() {
        let home = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("installed");
        let local = home.path().join(".local/bin/wflow");
        std::fs::create_dir_all(local.parent().unwrap()).unwrap();

        std::fs::write(&installed, b"new build").unwrap();
        std::fs::write(&local, b"old build").unwrap();
        make_executable(&installed).unwrap();

        // A stale copy is detected...
        let found = user_local_copy(Some(home.path()), &installed);
        assert_eq!(found.as_deref(), Some(local.as_path()));

        // ...and then refreshed in place.
        sync_copy(&installed, &local).unwrap();
        assert_eq!(std::fs::read(&local).unwrap(), b"new build");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&local).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "the copy must stay executable");
        }

        // No staged file is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(local.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".new"))
            .collect();
        assert!(leftovers.is_empty(), "staged file left behind");
    }

    /// Nothing to do when there is no user-local copy at all.
    #[test]
    #[cfg(not(target_os = "windows"))]
    fn ignores_a_missing_user_local_copy() {
        let home = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let running = dir.path().join("wflow");
        std::fs::write(&running, b"build").unwrap();

        assert!(user_local_copy(Some(home.path()), &running).is_none());
    }

    /// A copy that *is* the running binary is not touched again.
    #[test]
    #[cfg(not(target_os = "windows"))]
    fn skips_a_copy_that_is_the_running_binary() {
        let home = tempfile::tempdir().unwrap();
        let local = home.path().join(".local/bin/wflow");
        std::fs::create_dir_all(local.parent().unwrap()).unwrap();
        std::fs::write(&local, b"build").unwrap();

        assert!(
            user_local_copy(Some(home.path()), &local).is_none(),
            "the running binary is not a shadowing copy"
        );
    }

    /// Without a home directory nothing is synced.
    #[test]
    #[cfg(not(target_os = "windows"))]
    fn sync_needs_a_home_directory() {
        let dir = tempfile::tempdir().unwrap();
        let running = dir.path().join("wflow");
        std::fs::write(&running, b"build").unwrap();

        assert!(user_local_copy(None, &running).is_none());
    }

    /// Serves a fixed set of files over HTTP, so the download path can be
    /// tested without touching the network.
    #[cfg(not(target_os = "windows"))]
    struct FileServer {
        address: String,
    }

    #[cfg(not(target_os = "windows"))]
    impl FileServer {
        /// Serves the given bodies; the path has to contain the prefix.
        fn start_with(bodies: Vec<(String, Vec<u8>)>) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let address = format!("http://{}", listener.local_addr().unwrap());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    use std::io::Read;
                    let mut request = [0_u8; 2048];
                    let read = stream.read(&mut request).unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..read]).to_string();
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/")
                        .to_string();
                    let body = bodies
                        .iter()
                        .find(|(prefix, _)| path.contains(prefix.as_str()))
                        .map(|(_, body)| body.clone())
                        .unwrap_or_default();
                    use std::io::Write;
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&body);
                }
            });
            Self { address }
        }

        /// The URL of a file served by this server.
        fn url(&self, name: &str) -> String {
            format!("{}/{}", self.address, name)
        }
    }

    /// A release archive that contains a script reporting the given version.
    #[cfg(not(target_os = "windows"))]
    fn build_release_archive(path: &Path, version: &str) {
        let payload = format!("#!/bin/sh\necho \"wflow {version}\"\n");
        let file = std::fs::File::create(path).unwrap();
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(payload.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "./wflow", payload.as_bytes())
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
    }

    #[tokio::test]
    #[cfg(not(target_os = "windows"))]
    async fn installs_a_new_version_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("wflow-9.9.9-linux-x86_64.tar.gz");
        build_release_archive(&archive, "9.9.9");
        let archive_body = std::fs::read(&archive).unwrap();
        let digest = sha256_file(&archive).unwrap();

        let manifest = format!("{digest}  ./wflow-9.9.9-linux-x86_64.tar.gz\n");
        let server = FileServer::start_with(vec![
            ("wflow-9.9.9-linux-x86_64.tar.gz".to_string(), archive_body),
            ("SHA256SUMS".to_string(), manifest.into_bytes()),
        ]);

        let target = dir.path().join("wflow");
        std::fs::write(&target, "#!/bin/sh\necho \"wflow 1.0.0\"\n").unwrap();
        make_executable(&target).unwrap();

        let info = UpdateInfo {
            current: semver::Version::new(1, 0, 0),
            latest: semver::Version::new(9, 9, 9),
            title: "test".into(),
            notes: String::new(),
            release_url: server.url("release"),
            download_url: server.url("wflow-9.9.9-linux-x86_64.tar.gz"),
            archive_url: Some(server.url("wflow-9.9.9-linux-x86_64.tar.gz")),
            deb_url: None,
            windows_setup_url: None,
            checksums_url: Some(server.url("SHA256SUMS.txt")),
            published_at: None,
            prerelease: false,
        };

        let state: SharedInstallState = Arc::new(Mutex::new(InstallState::default()));
        // A sandboxed home keeps the test away from the real installation.
        let home = tempfile::tempdir().unwrap();
        // A stale user-local copy that would shadow the new binary in PATH.
        let local = home.path().join(".local/bin/wflow");
        std::fs::create_dir_all(local.parent().unwrap()).unwrap();
        std::fs::write(&local, "#!/bin/sh\necho \"wflow 1.0.0\"\n").unwrap();
        make_executable(&local).unwrap();

        install_into(&info, &state, false, &target, dir.path(), Some(home.path()))
            .await
            .unwrap();

        // The new binary is in place and reports the new version.
        let installed = binary_version(&target).unwrap();
        assert!(installed.contains("9.9.9"), "installed: {installed}");
        // The user-local copy was refreshed as well, so it can no longer
        // shadow the package in PATH.
        let local_version = binary_version(&local).unwrap();
        assert!(
            local_version.contains("9.9.9"),
            "the user-local copy was not refreshed: {local_version}"
        );
        // The progress ended in the "done" state.
        let guard = state.lock().unwrap();
        assert_eq!(guard.stage, InstallStage::Done);
        assert_eq!(guard.percent, 100);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn a_freshly_written_binary_can_be_executed() {
        // Executing a file that was just written can fail with ETXTBSY while
        // another thread forks. The version check has to survive that.
        let handles: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(move || {
                    for _ in 0..25 {
                        let dir = tempfile::tempdir().unwrap();
                        let archive = dir.path().join("release.tar.gz");
                        build_release_archive(&archive, "9.9.9");
                        let extracted = extract_binary(&archive, dir.path()).unwrap();
                        let version = binary_version(&extracted).unwrap();
                        assert!(version.contains("9.9.9"), "version: {version}");
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[tokio::test]
    #[cfg(not(target_os = "windows"))]
    async fn refuses_a_corrupted_archive() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("wflow-9.9.9-linux-x86_64.tar.gz");
        build_release_archive(&archive, "9.9.9");
        let archive_body = std::fs::read(&archive).unwrap();

        // The manifest publishes a digest that does not match the download.
        let manifest = format!("{}  ./wflow-9.9.9-linux-x86_64.tar.gz\n", "c".repeat(64));
        let server = FileServer::start_with(vec![
            ("wflow-9.9.9-linux-x86_64.tar.gz".to_string(), archive_body),
            ("SHA256SUMS".to_string(), manifest.into_bytes()),
        ]);

        let target = dir.path().join("wflow");
        std::fs::write(&target, "#!/bin/sh\necho \"wflow 1.0.0\"\n").unwrap();
        make_executable(&target).unwrap();

        let info = UpdateInfo {
            current: semver::Version::new(1, 0, 0),
            latest: semver::Version::new(9, 9, 9),
            title: "test".into(),
            notes: String::new(),
            release_url: server.url("release"),
            download_url: server.url("wflow-9.9.9-linux-x86_64.tar.gz"),
            archive_url: Some(server.url("wflow-9.9.9-linux-x86_64.tar.gz")),
            deb_url: None,
            windows_setup_url: None,
            checksums_url: Some(server.url("SHA256SUMS.txt")),
            published_at: None,
            prerelease: false,
        };

        let state: SharedInstallState = Arc::new(Mutex::new(InstallState::default()));
        let home = tempfile::tempdir().unwrap();
        let error = install_into(&info, &state, false, &target, dir.path(), Some(home.path()))
            .await
            .expect_err("a corrupted archive must be rejected");
        assert!(
            error.to_string().contains("corrupted"),
            "unexpected error: {error}"
        );
        // The old binary is untouched.
        assert!(binary_version(&target).unwrap().contains("1.0.0"));
    }
}
