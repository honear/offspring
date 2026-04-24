//! Read `%APPDATA%\Offspring\presets.json` from the shell extension at
//! menu-build time. The DLL runs inside Explorer.exe so we have no
//! persistent state; every flyout expansion re-reads from disk. That's
//! fine — the file is tiny and the user rarely has more than a dozen
//! presets.

use serde::Deserialize;
use std::path::PathBuf;

use windows::core::PCWSTR;
use windows::Win32::System::Registry::*;

#[derive(Deserialize, Clone, Debug)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    #[serde(default)]
    pub icon: Option<String>,
}

/// Settings subset the shell-ext cares about. We deliberately mirror
/// only the fields we need instead of pulling the whole struct from the
/// main app — the DLL runs inside Explorer, so minimizing its parse
/// surface keeps parse failures on unrelated additions from breaking
/// menu rendering.
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Settings {
    #[serde(default)]
    pub tools: ToolsSettings,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct ToolsSettings {
    #[serde(default)]
    pub merge: MergeTool,
    #[serde(default)]
    pub grayscale: GrayscaleTool,
    #[serde(default)]
    pub compare: CompareTool,
    #[serde(default)]
    pub overlay: OverlayTool,
}

#[derive(Deserialize, Clone, Debug)]
pub struct MergeTool {
    pub enabled: bool,
}

impl Default for MergeTool {
    fn default() -> Self {
        // Mirrors the main app's default — on by default now that Merge
        // is a single leaf entry (no sub-flyout, no preset picker). The
        // shell-ext reads this when settings.json is absent; otherwise
        // the user's saved value wins.
        Self { enabled: true }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct GrayscaleTool {
    pub enabled: bool,
}

impl Default for GrayscaleTool {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct CompareTool {
    pub enabled: bool,
}

impl Default for CompareTool {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct OverlayTool {
    pub enabled: bool,
}

impl Default for OverlayTool {
    fn default() -> Self {
        Self { enabled: false }
    }
}

fn presets_path() -> Option<PathBuf> {
    let mut p = dirs::data_dir()?;
    p.push("Offspring");
    p.push("presets.json");
    Some(p)
}

fn settings_path() -> Option<PathBuf> {
    let mut p = dirs::data_dir()?;
    p.push("Offspring");
    p.push("settings.json");
    Some(p)
}

/// Read `HKCU\Software\Offspring\ExePath` — written by the main app on
/// install and on every preset save. Returns None if the app hasn't
/// run yet (which should be impossible in practice — the MSIX toggle
/// only flips after the app is up).
pub fn read_exe_path() -> Option<String> {
    unsafe {
        let mut hkey = HKEY::default();
        let subkey: Vec<u16> = "Software\\Offspring\0".encode_utf16().collect();
        let status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_READ,
            &mut hkey,
        );
        if status.is_err() {
            return None;
        }
        let value_name: Vec<u16> = "ExePath\0".encode_utf16().collect();
        let mut buf = vec![0u16; 1024];
        let mut byte_len: u32 = (buf.len() * 2) as u32;
        let mut kind = REG_VALUE_TYPE::default();
        let rc = RegQueryValueExW(
            hkey,
            PCWSTR(value_name.as_ptr()),
            None,
            Some(&mut kind),
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut byte_len),
        );
        let _ = RegCloseKey(hkey);
        if rc.is_err() || byte_len < 2 {
            return None;
        }
        let chars = (byte_len as usize / 2).saturating_sub(1); // drop trailing NUL
        Some(String::from_utf16_lossy(&buf[..chars]))
    }
}

pub fn load_presets() -> Vec<Preset> {
    let Some(path) = presets_path() else { return Vec::new() };
    let Ok(raw) = std::fs::read_to_string(&path) else { return Vec::new() };
    serde_json::from_str::<Vec<Preset>>(&raw).unwrap_or_default()
}

pub fn load_settings() -> Settings {
    let Some(path) = settings_path() else { return Settings::default() };
    let Ok(raw) = std::fs::read_to_string(&path) else { return Settings::default() };
    serde_json::from_str::<Settings>(&raw).unwrap_or_default()
}
