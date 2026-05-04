use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Gif,
    Mp4,
    /// Single still-image output. The actual encoder + extension is
    /// driven by `Preset.image_codec` (PNG / JPEG / WebP / AVIF).
    /// Width/height/crop/greyscale all apply; fps + audio fields are
    /// ignored.
    Image,
}

/// Image codec selector for `Format::Image`. Each codec has its own
/// quality knob with its own native scale — the UI shows different
/// labels and ranges per codec, and `Preset.image_quality` stores the
/// raw native value so changing codec doesn't silently re-interpret
/// the number under a different scale.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImageCodec {
    /// Lossless. `image_quality` 0-9 = libpng compression level
    /// (0 fastest/largest, 9 slowest/smallest).
    Png,
    /// Lossy. `image_quality` 1-100 maps to ffmpeg's `-q:v` 31-2
    /// internally (lower q:v = better quality).
    Jpeg,
    /// Lossy by default. `image_quality` 0-100 passes directly to
    /// libwebp's `-quality`.
    Webp,
    /// Lossy via AV1 still-image. `image_quality` 0-63 = `-crf`
    /// (lower = better quality / larger file).
    Avif,
}

impl ImageCodec {
    /// Filename extension for the encoded output.
    pub fn ext(&self) -> &'static str {
        match self {
            ImageCodec::Png => "png",
            ImageCodec::Jpeg => "jpg",
            ImageCodec::Webp => "webp",
            ImageCodec::Avif => "avif",
        }
    }

    /// A reasonable starting quality value for fresh user-created
    /// presets when the field is left blank.
    pub fn default_quality(&self) -> u32 {
        match self {
            ImageCodec::Png => 6,
            ImageCodec::Jpeg => 85,
            ImageCodec::Webp => 80,
            ImageCodec::Avif => 24,
        }
    }
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

    // ---- Image-format fields (only meaningful when format == Image) ----
    /// Which image encoder to use. Determines output extension and
    /// the meaning of `image_quality`. `None` is treated as PNG by the
    /// encoder, which keeps deserialisation tolerant of older
    /// presets.json files written before this field existed.
    #[serde(default)]
    pub image_codec: Option<ImageCodec>,
    /// Quality / compression level for the chosen image codec, in the
    /// codec's NATIVE scale (see `ImageCodec` doc for ranges). Storing
    /// in native form avoids silent re-interpretation if the user
    /// changes codec — the value either still makes sense in the new
    /// codec's range or gets clamped at encode time.
    #[serde(default)]
    pub image_quality: Option<u32>,
    /// Strip EXIF / GPS / camera-serial metadata from the output.
    /// On by default for shipped image presets — most "send this
    /// image" workflows want privacy-preserving output. Implemented
    /// via ffmpeg's `-map_metadata -1`.
    #[serde(default)]
    pub strip_metadata: Option<bool>,

    /// Desaturate to greyscale. Independent of format — works on both
    /// GIF and MP4 outputs. When true, adds `format=gray` to the filter
    /// chain (MP4 keeps `yuv420p` as pix_fmt, GIF palette is generated
    /// from the already-greyscale frames). Also reachable as a Tool
    /// (right-click → Greyscale) via `derive_grayscale_preset`.
    #[serde(default)]
    pub grayscale: Option<bool>,

    /// Burn in the current frame number in the top-left corner.
    /// Uses Windows' bundled Consolas font. Independent of format —
    /// runs on both GIF (pre-palettegen) and MP4 outputs. Will also
    /// be reachable via the planned Overlay tool's timecode option.
    #[serde(default)]
    pub timecode: Option<bool>,

    /// Aspect-ratio overlay boxes. Populated only by the Guides tool
    /// via `derive_guides_preset` — no user preset ever writes this,
    /// so it's `skip`-d on serialize to keep presets.json clean.
    #[serde(skip)]
    pub guides: Option<GuidesConfig>,

    /// Rich-overlay config (per-corner text, border, opacity, color,
    /// guides). Populated only by the Overlay tool via
    /// `derive_overlay_preset`. Skip on serialize — this is tool-only
    /// state that never belongs in presets.json.
    #[serde(skip)]
    pub overlay: Option<OverlayConfig>,

    // icon (absolute path or empty to use default)
    #[serde(default)]
    pub icon: Option<String>,

    // order in SendTo / UI
    #[serde(default)]
    pub order: u32,
}

