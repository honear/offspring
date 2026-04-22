use anyhow::{Context, Result};
use std::path::PathBuf;

pub fn data_dir() -> Result<PathBuf> {
    let mut p = dirs::data_dir().context("no APPDATA directory")?;
    p.push("Offspring");
    std::fs::create_dir_all(&p).ok();
    Ok(p)
}

pub fn local_data_dir() -> Result<PathBuf> {
    let mut p = dirs::data_local_dir().context("no LOCALAPPDATA directory")?;
    p.push("Offspring");
    std::fs::create_dir_all(&p).ok();
    Ok(p)
}

pub fn presets_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("presets.json"))
}

pub fn custom_last_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("custom_last.json"))
}

pub fn settings_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("settings.json"))
}

pub fn ffmpeg_managed_path() -> Result<PathBuf> {
    Ok(local_data_dir()?.join("ffmpeg").join("bin").join("ffmpeg.exe"))
}

#[allow(dead_code)]
pub fn icons_dir() -> Result<PathBuf> {
    let p = data_dir()?.join("icons");
    std::fs::create_dir_all(&p).ok();
    Ok(p)
}

pub fn sendto_dir() -> Result<PathBuf> {
    let mut p = dirs::data_dir().context("no APPDATA directory")?;
    p.push("Microsoft");
    p.push("Windows");
    p.push("SendTo");
    Ok(p)
}

/// Where we record which SendTo shortcut filenames we've created. We used to
/// recognize our own shortcuts by a shared "Offspring - " filename prefix, but
/// the user asked for shorter names. Without a marker we can't tell our .lnks
/// apart from unrelated SendTo entries, so we track them ourselves.
pub fn sendto_manifest_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("sendto-manifest.json"))
}
