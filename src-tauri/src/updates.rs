//! Lightweight GitHub-Releases-based update check + in-app installer
//! download.
//!
//! `check_for_updates` hits `https://api.github.com/repos/<slug>/releases/latest`,
//! parses the `tag_name` / asset list, and tells the UI whether a newer
//! version is published. `download_update` streams the Inno Setup `.exe`
//! asset into `%LOCALAPPDATA%\Offspring\updates\` with progress events on
//! `update-download`. `install_update` launches that downloaded installer
//! with `/VERYSILENT /CLOSEAPPLICATIONS /RESTARTAPPLICATIONS` and exits
//! the current process so the installer can overwrite the exe.
//!
//! Any HTTP/parse failure in the check degrades silently to
//! `update_available: false` so a network blip or an empty releases page
//! never shows up as an error badge.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::paths;

/// Owner/repo on GitHub — update this if the repo moves.
const GITHUB_SLUG: &str = "honear/offspring";

/// HTTP timeout for the release metadata check. The asset download uses a
/// longer read timeout since it streams ~10-30 MB.
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
    /// Release landing page — used as a fallback if the direct installer
    /// URL is missing.
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

#[derive(Serialize, Clone, Debug)]
pub struct UpdateDownloadEvent {
    /// "downloading" | "done" | "error"
    pub phase: String,
    /// 0.0..=100.0 when a Content-Length is available; None otherwise.
    pub percent: Option<f32>,
    /// On "done": absolute path to the downloaded installer. On "error":
    /// the error message. On "downloading": human-readable byte count.
    pub message: Option<String>,
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

fn fetch_latest() -> Result<GhRelease> {
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

/// Start a background download of the installer for `version` from
/// `installer_url`. Returns immediately; observe progress on the
/// `update-download` event. The downloaded file lives at
/// `%LOCALAPPDATA%\Offspring\updates\Offspring-Setup-<version>.exe` so a
/// subsequent `install_update` call can find it without re-passing the path.
#[tauri::command]
pub fn download_update(app: AppHandle, version: String, installer_url: String) -> Result<(), String> {
    if installer_url.is_empty() {
        return Err("no installer asset available on this release".into());
    }
    std::thread::spawn(move || {
        let result = stream_installer(&app, &version, &installer_url);
        match result {
            Ok(path) => emit(
                &app,
                UpdateDownloadEvent {
                    phase: "done".into(),
                    percent: Some(100.0),
                    message: Some(path.display().to_string()),
                },
            ),
            Err(e) => emit(
                &app,
                UpdateDownloadEvent {
                    phase: "error".into(),
                    percent: None,
                    message: Some(e.to_string()),
                },
            ),
        }
    });
    Ok(())
}

/// Launch the previously-downloaded installer for `version` and exit the
/// app so Inno Setup can overwrite `offspring.exe`. A detached PowerShell
/// watcher waits for the installer process to exit, then relaunches the
/// freshly-installed exe. If no matching downloaded file exists, returns
/// an error and does NOT exit.
///
/// We deliberately don't pass Inno's `/RESTARTAPPLICATIONS` — it only
/// works for applications registered with Windows Restart Manager
/// (Tauri apps aren't, by default) and silently no-ops otherwise.
#[tauri::command]
pub fn install_update(version: String) -> Result<(), String> {
    let path = installer_path(&version).map_err(|e| e.to_string())?;
    if !path.exists() {
        return Err(format!(
            "installer not found at {} — download it first",
            path.display()
        ));
    }

    // /VERYSILENT — no UI. /NORESTART — never reboot the machine.
    // /SUPPRESSMSGBOXES — pair with /SILENT to swallow any "another
    // instance is running" prompts if the restart-app handshake misfires.
    std::process::Command::new(&path)
        .args(["/VERYSILENT", "/NORESTART", "/SUPPRESSMSGBOXES"])
        .spawn()
        .map_err(|e| format!("spawning installer: {e}"))?;

    // Post-swap relaunch. We can't wait on the installer PID because
    // Inno Setup forks almost immediately (extracts Setup.tmp, re-launches
    // elevated, exits the original). Instead, the watcher polls our own
    // exe file for exclusive read access — that test fails while *any*
    // process (us, the installer, the UAC bootstrap) has it open, and
    // succeeds the moment all file handles are released. Then it waits
    // one more second for Windows to settle the new image and launches
    // offspring.exe.
    //
    // Spawned with DETACHED_PROCESS so it survives our own exit with no
    // console and no parent-process dependency. Logs to
    // `%LOCALAPPDATA%\Offspring\update-relaunch.log` so a silent failure
    // can be diagnosed after the fact.
    let exe = std::env::current_exe()
        .map_err(|e| format!("locating current exe: {e}"))?;
    let log = paths::local_data_dir()
        .map_err(|e| format!("locating local data dir: {e}"))?
        .join("update-relaunch.log");
    let exe_str = exe.display().to_string().replace('\'', "''");
    let log_str = log.display().to_string().replace('\'', "''");
    let ps_cmd = format!(
        "$exe = '{exe_str}'; \
         $log = '{log_str}'; \
         \"[$(Get-Date -Format o)] watcher started, polling $exe\" | Out-File -FilePath $log -Append -Encoding utf8; \
         $ok = $false; \
         for ($i = 0; $i -lt 60; $i++) {{ \
             Start-Sleep -Seconds 1; \
             try {{ \
                 $s = [System.IO.File]::Open($exe, 'Open', 'Read', 'None'); \
                 $s.Close(); \
                 $ok = $true; \
                 break \
             }} catch {{}} \
         }} \
         \"[$(Get-Date -Format o)] poll result: $ok after $i iterations\" | Out-File -FilePath $log -Append -Encoding utf8; \
         Start-Sleep -Seconds 1; \
         try {{ \
             Start-Process -FilePath $exe -ErrorAction Stop; \
             \"[$(Get-Date -Format o)] relaunch OK\" | Out-File -FilePath $log -Append -Encoding utf8 \
         }} catch {{ \
             \"[$(Get-Date -Format o)] relaunch FAIL: $($_.Exception.Message)\" | Out-File -FilePath $log -Append -Encoding utf8 \
         }}"
    );
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps_cmd])
        .creation_flags(DETACHED_PROCESS)
        .spawn()
        .map_err(|e| format!("spawning relaunch watcher: {e}"))?;

