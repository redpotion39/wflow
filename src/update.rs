//! Update checker backed by GitHub Releases.
//!
//! The check is read-only and never downloads or executes anything: it reports
//! the newest version plus a link the user can open manually.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use semver::Version;
use serde::Deserialize;
use std::time::Duration;

pub const DEFAULT_REPO: &str = "redpotion39/wflow";

fn repo() -> String {
    crate::secrets::env_non_empty("WFLOW_UPDATE_REPO").unwrap_or_else(|| DEFAULT_REPO.to_string())
}

pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub current: Version,
    pub latest: Version,
    pub title: String,
    pub notes: String,
    pub release_url: String,
    /// Installer for the current platform, opened in the browser.
    pub download_url: String,
    /// Portable archive with the raw binary; used for the self-update.
    pub archive_url: Option<String>,
    /// Debian package; used when the running binary cannot be replaced.
    pub deb_url: Option<String>,
    /// Windows installer (Inno Setup); launched instead of a self-replace.
    pub windows_setup_url: Option<String>,
    /// Checksum manifest of the release.
    pub checksums_url: Option<String>,
    pub published_at: Option<String>,
    pub prerelease: bool,
}

impl UpdateInfo {
    pub fn is_newer(&self) -> bool {
        self.latest > self.current
    }
}

#[derive(Debug, Clone, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

fn parse_tag(tag: &str) -> Option<Version> {
    let trimmed = tag.trim().trim_start_matches(['v', 'V']);
    Version::parse(trimmed).ok()
}

fn asset_priority(name: &str) -> Option<u8> {
    let name = name.to_ascii_lowercase();
    let candidates: [(&str, u8); 9] = [
        (".deb", 0),
        (".run", 1),
        ("setup.exe", 2),
        (".msi", 3),
        (".appimage", 4),
        (".dmg", 5),
        (".tar.gz", 6),
        (".zip", 7),
        (".exe", 8),
    ];
    candidates
        .iter()
        .find(|(suffix, _)| name.ends_with(suffix))
        .map(|(_, priority)| *priority)
}

fn platform_rank(name: &str) -> Option<u8> {
    let name = name.to_ascii_lowercase();
    #[cfg(target_os = "windows")]
    {
        if name.contains("setup.exe") || name.ends_with(".msi") || name.ends_with(".exe") {
            return Some(0);
        }
        if name.ends_with(".zip") {
            return Some(1);
        }
    }
    #[cfg(target_os = "linux")]
    {
        if name.ends_with(".deb") || name.ends_with(".run") || name.ends_with(".appimage") {
            return Some(0);
        }
        if name.ends_with(".tar.gz") {
            return Some(1);
        }
    }
    #[cfg(target_os = "macos")]
    {
        if name.ends_with(".dmg") {
            return Some(0);
        }
    }
    let _ = &name;
    None
}

fn pick_download(release: &GithubRelease) -> String {
    let mut best: Option<(&GithubAsset, (u8, u8))> = None;
    for asset in &release.assets {
        let Some(rank) = platform_rank(&asset.name) else {
            continue;
        };
        let Some(kind) = asset_priority(&asset.name) else {
            continue;
        };
        let score = (rank, kind);
        if best.map(|(_, current)| score < current).unwrap_or(true) {
            best = Some((asset, score));
        }
    }
    best.map(|(asset, _)| asset.browser_download_url.clone())
        .unwrap_or_else(|| release.html_url.clone())
}

/// Finds the first asset whose name matches the predicate.
fn find_asset<F: Fn(&str) -> bool>(release: &GithubRelease, predicate: F) -> Option<String> {
    release
        .assets
        .iter()
        .find(|asset| predicate(&asset.name))
        .map(|asset| asset.browser_download_url.clone())
}

/// The portable archive holding the raw binary (`wflow-<version>-linux-x86_64.tar.gz`).
///
/// Only archives built for the current platform are picked, so a Linux build
/// is never offered to a macOS or Windows user.
fn find_archive(release: &GithubRelease) -> Option<String> {
    let platform = platform_tag();
    find_asset(release, |name| {
        let name = name.to_ascii_lowercase();
        if !(name.ends_with(".tar.gz") || name.ends_with(".tgz")) {
            return false;
        }
        match platform {
            Some(tag) => name.contains(tag),
            None => true,
        }
    })
}

/// The platform tag used in the release asset names.
fn platform_tag() -> Option<&'static str> {
    #[cfg(target_os = "linux")]
    {
        Some("linux")
    }
    #[cfg(target_os = "windows")]
    {
        Some("windows")
    }
    #[cfg(target_os = "macos")]
    {
        Some("macos")
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        None
    }
}

/// The Debian package, used when the running binary may not be replaced.
fn find_deb(release: &GithubRelease) -> Option<String> {
    find_asset(release, |name| name.to_ascii_lowercase().ends_with(".deb"))
}

/// The Windows installer (Inno Setup).
fn find_windows_setup(release: &GithubRelease) -> Option<String> {
    find_asset(release, |name| {
        let name = name.to_ascii_lowercase();
        name.ends_with(".exe") && name.contains("setup")
    })
}

/// The checksum manifest that covers every other asset.
fn find_checksums(release: &GithubRelease) -> Option<String> {
    find_asset(release, |name| {
        let name = name.to_ascii_lowercase();
        name.starts_with("sha256sums") || name == "sha256sums.txt"
    })
}