/// In-memory config for the guides block inside Overlay — which
/// aspect-ratio boxes to draw and what opacity to use. Never serialized
/// into Preset (see `#[serde(skip)]` on `Preset.guides`); the on-disk
/// shape lives on the Overlay tool settings instead, which also fills
/// this struct at encode time.
#[derive(Clone, Debug)]
pub struct GuidesConfig {
    pub show_16_9: bool,
    pub show_9_16: bool,
    pub show_4_5: bool,
    /// ffmpeg-parseable color strings (e.g. "red", "0xff0000").
    /// `@alpha` is appended by the filter code from [`Self::opacity`].
    pub color_16_9: String,
    pub color_9_16: String,
    pub color_4_5: String,
    /// 0.0–1.0 alpha for both the drawbox outline and the ratio label.
    /// User-facing slider is 0–100; commands.rs divides by 100 before
    /// populating this.
    pub opacity: f32,
}

/// Per-corner overlay content. `None` means the corner stays blank.
/// `Custom` and `Custom2` are independent free-text slots so a user can
/// place, e.g., a shot code in one corner and a version tag in another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OverlaySlotKind {
    None,
    Filename,
    Timecode,
    Custom,
    Custom2,
}

/// Runtime config for the Overlay tool. Built from `OverlayTool` settings
/// plus the current input file (filename resolves per-file). Never
/// serialized; reachable only through `derive_overlay_preset`.
#[derive(Clone, Debug)]
pub struct OverlayConfig {
    pub top_left: OverlaySlotKind,
    pub top_right: OverlaySlotKind,
    pub bottom_left: OverlaySlotKind,
    pub bottom_right: OverlaySlotKind,
    /// User-entered string used when any slot resolves to `Custom`.
    pub custom_text: String,
    /// Second independent custom-text slot for `Custom2`.
    pub custom_text_2: String,
    /// Filename (no extension) of the current input. Filled by the
    /// encode-time helper before the filter chain is built.
    pub filename: String,
    /// 0.0–1.0. Applied to drawtext `fontcolor@opacity`.
    pub opacity: f32,
    /// ffmpeg-compatible color string (e.g. `white`, `0xffffff`).
    pub color: String,
    /// When true, pad the frame with black bars top + bottom and draw
    /// corner text inside those bars instead of on top of the image.
    pub border: bool,
    /// When true, emit the corner drawtext + optional border. Gates the
    /// "metadata" half of the Overlay pane in the UI. When false, the
    /// overlay encode only draws the aspect-ratio guide boxes (if those
    /// are enabled inside [`Self::guides`]).
    pub metadata: bool,
    /// Font scale multiplier for corner text. 1.0 = the legacy default
    /// (fontsize=h/25). Margins and box border scale in lockstep so the
    /// overall layout stays visually balanced at any size.
    pub font_scale: f32,
    /// When true, include the Guides aspect-ratio boxes in the output.
    pub guides: GuidesConfig,
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
    /// Whether to mirror the preset list into the user's SendTo folder.
    /// Off by default — the right-click "Offspring ►" submenu from the
    /// registry-based integration replaces the SendTo surface. Flip this
    /// on to get entries under "Send to" on top of that.
    #[serde(default)]
    pub sendto_enabled: Option<bool>,
    /// Whether the Windows 11 modern (top-level) right-click menu should
    /// carry Offspring. Requires registering an MSIX sparse package with
    /// a self-signed cert; handled by `integration::modern_menu`. Off by
    /// default because enabling it prompts the user for cert trust.
    #[serde(default)]
    pub modern_menu_enabled: Option<bool>,
    /// Extension tools (auto-detect sequences, merge multi-select, …).
    /// See `ToolsSettings` for the per-tool knobs. Absent / partial JSON
    /// falls back to `ToolsSettings::default()` so old settings files
    /// keep parsing.
    #[serde(default)]
    pub tools: ToolsSettings,
}

