//! FFmpeg bootstrap.
//!
//! Downloads the latest LGPL "essentials" FFmpeg build from gyan.dev and
//! extracts it under `%LOCALAPPDATA%\Offspring\ffmpeg\`. Runs on a
//! background thread, emitting `ffmpeg-download` progress events.
//!
//! This mirrors what `installer/scripts/download_ffmpeg.ps1` does at install
//! time — keeping it in-app means the default NSIS installer works too, and
//! users whose first install skipped the download can recover without having
//! to re-run the Inno Setup installer.

use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::paths;

const FFMPEG_URL: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";
/// gyan.dev publishes a SHA-256 sidecar next to every release ZIP. The
/// app downloads both, computes the hash of the ZIP, and refuses to
/// extract on mismatch. Doesn't fully neutralise an attacker who can
/// MITM gyan.dev's TLS (they could swap both files), but it does
/// defeat partial-compromise / cache-poisoning scenarios where one
/// URL gets tampered with and not the other, and it catches transport-
/// or storage-level corruption.
const FFMPEG_SHA256_URL: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip.sha256";

#[derive(Serialize, Clone, Debug)]
pub struct DownloadEvent {
    /// "downloading" | "extracting" | "done" | "error"
    pub phase: String,
    /// 0.0..=100.0 for downloading; None otherwise
    pub percent: Option<f32>,
    /// Optional human-readable status
    pub message: Option<String>,
}

fn emit(app: &AppHandle, ev: DownloadEvent) {
    let _ = app.emit("ffmpeg-download", ev);
}

/// Run the full download → extract → install pipeline on a background
/// thread. Returns immediately; observe progress via `ffmpeg-download`
/// events.
pub fn spawn_download(app: AppHandle) {
    std::thread::spawn(move || {
        let result = download_and_install(&app);
        match result {
            Ok(path) => emit(
                &app,
                DownloadEvent {
                    phase: "done".into(),
                    percent: Some(100.0),
                    message: Some(path.display().to_string()),
                },
            ),
            Err(e) => emit(
                &app,
                DownloadEvent {
                    phase: "error".into(),
                    percent: None,
                    message: Some(e.to_string()),
                },
            ),
        }
    });
}