fn select_update(releases: Vec<GithubRelease>, include_prerelease: bool) -> Option<UpdateInfo> {
    let current = current_version();
    let mut candidates: Vec<(Version, GithubRelease)> = releases
        .into_iter()
        .filter(|r| !r.draft && (include_prerelease || !r.prerelease))
        .filter_map(|r| parse_tag(&r.tag_name).map(|v| (v, r)))
        .collect();

    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let (latest, release) = candidates.into_iter().next()?;
    if latest <= current {
        return None;
    }

    Some(UpdateInfo {
        current,
        latest,
        title: release
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| release.tag_name.clone()),
        notes: release.body.clone().unwrap_or_default(),
        download_url: pick_download(&release),
        archive_url: find_archive(&release),
        deb_url: find_deb(&release),
        windows_setup_url: find_windows_setup(&release),
        checksums_url: find_checksums(&release),
        release_url: release.html_url,
        published_at: release.published_at,
        prerelease: release.prerelease,
    })
}

/// How long the whole update check may take before it is given up on.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// Fetches the release feed and returns an update when one is available.
///
/// The check is bounded by a timeout so the UI can never get stuck on
/// "checking…" when the network is slow or blocked.
pub async fn check(include_prerelease: bool) -> Result<Option<UpdateInfo>> {
    match tokio::time::timeout(CHECK_TIMEOUT, check_inner(include_prerelease)).await {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!(
            "การตรวจสอบอัปเดตใช้เวลานานเกินไป (หมดเวลา {} วินาที) — ตรวจสอบการเชื่อมต่ออินเทอร์เน็ต",
            CHECK_TIMEOUT.as_secs()
        )),
    }
}

async fn check_inner(include_prerelease: bool) -> Result<Option<UpdateInfo>> {
    let url = format!(
        "https://api.github.com/repos/{}/releases?per_page=30",
        repo()
    );
    let client = crate::http::client();
    let token = crate::secrets::env_non_empty("WFLOW_GITHUB_TOKEN");
    let response = crate::http::send_with_retry("GitHub releases", 3, || {
        let mut request = client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = token.as_ref() {
            request = request.header("Authorization", format!("Bearer {}", token));
        }
        request.timeout(Duration::from_secs(10))
    })
    .await?;

    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        anyhow::bail!(
            "ไม่พบ repository สำหรับตรวจสอบอัปเดต ({}) — อาจเป็น repo แบบ private หรือยังไม่มี Release (ตั้ง WFLOW_GITHUB_TOKEN ได้)",
            repo()
        );
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        anyhow::bail!(
            "GitHub ปฏิเสธการตรวจสอบอัปเดต ({}) — ตรวจสอบ WFLOW_GITHUB_TOKEN",
            status
        );
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "ตรวจสอบอัปเดตไม่สำเร็จ (HTTP {}): {}",
            status,
            body.chars().take(200).collect::<String>()
        );
    }

    let releases: Vec<GithubRelease> = response
        .json()
        .await
        .context("Failed to parse GitHub releases response")?;
    Ok(select_update(releases, include_prerelease))
}

/// Returns true when the last check is missing, unparseable, or older than the
/// configured interval.
pub fn should_check(last_check: Option<&str>, interval_hours: u64) -> bool {
    let Some(raw) = last_check else {
        return true;
    };
    let Ok(parsed) = DateTime::parse_from_rfc3339(raw) else {
        return true;
    };
    let elapsed = Utc::now().signed_duration_since(parsed.with_timezone(&Utc));
    elapsed.num_hours() >= interval_hours.max(1) as i64
}

pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool, assets: &[(&str, &str)]) -> GithubRelease {
        GithubRelease {
            tag_name: tag.to_string(),
            name: Some(format!("Wflow {}", tag)),
            body: Some("- notes".to_string()),
            html_url: format!("https://example.test/{tag}"),
            published_at: None,
            prerelease,
            draft: false,
            assets: assets
                .iter()
                .map(|(name, url)| GithubAsset {
                    name: (*name).to_string(),
                    browser_download_url: (*url).to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn parses_prefixed_tags() {
        assert_eq!(parse_tag("v1.2.3").unwrap(), Version::new(1, 2, 3));
        assert_eq!(parse_tag("1.2.3").unwrap(), Version::new(1, 2, 3));
        assert!(parse_tag("nightly").is_none());
    }

    #[test]
    fn skips_prereleases_unless_requested() {
        let releases = vec![
            release("v9.0.0-beta.1", true, &[]),
            release("v1.0.0", false, &[]),
        ];
        assert_eq!(
            select_update(releases.clone(), false).unwrap().latest,
            Version::new(1, 0, 0)
        );
        let latest = select_update(releases, true).unwrap().latest;
        assert_eq!(latest.major, 9);
        assert!(!latest.pre.is_empty());
    }

    #[test]
    fn returns_none_when_current_is_latest() {
        let releases = vec![release("v0.0.1", false, &[])];
        assert!(select_update(releases, false).is_none());
    }

    #[test]
    fn picks_platform_asset() {
        let releases = vec![release(
            "v99.0.0",
            false,
            &[
                ("wflow-99-linux.tar.gz", "https://example.test/tar"),
                ("wflow_99_amd64.deb", "https://example.test/deb"),
            ],
        )];
        let info = select_update(releases, false).unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(info.download_url, "https://example.test/deb");
        #[cfg(not(target_os = "linux"))]
        assert_eq!(info.download_url, "https://example.test/v99.0.0");
    }

    #[test]
    fn interval_is_respected() {
        assert!(should_check(None, 24));
        assert!(should_check(Some("garbage"), 24));
        let now = Utc::now().to_rfc3339();
        assert!(!should_check(Some(&now), 24));
        let old = (Utc::now() - chrono::Duration::hours(48)).to_rfc3339();
        assert!(should_check(Some(&old), 24));
    }
}
