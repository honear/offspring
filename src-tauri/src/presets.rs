use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Gif,
    Mp4,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Crop {
    #[serde(rename = "16:9")]
    H16x9,
    #[serde(rename = "9:16")]
    V9x16,
    #[serde(rename = "1:1")]
    S1x1,
    #[serde(rename = "4:3")]
    H4x3,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Dither {
    Bayer,
    FloydSteinberg,
    Sierra2,
    Sierra24a,
    None,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub format: Format,
    pub suffix: String,

    // video sizing / cropping
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub fps: Option<u32>,
    #[serde(default)]
    pub crop: Option<Crop>,

    // gif-specific
    #[serde(default)]
    pub palette_colors: Option<u32>,
    #[serde(default)]
    pub dither: Option<Dither>,
    #[serde(default)]
    pub bayer_scale: Option<u32>,

    // mp4-specific
    #[serde(default)]
    pub crf: Option<u32>,
    #[serde(default)]
    pub preset_speed: Option<String>, // ultrafast..veryslow
    #[serde(default)]
    pub video_bitrate: Option<String>, // e.g. "2M"
    #[serde(default)]
    pub audio_bitrate: Option<String>, // e.g. "96k"
    #[serde(default)]
    pub use_cuda: Option<bool>,

    // target maximum output size in MB (auto-adjusts quality/width)
    #[serde(default)]
    pub target_max_mb: Option<u32>,

    // icon (absolute path or empty to use default)
    #[serde(default)]
    pub icon: Option<String>,

    // order in SendTo / UI
    #[serde(default)]
    pub order: u32,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Settings {
    #[serde(default)]
    pub ffmpeg_path: Option<String>,
    #[serde(default)]
    pub verbosity: Option<String>, // warning | info
    #[serde(default)]
    pub pause_after: Option<bool>,
    #[serde(default)]
    pub descriptive_names: Option<bool>,
}

pub fn load_presets() -> Result<Vec<Preset>> {
    let path = paths::presets_path()?;
    if !path.exists() {
        return Ok(crate::defaults::default_presets());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let presets: Vec<Preset> = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(presets)
}

pub fn save_presets(presets: &[Preset]) -> Result<()> {
    let path = paths::presets_path()?;
    let json = serde_json::to_string_pretty(presets)?;
    std::fs::write(&path, json)?;
    Ok(())
}

pub fn load_settings() -> Result<Settings> {
    let path = paths::settings_path()?;
    if !path.exists() {
        return Ok(Settings::default());
    }
    let raw = std::fs::read_to_string(&path)?;
    let s: Settings = serde_json::from_str(&raw)?;
    Ok(s)
}

pub fn save_settings(s: &Settings) -> Result<()> {
    let path = paths::settings_path()?;
    let json = serde_json::to_string_pretty(s)?;
    std::fs::write(&path, json)?;
    Ok(())
}

pub fn load_custom_last() -> Result<Preset> {
    let path = paths::custom_last_path()?;
    if !path.exists() {
        return Ok(crate::defaults::default_custom());
    }
    let raw = std::fs::read_to_string(&path)?;
    let p: Preset = serde_json::from_str(&raw)?;
    Ok(p)
}

pub fn save_custom_last(p: &Preset) -> Result<()> {
    let path = paths::custom_last_path()?;
    let json = serde_json::to_string_pretty(p)?;
    std::fs::write(&path, json)?;
    Ok(())
}
