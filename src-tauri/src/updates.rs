//! Lightweight GitHub-Releases-based update check.
//!
//! We hit `https://api.github.com/repos/<slug>/releases/latest`, parse the
//! `tag_name` / asset list, and tell the UI whether a newer version is
//! published. Intentionally minimal — no auto-download, no signature
//! verification. When an update is available the UI links the user to the
//! release page (via `tauri-plugin-opener`) and they re-run the installer.
//!
//! Any HTTP/parse failure degrades silently to `update_available: false` so a
//! network blip or an empty releases page never shows up as an error badge.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Owner/repo on GitHub — update this if the repo moves.
const GITHUB_SLUG: &str = "honear/offspring";

/// HTTP timeout. GitHub's API is usually sub-second; if we hit more than 5s
/// the user's probably offline and we'd rather fail fast.
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Serialize, Clone, Debug, Default)]
pub struct UpdateInfo {
    /// Running version from `CARGO_PKG_VERSION` (e.g. "0.2.0").
    pub current: String,
    /// Latest published version with the leading `v` stripped, or empty if
    /// the check failed.
    pub latest: String,
    /// True iff `latest` is strictly greater than `current`.
    pub update_available: bool,
    /// Release landing page — what we open when the user clicks the banner.
    pub html_url: String,
    /// Direct .exe asset URL if we could find an `Offspring-Setup-*.exe`
    /// attached to the release. Empty otherwise.
    pub installer_url: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[tauri::command]
pub fn check_for_updates() -> UpdateInfo {
    let current = env!("CARGO_PKG_VERSION").to_string();
    match fetch_latest() {
        Ok(rel) if !rel.draft && !rel.prerelease => {
            let latest = rel.tag_name.trim_start_matches('v').to_string();
            let update_available = is_newer(&latest, &current);
            let installer_url = rel
                .assets
                .iter()
                .find(|a| {
                    let n = a.name.to_ascii_lowercase();
                    n.starts_with("offspring-setup") && n.ends_with(".exe")
                })
                .map(|a| a.browser_download_url.clone())
                .unwrap_or_default();
            UpdateInfo {
                current,
                latest,
                update_available,
                html_url: rel.html_url,
                installer_url,
            }
        }
        _ => UpdateInfo {
            current,
            ..Default::default()
        },
    }
}

fn fetch_latest() -> anyhow::Result<GhRelease> {
    let url = format!("https://api.github.com/repos/{GITHUB_SLUG}/releases/latest");
    let agent = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .build();
    // GitHub 403's requests without a User-Agent. Accept header pins us to
    // the stable v3 schema so a future breaking change can't swap the field
    // names out from under us.
    let body: String = agent
        .get(&url)
        .set("User-Agent", &format!("Offspring/{}", env!("CARGO_PKG_VERSION")))
        .set("Accept", "application/vnd.github+json")
        .call()?
        .into_string()?;
    Ok(serde_json::from_str(&body)?)
}

/// Semver-lite "is `a` newer than `b`". Both are expected to look like
/// "N.N.N"; any component that fails to parse is treated as 0, which means a
/// malformed tag never ghosts-shows an update prompt. We deliberately ignore
/// pre-release suffixes — the GitHub API already filters those via the
/// `prerelease` flag.
fn is_newer(a: &str, b: &str) -> bool {
    parts(a) > parts(b)
}

fn parts(v: &str) -> (u32, u32, u32) {
    let mut it = v.split(|c: char| !c.is_ascii_digit() && c != '.').next().unwrap_or("").split('.');
    let major = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_ordering() {
        assert!(is_newer("0.3.0", "0.2.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(is_newer("0.2.1", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        // Trailing garbage / pre-release suffixes shouldn't matter.
        assert!(is_newer("0.3.0-rc1", "0.2.0"));
    }
}