/// Aggregate of every per-tool config block. Keeping tools in their own
/// sub-object (rather than flat fields on `Settings`) means we can add or
/// remove tools without churning top-level field names, and the UI can
/// mirror the shape one-to-one.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolsSettings {
    #[serde(default = "SequenceTool::default")]
    pub sequence: SequenceTool,
    #[serde(default = "MergeTool::default")]
    pub merge: MergeTool,
    #[serde(default = "GrayscaleTool::default")]
    pub grayscale: GrayscaleTool,
    #[serde(default = "CompareTool::default")]
    pub compare: CompareTool,
    #[serde(default = "OverlayTool::default")]
    pub overlay: OverlayTool,
    #[serde(default = "TrimTool::default")]
    pub trim: TrimTool,
    #[serde(default = "InvertTool::default")]
    pub invert: InvertTool,
    #[serde(default = "MakeSquareTool::default")]
    pub make_square: MakeSquareTool,
}

impl Default for ToolsSettings {
    fn default() -> Self {
        Self {
            sequence: SequenceTool::default(),
            merge: MergeTool::default(),
            grayscale: GrayscaleTool::default(),
            compare: CompareTool::default(),
            overlay: OverlayTool::default(),
            trim: TrimTool::default(),
            invert: InvertTool::default(),
            make_square: MakeSquareTool::default(),
        }
    }
}

/// Auto-detect image sequences on single-file right-click and encode the
/// whole sequence with the preset's FPS instead of a one-frame clip.
/// Enabled by default — the detection is conservative (images only, stem
/// must end in N+ zero-padded digits, and at least one sibling must match).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SequenceTool {
    pub enabled: bool,
    /// Minimum trailing zero-padded digit count that counts as a sequence.
    /// Four is the VFX/render-farm convention (`render_0001.png`) and
    /// filters out accidental matches like version tags (`r01`, `v02`).
    pub min_digits: u32,
    /// Fallback framerate used when the preset doesn't specify one.
    /// Float because VFX/broadcast rates (23.976, 29.97) are common; the
    /// image2 demuxer accepts them directly via `-framerate`. Presets
    /// that DO set `fps` win over this — it only kicks in for MP4
    /// presets that leave fps unset.
    #[serde(default = "default_sequence_fps")]
    pub default_fps: f32,
}

fn default_sequence_fps() -> f32 {
    24.0
}

impl Default for SequenceTool {
    fn default() -> Self {
        Self {
            enabled: true,
            min_digits: 4,
            default_fps: default_sequence_fps(),
        }
    }
}

/// Merge multiple selected videos into a single output via ffmpeg's
/// concat demuxer. Exposed as a single "Merge" entry inside the Offspring
/// modern-menu flyout (hidden on single-file selection). Output format +
/// settings are inherited from the first selected file — no preset
/// picker. On by default because a single toggle-able verb has a much
/// lower "where'd that come from?" cost than the per-preset sub-flyout
/// it replaces.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MergeTool {
    pub enabled: bool,
}

impl Default for MergeTool {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// One-shot greyscale conversion that inherits format + dimensions + fps
/// from the input. Appears as a single leaf entry in the modern menu and
/// as "Offspring Greyscale.lnk" in SendTo when the toggle is on. Users
/// who want a specific quality knob combined with greyscale should
/// instead set the per-preset `grayscale` field on a saved preset.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GrayscaleTool {
    pub enabled: bool,
}

impl Default for GrayscaleTool {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn default_guides_16_9() -> bool { true }
fn default_guides_9_16() -> bool { true }
fn default_guides_4_5() -> bool { false }

fn default_color_16_9() -> String { "0xe5484d".into() }
fn default_color_9_16() -> String { "0x00c2d7".into() }
fn default_color_4_5() -> String { "0xf5d90a".into() }

/// Frame-accurate trim: strip N frames from the start and/or end of each
/// input. Exposed as a "Trim..." entry that opens a mini dialog asking
/// for two frame counts; per-file independent (each input gets the same
/// pair of values applied to ITS own timeline). Output keeps the source
/// format and inherits the same MP4/GIF baseline used by Greyscale and
/// Merge (CRF 23 / medium for MP4, 128-color bayer for GIF) — frame
/// boundaries forbid stream-copy, so we re-encode at a known-good
/// quality. Suffix `_trimmed`. On by default.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TrimTool {
    pub enabled: bool,
}