fn download_and_install(app: &AppHandle) -> Result<PathBuf> {
    let target = paths::local_data_dir()?.join("ffmpeg");
    let bin_exe = target.join("bin").join("ffmpeg.exe");

    // Unique temp paths so parallel installs (unlikely, but possible) don't clash
    let uid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_zip = std::env::temp_dir().join(format!("ffmpeg-offspring-{uid}-{ts}.zip"));
    let tmp_extract = std::env::temp_dir().join(format!("ffmpeg-offspring-extract-{uid}-{ts}"));

    // --- 1. Download ------------------------------------------------------
    emit(
        app,
        DownloadEvent {
            phase: "downloading".into(),
            percent: Some(0.0),
            message: Some("Connecting to gyan.dev…".into()),
        },
    );

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(120))
        .build();
    let resp = agent
        .get(FFMPEG_URL)
        .call()
        .context("downloading FFmpeg")?;

    let total_len: Option<u64> = resp
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok());

    let mut reader = resp.into_reader();
    let mut file = std::fs::File::create(&tmp_zip)
        .with_context(|| format!("creating temp file {}", tmp_zip.display()))?;
    let mut buf = [0u8; 64 * 1024];
    let mut downloaded: u64 = 0;
    let mut last_emit = std::time::Instant::now();
    // Hash the bytes as they stream past so we don't have to re-read
    // the file from disk after the download finishes. SHA-256 is fast
    // enough (~500 MB/s on modern x86) that this never gates progress.
    let mut hasher = Sha256::new();

    loop {
        let n = reader.read(&mut buf).context("reading zip stream")?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).context("writing temp zip")?;
        hasher.update(&buf[..n]);
        downloaded += n as u64;

        // Throttle progress emits so we don't flood the event bus.
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
                DownloadEvent {
                    phase: "downloading".into(),
                    percent: pct,
                    message: msg,
                },
            );
        }
    }
    drop(file);
    let computed_hash = hex_lower(&hasher.finalize());

    // --- 1b. Verify the SHA-256 against gyan.dev's published sidecar ---
    // Use a fresh agent so the verify call doesn't inherit the long
    // read timeout the download agent needed. Failing here aborts the
    // bootstrap before we touch the extract path, so a tampered or
    // corrupted ZIP never reaches the user's filesystem as a real
    // ffmpeg.exe.
    emit(
        app,
        DownloadEvent {
            phase: "downloading".into(),
            percent: None,
            message: Some("Verifying checksum…".into()),
        },
    );
    let expected_hash = fetch_expected_sha256()
        .context("fetching FFmpeg SHA-256 sidecar from gyan.dev")?;
    if !constant_time_eq(computed_hash.as_bytes(), expected_hash.as_bytes()) {
        let _ = std::fs::remove_file(&tmp_zip);
        bail!(
            "FFmpeg ZIP integrity check failed: expected sha256 {expected_hash}, got {computed_hash}. \
             Refusing to install. Try again later, or set a manual ffmpeg.exe path in Settings."
        );
    }

    // --- 2. Extract -------------------------------------------------------
    emit(
        app,
        DownloadEvent {
            phase: "extracting".into(),
            percent: None,
            message: Some("Unpacking archive…".into()),
        },
    );

    std::fs::create_dir_all(&tmp_extract).context("creating extract dir")?;
    let zip_file = std::fs::File::open(&tmp_zip).context("reopening downloaded zip")?;
    let mut archive = zip::ZipArchive::new(zip_file).context("opening zip archive")?;
    // `extract` handles nested dirs for us.
    archive.extract(&tmp_extract).context("extracting zip")?;
    drop(archive);

    // gyan.dev ships a single top-level folder like "ffmpeg-N.N.N-essentials_build/"
    let nested = std::fs::read_dir(&tmp_extract)
        .context("listing extracted files")?
        .filter_map(|e| e.ok())
        .find(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .ok_or_else(|| anyhow!("unexpected archive layout (no top-level folder)"))?;

    // --- 3. Install -------------------------------------------------------
    std::fs::create_dir_all(&target).context("creating install dir")?;
    for sub in ["bin", "presets", "doc"] {
        let src = nested.path().join(sub);
        let dst = target.join(sub);
        if src.exists() {
            if dst.exists() {
                let _ = std::fs::remove_dir_all(&dst);
            }
            // rename across drives can fail; fall back to recursive copy
            if std::fs::rename(&src, &dst).is_err() {
                copy_dir_recursive(&src, &dst)?;
            }
        }
    }
    let license = nested.path().join("LICENSE");
    if license.exists() {
        let _ = std::fs::copy(&license, target.join("LICENSE"));
    }

    // Cleanup best-effort
    let _ = std::fs::remove_file(&tmp_zip);
    let _ = std::fs::remove_dir_all(&tmp_extract);

    if !bin_exe.exists() {
        bail!(
            "ffmpeg.exe missing after extraction: expected {}",
            bin_exe.display()
        );
    }
    Ok(bin_exe)
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dst_path)?;
        } else {
            std::fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

/// Lowercase hex of a digest. Standalone instead of pulling in the
/// `hex` crate just for this — the loop is straightforward and the
/// result is only used for a constant-time string comparison.
fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(*b >> 4) as usize] as char);
        s.push(HEX[(*b & 0x0f) as usize] as char);
    }
    s
}

/// Constant-time byte-slice equality. Avoids leaking timing information
/// during hash comparison — almost certainly overkill against a remote
/// attacker, but cheap enough that we may as well not skip it.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Pull gyan.dev's `.sha256` sidecar and extract the first 64-hex-char
/// run we find. Sidecar format varies — sometimes a bare hash, sometimes
/// `<hash> *<filename>` (BSD style), sometimes `<hash>  <filename>` (GNU
/// style). All of those have the same first token, so a regex-light
/// scan is enough. We tolerate trailing junk but reject the response if
/// no 64-hex run is present (HTML 5xx pages, redirects, blank).
fn fetch_expected_sha256() -> Result<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(20))
        .timeout_read(Duration::from_secs(30))
        .build();
    let body: String = agent
        .get(FFMPEG_SHA256_URL)
        .call()?
        .into_string()?;
    let trimmed = body.trim();
    // Find the first 64-hex run. Anchored to ASCII so multi-byte
    // surprises in a tampered response can't slide a "valid" hash past us.
    for word in trimmed.split_whitespace() {
        if word.len() == 64 && word.chars().all(|c| c.is_ascii_hexdigit()) {
            return Ok(word.to_ascii_lowercase());
        }
    }
    Err(anyhow!(
        "SHA-256 sidecar response did not contain a 64-hex hash: {:?}",
        trimmed.chars().take(120).collect::<String>()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_lower_basic() {
        assert_eq!(hex_lower(&[0x00, 0xff, 0xab]), "00ffab");
        assert_eq!(hex_lower(&[]), "");
    }

    #[test]
    fn constant_time_eq_basic() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(constant_time_eq(b"", b""));
    }
}