    // Give the watcher and installer a beat to initialize before we
    // release our own exe file handle by exiting.
    std::thread::sleep(Duration::from_millis(500));
    std::process::exit(0);
}

fn emit(app: &AppHandle, ev: UpdateDownloadEvent) {
    let _ = app.emit("update-download", ev);
}

fn installer_path(version: &str) -> Result<PathBuf> {
    let dir = paths::local_data_dir()?.join("updates");
    std::fs::create_dir_all(&dir).context("creating updates dir")?;
    Ok(dir.join(format!("Offspring-Setup-{version}.exe")))
}

fn stream_installer(app: &AppHandle, version: &str, url: &str) -> Result<PathBuf> {
    let final_path = installer_path(version)?;

    // Short-circuit: if we've already fully downloaded this version in a
    // previous session, skip the network round-trip. Size check is a
    // light sanity guard against a truncated partial from a prior crash —
    // <1 MB almost certainly means a broken download, since every
    // Offspring installer to date is several MB.
    if final_path.exists() {
        if let Ok(meta) = std::fs::metadata(&final_path) {
            if meta.len() > 1_000_000 {
                return Ok(final_path);
            }
        }
    }

    // Download into a sibling .part file, then atomically rename. That way
    // a crashed / cancelled download never leaves a truncated .exe that
    // `install_update` would try to run.
    let tmp_path = final_path.with_extension("exe.part");

    emit(
        app,
        UpdateDownloadEvent {
            phase: "downloading".into(),
            percent: Some(0.0),
            message: Some("Connecting to github.com…".into()),
        },
    );

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(180))
        .build();
    let resp = agent
        .get(url)
        .set("User-Agent", &format!("Offspring/{}", env!("CARGO_PKG_VERSION")))
        .call()
        .context("downloading installer")?;

    let total_len: Option<u64> = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok());

    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&tmp_path)
        .with_context(|| format!("creating {}", tmp_path.display()))?;
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    let mut last_emit = std::time::Instant::now();

    loop {
        let n = reader.read(&mut buf).context("reading installer stream")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).context("writing installer")?;
        downloaded += n as u64;

        if last_emit.elapsed() >= Duration::from_millis(150) {
            last_emit = std::time::Instant::now();
            let (pct, msg) = match total_len {
                Some(total) if total > 0 => {
                    let p = (downloaded as f64 / total as f64 * 100.0).min(100.0) as f32;
                    (
                        Some(p),
                        Some(format!(
                            "{:.1} of {:.1} MB",
                            downloaded as f64 / 1_048_576.0,
                            total as f64 / 1_048_576.0
                        )),
                    )
                }
                _ => (
                    None,
                    Some(format!("{:.1} MB", downloaded as f64 / 1_048_576.0)),
                ),
            };
            emit(
                app,
                UpdateDownloadEvent {
                    phase: "downloading".into(),
                    percent: pct,
                    message: msg,
                },
            );
        }
    }
    drop(file);

    if downloaded < 1_000_000 {
        let _ = std::fs::remove_file(&tmp_path);
        bail!("downloaded installer is suspiciously small ({downloaded} bytes) — server returned a truncated response");
    }

    // Atomically move into place. If a stale file exists from an earlier
    // partial attempt, nuke it first — Windows's rename won't overwrite.
    if final_path.exists() {
        let _ = std::fs::remove_file(&final_path);
    }
    std::fs::rename(&tmp_path, &final_path)
        .or_else(|_| std::fs::copy(&tmp_path, &final_path).map(|_| ()))
        .map_err(|e| anyhow!("finalizing installer path: {e}"))?;
    let _ = std::fs::remove_file(&tmp_path);

    Ok(final_path)
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