impl Default for TrimTool {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Color/alpha invert tool. Image-only — refuses video inputs with a
/// clear error. Uses ffmpeg's `negate` filter for the RGB invert; the
/// alpha channel is preserved untouched so a transparent PNG with
/// black opaque content comes out as the same shape rendered white.
///
/// `clamp` makes the output a strict 1-bit-per-channel result —
/// every channel (including alpha if present) gets thresholded to
/// either 0 or 255. Useful for cleaning up masks where the source
/// has anti-aliased edges or compression artifacts.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InvertTool {
    pub enabled: bool,
    /// Threshold every channel to {0, 255} after inverting. Off by
    /// default — most users invert photos / soft masks where they
    /// want to preserve gradients. On is what you want for binary
    /// masks (alpha-channel masks, B/W layer masks).
    #[serde(default)]
    pub clamp: bool,
}

impl Default for InvertTool {
    fn default() -> Self {
        Self {
            enabled: true,
            clamp: false,
        }
    }
}

/// Make-Square tool. Pads the shorter edge of an image to match the
/// longer one, producing a square output. Image-only — refuses video
/// inputs.
///
/// `fill_mode` chooses what fills the new pixels:
///   * `Transparent` — `pad` with `black@0`. Forces the output codec
///     to one that supports alpha (PNG / WebP / AVIF). When the input
///     is JPEG (no alpha), we emit PNG instead.
///   * `EdgeColor` — sample the top-left pixel of the input via a
///     short ffmpeg probe and use that as the pad color. Output keeps
///     the input codec.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MakeSquareTool {
    pub enabled: bool,
    #[serde(default)]
    pub fill_mode: MakeSquareFillMode,
}

impl Default for MakeSquareTool {
    fn default() -> Self {
        Self {
            enabled: true,
            fill_mode: MakeSquareFillMode::Transparent,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MakeSquareFillMode {
    Transparent,
    EdgeColor,
}

impl Default for MakeSquareFillMode {
    fn default() -> Self {
        Self::Transparent
    }
}

/// Last-used Trim dialog values, persisted to `trim_last.json` so the
/// dialog reopens with the user's previous numbers instead of zeros.
/// Mirrors the `custom_last.json` pattern.
///
/// `remove_from` / `remove_to` are an optional middle-range cut: when
/// both are `Some` and `to >= from`, the encoder excises that frame
/// range (inclusive both ends) from each input in addition to whatever
/// `start_frames`/`end_frames` strip from the ends. They're `Option`
/// rather than 0-as-disabled because frame 0 is a legitimate range
/// boundary — we need a way to say "no middle cut" that isn't "the
/// user picked frame 0 to frame 0".
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TrimLast {
    #[serde(default)]
    pub start_frames: u32,
    #[serde(default)]
    pub end_frames: u32,
    #[serde(default)]
    pub remove_from: Option<u32>,
    #[serde(default)]
    pub remove_to: Option<u32>,
}

/// Side-by-side A/B compare: stack N selected files horizontally into
/// one output. Heights are normalized to the first file so hstack
/// accepts them; framerate is normalized to the first file's too so
/// the streams stay in sync. Output format matches the first file.
/// Enabled by default — single-click review workflow, no config.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CompareTool {
    pub enabled: bool,
}

impl Default for CompareTool {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Overlay tool: burns per-corner text (filename, timecode, or user
/// string), optional aspect-ratio guide boxes, and an optional solid
/// border onto each input. All four corners share the same color,
/// opacity, and custom-text fields so the UI stays compact.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OverlayTool {
    pub enabled: bool,
    #[serde(default = "OverlaySlot::none")]
    pub top_left: OverlaySlot,
    #[serde(default = "OverlaySlot::none")]
    pub top_right: OverlaySlot,
    #[serde(default = "OverlaySlot::none")]
    pub bottom_left: OverlaySlot,
    #[serde(default = "OverlaySlot::none")]
    pub bottom_right: OverlaySlot,
    /// Shared text used by any corner whose slot is `Custom`.
    #[serde(default)]
    pub custom_text: String,
    /// Second independent text slot, wired to the `Custom 2…` dropdown
    /// option so one overlay can carry two arbitrary labels at once.
    #[serde(default)]
    pub custom_text_2: String,
    /// 0–100, UI scale. Converted to 0.0–1.0 before being baked into
    /// the `fontcolor@opacity` string.
    #[serde(default = "default_overlay_opacity")]
    pub opacity: u32,
    /// Overlay text color. Stored in ffmpeg-parseable form — the UI
    /// sends hex with the `#` stripped and an `0x` prefix applied.
    #[serde(default = "default_overlay_color")]
    pub color: String,
    /// Pad the clip on all four sides so corner text sits outside the
    /// image. The left/right strips are left blank by design — the user
    /// asked for equal borders for a clean frame.
    #[serde(default)]
    pub border: bool,
    /// Gate for the "metadata" half of the Overlay pane (corner text,
    /// text color/opacity, border). When off, the overlay encode only
    /// draws the aspect-ratio guides (if those are enabled). Defaults to
    /// true so existing installs keep showing corner text after upgrade.
    #[serde(default = "default_overlay_metadata")]
    pub metadata: bool,
    /// When true, also draw the Guides aspect-ratio boxes (see below).
    #[serde(default)]
    pub guides: bool,
    #[serde(default = "default_guides_16_9")]
    pub show_16_9: bool,
    #[serde(default = "default_guides_9_16")]
    pub show_9_16: bool,
    #[serde(default = "default_guides_4_5")]
    pub show_4_5: bool,
    #[serde(default = "default_color_16_9")]
    pub color_16_9: String,
    #[serde(default = "default_color_9_16")]
    pub color_9_16: String,
    #[serde(default = "default_color_4_5")]
    pub color_4_5: String,
    /// 0–100, UI scale. Applied only to the guide boxes (independent of
    /// the metadata opacity field, which controls corner text). Defaults
    /// to 90 — matches the old hard-coded `@0.9` behavior.
    #[serde(default = "default_overlay_opacity")]
    pub guides_opacity: u32,
    /// Font size as a percentage (50–200). 100 = legacy default.
    /// Margins + box border scale with it so proportions stay balanced.
    #[serde(default = "default_overlay_font_scale")]
    pub metadata_font_scale: u32,
}

/// Settings-level shape of a single overlay slot. Parallel to
/// `OverlaySlotKind` but serde-friendly; the runtime `OverlayConfig`
/// dereferences this into a `OverlaySlotKind` before filter-building.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum OverlaySlot {
    None,
    Filename,
    Timecode,
    Custom,
    Custom2,
}

impl OverlaySlot {
    fn none() -> Self {
        OverlaySlot::None
    }
    pub fn to_kind(&self) -> OverlaySlotKind {
        match self {
            OverlaySlot::None => OverlaySlotKind::None,
            OverlaySlot::Filename => OverlaySlotKind::Filename,
            OverlaySlot::Timecode => OverlaySlotKind::Timecode,
            OverlaySlot::Custom => OverlaySlotKind::Custom,
            OverlaySlot::Custom2 => OverlaySlotKind::Custom2,
        }
    }
}

fn default_overlay_opacity() -> u32 {
    90
}

fn default_overlay_color() -> String {
    "white".into()
}

fn default_overlay_metadata() -> bool {
    true
}

fn default_overlay_font_scale() -> u32 {
    100
}

impl Default for OverlayTool {
    fn default() -> Self {
        Self {
            enabled: false,
            top_left: OverlaySlot::Filename,
            top_right: OverlaySlot::None,
            bottom_left: OverlaySlot::None,
            bottom_right: OverlaySlot::Timecode,
            custom_text: String::new(),
            custom_text_2: String::new(),
            opacity: default_overlay_opacity(),
            color: default_overlay_color(),
            border: false,
            metadata: default_overlay_metadata(),
            guides: false,
            show_16_9: default_guides_16_9(),
            show_9_16: default_guides_9_16(),
            show_4_5: default_guides_4_5(),
            color_16_9: default_color_16_9(),
            color_9_16: default_color_9_16(),
            color_4_5: default_color_4_5(),
            guides_opacity: default_overlay_opacity(),
            metadata_font_scale: default_overlay_font_scale(),
        }
    }
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

pub fn load_trim_last() -> Result<TrimLast> {
    let path = paths::trim_last_path()?;
    if !path.exists() {
        return Ok(TrimLast::default());
    }
    let raw = std::fs::read_to_string(&path)?;
    let t: TrimLast = serde_json::from_str(&raw).unwrap_or_default();
    Ok(t)
}

pub fn save_trim_last(t: &TrimLast) -> Result<()> {
    let path = paths::trim_last_path()?;
    let json = serde_json::to_string_pretty(t)?;
    std::fs::write(&path, json)?;
    Ok(())
}
