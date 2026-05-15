use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::paths;
use crate::presets::{
    Crop, Dither, Format, GuidesConfig, OverlayConfig, OverlaySlotKind, Preset, Settings,
};
use crate::sequence::SequenceInfo;

/// Shapes of ffmpeg input we support.
///   * `File` — classic one-file encode.
///   * `Sequence` — image sequence via the `image2` demuxer.
///   * `Concat` — N videos glued via the `concat` demuxer. The caller
///     is responsible for writing the listing file to disk at
///     `list_path` before passing this in, and cleaning it up after.
#[derive(Debug, Clone)]
pub enum EncodeInput {
    File(PathBuf),
    /// fps is the rate the sequence is fed INTO ffmpeg — the encoded
    /// output framerate is still governed by the preset's `fps` filter.
    /// Callers typically pass the same value for both so input and
    /// output timing line up 1:1. f32 because VFX rates like 23.976
    /// and 29.97 aren't representable as integers; the image2 demuxer
    /// accepts decimals directly after `-framerate`.
    Sequence { info: SequenceInfo, fps: f32 },
    Concat {
        /// Text file listing `file '<path>'` lines. Written by the
        /// caller; ffmpeg reads it via the concat demuxer.
        list_path: PathBuf,
        /// Where the final output should land.
        output_dir: PathBuf,
        /// Base name (no extension) for the output file.
        output_stem: String,
        /// Pre-computed sum of input durations for the progress bar.
        /// None if any ffprobe call failed — progress just won't show
        /// a percentage in that case.
        total_duration_s: Option<f64>,
    },
}

impl EncodeInput {
    /// Ffmpeg input arg list. For files that's just `-i <path>`. For
    /// sequences we prepend `-framerate` + `-start_number` because the
    /// image2 demuxer needs those before `-i` to interpret the pattern.
    fn input_args(&self) -> Vec<OsString> {
        match self {
            Self::File(p) => vec![OsString::from("-i"), p.as_os_str().to_owned()],
            Self::Sequence { info, fps } => vec![
                OsString::from("-framerate"),
                // f32's Display trims the trailing zero on whole numbers
                // (24.0 → "24") and keeps the fraction for decimals
                // (23.976 → "23.976"), which is exactly what ffmpeg
                // wants after `-framerate`.
                OsString::from(fps.to_string()),
                OsString::from("-start_number"),
                OsString::from(info.start_number.to_string()),
                OsString::from("-i"),
                info.ffmpeg_input_pattern().into_os_string(),
            ],
            Self::Concat { list_path, .. } => vec![
                OsString::from("-f"),
                OsString::from("concat"),
                OsString::from("-safe"),
                OsString::from("0"),
                OsString::from("-i"),
                list_path.as_os_str().to_owned(),
            ],
        }
    }

    /// Directory the output file should land in.
    fn output_dir(&self) -> PathBuf {
        match self {
            Self::File(p) => p.parent().unwrap_or(Path::new(".")).to_path_buf(),
            Self::Sequence { info, .. } => info.dir.clone(),
            Self::Concat { output_dir, .. } => output_dir.clone(),
        }
    }

    /// Base name (no extension, no suffix) for the output file.
    fn output_stem(&self) -> String {
        match self {
            Self::File(p) => p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("output")
                .to_string(),
            Self::Sequence { info, .. } => info.output_stem(),
            Self::Concat { output_stem, .. } => output_stem.clone(),
        }
    }

    /// Human-readable label used in progress events. For non-file
    /// variants we stringify the pattern / list path so the progress UI
    /// still shows something recognizable rather than a blank.
    pub fn display(&self) -> String {
        match self {
            Self::File(p) => p.display().to_string(),
            Self::Sequence { info, .. } => info.ffmpeg_input_pattern().display().to_string(),
            Self::Concat { output_stem, .. } => format!("merge: {output_stem}"),
        }
    }

    /// Best-effort clip duration. Files fall back to ffprobe. Sequences
    /// compute from frame_count / fps directly — ffprobe can be flaky on
    /// `%04d` patterns and we already have the numbers. Concat reuses
    /// the summed duration the caller already computed.
    pub fn duration_hint(&self, ffmpeg: &Path) -> Option<f64> {
        match self {
            Self::File(p) => probe_duration(ffmpeg, p),
            Self::Sequence { info, fps } => {
                if *fps <= 0.0 {
                    None
                } else {
                    Some(info.frame_count as f64 / *fps as f64)
                }
            }
            Self::Concat { total_duration_s, .. } => *total_duration_s,
        }
    }
}

/// Windows flag that prevents the child process from ever opening a console
/// window. Our parent process is a GUI (Tauri) binary, but FFmpeg/ffprobe
/// would still flash a console if we didn't set this.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Apply the console-suppression flag on Windows. No-op elsewhere so the
/// project keeps building on macOS/Linux for development.
fn hide_console(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Resolve which `ffmpeg.exe` to invoke for this encode session.
///
/// Order of precedence:
///   1. **Explicit user override** from `settings.ffmpeg_path`. If
///      this is set to anything non-empty, IT WINS — and if it's
///      invalid we surface a hard error instead of silently falling
///      through. Falling back would re-introduce the exact bug a
///      user is trying to fix by setting the override (e.g. ImageMagick's
///      bundled `ffmpeg.exe` getting picked up off PATH and shadowing
///      the build the user actually wants).
///   2. **Managed bundled FFmpeg** at
///      `%LOCALAPPDATA%\Offspring\ffmpeg\bin\ffmpeg.exe` — what the
///      first-run download writes into.
///   3. **PATH lookup** for `ffmpeg.exe` as a last resort. Works for
///      developers with a system FFmpeg, BUT can land on a stripped-
///      down build from another app (ImageMagick, OBS, etc.) that's
///      missing filters Offspring needs. That's why the explicit-
///      override path is offered in Settings — and why we don't fall
///      back to PATH when the user has chosen a specific path.
///
/// Validation for the explicit-override path:
///   * Must point at a regular file (not a directory).
///   * Filename must be `ffmpeg.exe` (case-insensitive). Custom builds
///     renamed to `ffmpeg-static.exe` etc. fall out — rare in practice,
///     and a clear error here is better than a confusing subprocess
///     failure later when the binary doesn't accept ffmpeg flags.
pub fn resolve_ffmpeg(settings: &Settings) -> Result<PathBuf> {
    if let Some(ref configured) = settings.ffmpeg_path {
        let trimmed = configured.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if !p.is_file() {
                bail!(
                    "The FFmpeg path you set in Settings doesn't point at a file: \
                     {} — clear the path to use the bundled FFmpeg, or pick the \
                     real ffmpeg.exe.",
                    p.display()
                );
            }
            let is_ffmpeg_exe = p
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.eq_ignore_ascii_case("ffmpeg.exe"))
                .unwrap_or(false);
            if !is_ffmpeg_exe {
                bail!(
                    "The FFmpeg path you set in Settings isn't named ffmpeg.exe: \
                     {} — pick the actual ffmpeg.exe file, or clear the path to \
                     use the bundled FFmpeg.",
                    p.display()
                );
            }
            return Ok(p);
        }
        // Empty / whitespace-only string is treated the same as None —
        // the user has indicated "use the default" by clearing the field.
    }
    let managed = paths::ffmpeg_managed_path()?;
    if managed.is_file() {
        return Ok(managed);
    }
    if let Some(p) = which("ffmpeg") {
        return Ok(p);
    }
    bail!("ffmpeg.exe not found. Click \"Download FFmpeg\" in Settings, or point Offspring at an existing ffmpeg.exe.")
}

fn which(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let candidate = dir.join(format!("{name}.exe"));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

pub fn output_path(input: &EncodeInput, preset: &Preset) -> PathBuf {
    let ext = match preset.format {
        Format::Gif => "gif",
        Format::Mp4 => "mp4",
        // Image: extension comes from the chosen codec. None falls
        // back to PNG — same as the encode branch's default.
        Format::Image => preset
            .image_codec
            .as_ref()
            .map(|c| c.ext())
            .unwrap_or("png"),
    };
    let base = input
        .output_dir()
        .join(format!("{}{}.{ext}", input.output_stem(), preset.suffix));
    unique_output_path(&base)
}

/// Standard image extensions Offspring recognises as "still image
/// input". Used to:
///   * Refuse video-format presets on image inputs with a clear error
///     (rather than letting ffmpeg produce nonsense).
///   * Refuse Trim/Merge tool invocations on image-only selections.
///   * Pick the right encode pipeline in `encode_file` and the tools.
///
/// Lowercase comparison; lives next to the format dispatch in
/// `encode_file` so the list stays close to the code that depends on it.
pub fn is_image_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("png")
            | Some("jpg")
            | Some("jpeg")
            | Some("webp")
            | Some("avif")
            | Some("bmp")
            | Some("tif")
            | Some("tiff")
    )
}

/// If `path` doesn't exist, return it. Otherwise return the first
/// `<stem>_NN.<ext>` (NN = 01, 02, …) that doesn't exist. Keeps every
/// encode non-destructive — re-running a preset on the same input stacks
/// outputs instead of silently overwriting the previous result.
///
/// The suffix starts at `_01` so the first collision becomes
/// `foo_01.mp4`, which reads as "the next copy" rather than "a missing
/// zeroth". Hard cap at 99 — if someone genuinely has 99 identically
/// named encodes in one folder they have bigger problems, and returning
/// the original path at that point means ffmpeg will overwrite rather
/// than loop forever.
pub fn unique_output_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or(Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    for n in 1..=99u32 {
        let candidate = if ext.is_empty() {
            parent.join(format!("{stem}_{n:02}"))
        } else {
            parent.join(format!("{stem}_{n:02}.{ext}"))
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_path_buf()
}

fn crop_expr(c: &Crop) -> &'static str {
    match c {
        Crop::H16x9 => "crop='min(iw,ih*16/9)':'min(ih,iw*9/16)'",
        Crop::V9x16 => "crop='min(iw,ih*9/16)':'min(ih,iw*16/9)'",
        Crop::S1x1 => "crop='min(iw,ih)':'min(iw,ih)'",
        Crop::H4x3 => "crop='min(iw,ih*4/3)':'min(ih,iw*3/4)'",
    }
}

fn scale_expr(preset: &Preset) -> Option<String> {
    match (preset.width, preset.height) {
        (Some(w), Some(h)) => Some(format!("scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2")),
        (Some(w), None) => Some(format!("scale={w}:-2:flags=lanczos")),
        (None, Some(h)) => Some(format!("scale=-2:{h}:flags=lanczos")),
        (None, None) => None,
    }
}

fn build_filter_chain(preset: &Preset) -> String {
    let mut parts: Vec<String> = Vec::new();
    // Free-form crop rectangle from the Crop tool runs FIRST so every
    // subsequent filter (fps, scale, grayscale, etc.) sees only the
    // cropped region. This matches the user's mental model: "crop is
    // what I drew on the preview, then everything else."
    if let Some((x, y, w, h)) = preset.crop_rect {
        parts.push(format!("crop={w}:{h}:{x}:{y}"));
    }
    if let Some(fps) = preset.fps {
        parts.push(format!("fps={fps}"));
    }
    if let Some(ref c) = preset.crop {
        parts.push(crop_expr(c).to_string());
    }
    if let Some(s) = scale_expr(preset) {
        parts.push(s);
    }
    if preset.grayscale.unwrap_or(false) {
        // `format=gray` is a one-pass desaturate that the encoder still
        // re-packs to yuv420p afterwards (the `-pix_fmt yuv420p` arg
        // later in the MP4 path handles that). Placed last so any
        // upstream crop/scale runs on the original color data.
        parts.push("format=gray".to_string());
    }
    if let Some(ref g) = preset.guides {
        parts.extend(guides_filters(g));
    }
    if let Some(ref o) = preset.overlay {
        parts.extend(overlay_filters(o));
    }
    if preset.timecode.unwrap_or(false) {
        parts.push(timecode_filter());
    }
    // Modify-tool transforms run AFTER cropping / scaling / overlay
    // so the user's mental model ("flip the result") matches the
    // pixel reality. Order: rotate → hflip → vflip → reverse.
    //
    // Rotation goes BEFORE flips so a user who picks "90° CW + flip
    // horizontal" sees the flip applied to the rotated frame, not the
    // source orientation. `transpose=1` is 90° CW, `transpose=2` is
    // 90° CCW (= 270° CW). 180° is two chained `transpose=1` calls,
    // which is cheaper than the float-math `rotate=PI` filter and
    // produces identical pixels.
    match preset.modify_rotate.unwrap_or(0) {
        90 => parts.push("transpose=1".to_string()),
        180 => {
            parts.push("transpose=1".to_string());
            parts.push("transpose=1".to_string());
        }
        270 => parts.push("transpose=2".to_string()),
        _ => {}
    }
    if preset.modify_flip_h.unwrap_or(false) {
        parts.push("hflip".to_string());
    }
    if preset.modify_flip_v.unwrap_or(false) {
        parts.push("vflip".to_string());
    }
    if preset.modify_reverse.unwrap_or(false) {
        // `reverse` buffers every frame before writing — fine for
        // short clips, painful for long ones. Accept the limit; the
        // dialog warns users.
        parts.push("reverse".to_string());
    }
    parts.join(",")
}

/// Burn-in drawtext for the current frame number. Uses Windows'
/// stock Consolas font — guaranteed on Win7+, zero-byte bundle cost.
/// The `:` in the `C:/...` path is ffmpeg's parameter separator, so
/// we escape it with `\:` (written `\\:` in the Rust source).
fn timecode_filter() -> String {
    r"drawtext=fontfile='C\:/Windows/Fonts/consola.ttf':text='%{frame_num}':fontcolor=white:fontsize=h/20:x=12:y=12:box=1:boxcolor=black@0.55:boxborderw=6".to_string()
}

/// drawbox + drawtext filters for the guide boxes. One box per enabled
/// ratio, sized to fit within the source frame (letterbox logic) so the
/// box represents the final crop window for each aspect. Each box is
/// followed by a small label (`16:9`, `9:16`, `4:5`) pinned to its
/// top-right corner. Opacity comes from [`GuidesConfig::opacity`].
pub(crate) fn guides_filters(g: &GuidesConfig) -> Vec<String> {
    let mut out = Vec::new();
    let a = g.opacity.clamp(0.0, 1.0);
    if g.show_16_9 {
        out.extend(guide_box_with_label("16/9", "16:9", &color_with_alpha(&g.color_16_9, a)));
    }
    if g.show_9_16 {
        out.extend(guide_box_with_label("9/16", "9:16", &color_with_alpha(&g.color_9_16, a)));
    }
    if g.show_4_5 {
        out.extend(guide_box_with_label("4/5", "4:5", &color_with_alpha(&g.color_4_5, a)));
    }
    out
}

/// Strict whitelist of color tokens accepted from user-controlled
/// preset/settings fields. Accepts only:
///   * one of the basic named colors the UI dropdown can produce
///     (white/black/red/green/blue/yellow/cyan/magenta), or
///   * a hex literal in `#rrggbb`, `#rrggbbaa`, `0xrrggbb`, or
///     `0xrrggbbaa` form.
///
/// Anything else (extra `:`/`,`/`@` separators, unknown words, malformed
/// hex) falls back to `white` — same fallback the existing empty-string
/// branch used. Defense-in-depth against filter-graph injection: color
/// values flow into unquoted ffmpeg filter args
/// (`drawbox=...:color={c}:thickness=3`), so without a whitelist a
/// string like `red:thickness=99999` would inject extra k/v pairs.
fn sanitize_color(c: &str) -> String {
    const NAMED: &[&str] = &[
        "white", "black", "red", "green", "blue", "yellow", "cyan", "magenta",
    ];
    let trimmed = c.trim();
    if trimmed.is_empty() {
        return "white".to_string();
    }
    let lowered = trimmed.to_ascii_lowercase();
    if NAMED.iter().any(|n| *n == lowered) {
        return lowered;
    }
    let hex_body: Option<&str> = if let Some(rest) = trimmed.strip_prefix('#') {
        Some(rest)
    } else if let Some(rest) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        Some(rest)
    } else {
        None
    };
    if let Some(rest) = hex_body {
        if matches!(rest.len(), 6 | 8) && rest.chars().all(|c| c.is_ascii_hexdigit()) {
            return format!("0x{}", rest.to_ascii_lowercase());
        }
    }
    "white".to_string()
}

/// Return an ffmpeg-parseable color string with the given alpha baked in.
/// Routes through [`sanitize_color`] so a malformed/hostile color field
/// can't inject extra filter k/v pairs. Empty strings fall back to white
/// rather than producing `@0.9` alone, which ffmpeg rejects with a
/// filter-init error.
fn color_with_alpha(c: &str, alpha: f32) -> String {
    let a = alpha.clamp(0.0, 1.0);
    let base = sanitize_color(c);
    format!("{base}@{a:.2}")
}

/// Emit a drawbox + a drawtext label, both sized/placed relative to the
/// largest rect of the given aspect that fits inside the source frame
/// (centered). `ratio` is a fraction literal like "16/9"; `label` is
/// human-readable like "16:9" (colons will be escaped for drawtext).
fn guide_box_with_label(ratio: &str, label: &str, color: &str) -> Vec<String> {
    // Commas inside if() arguments are filter-graph separators, so they
    // must be backslash-escaped. drawbox's `x` / `y` expressions can
    // reference the computed `w` / `h`, so we compute box dims by
    // comparing source aspect to target, then center.
    let box_filter = format!(
        "drawbox=w=if(gt(iw/ih\\,{r})\\,ih*{r}\\,iw):h=if(gt(iw/ih\\,{r})\\,ih\\,iw/({r})):x=(iw-w)/2:y=(ih-h)/2:color={c}:thickness=3",
        r = ratio,
        c = color,
    );

    // Label lives at the top-right inside the box. The box rect isn't
    // addressable by name in drawtext, so we inline the same box-width
    // expression and offset by `tw` (text width) + a small margin.
    //
    // drawtext's x/y expressions DO NOT accept `iw`/`ih` (those are
    // drawbox-only). The equivalents in drawtext are `W`/`H` — the
    // padded input width/height. Using `iw`/`ih` here makes the filter
    // parser fail with "Undefined constant or missing '(' in
    // 'iw/ih,<r>),...'" which kills the whole encode.
    let label_escaped = escape_drawtext_literal(label);
    let bw = format!("if(gt(W/H\\,{r})\\,H*{r}\\,W)", r = ratio);
    let bh = format!("if(gt(W/H\\,{r})\\,H\\,W/({r}))", r = ratio);
    let x_expr = format!("(W-{bw})/2+{bw}-tw-8");
    let y_expr = format!("(H-{bh})/2+6");
    let label_filter = format!(
        "drawtext=fontfile='C\\:/Windows/Fonts/consola.ttf':text='{text}':fontcolor={c}:fontsize=h/40:x={x}:y={y}:box=1:boxcolor=black@0.45:boxborderw=3",
        text = label_escaped,
        c = color,
        x = x_expr,
        y = y_expr,
    );

    vec![box_filter, label_filter]
}

/// Build the filter segments for the Overlay tool. Emits (in order):
/// optional `drawbox` guide boxes drawn on the source-sized frame, an
/// optional `pad` adding black bars top+bottom for the border mode, and
/// one `drawtext` per non-empty corner. Guides run BEFORE pad so the
/// aspect boxes hug the image, not the black border strips. Corners +
/// border are gated on `cfg.metadata` (the "Add metadata" toggle);
/// guides themselves are gated by the per-ratio booleans inside
/// `cfg.guides`, so an all-false GuidesConfig emits nothing.
pub(crate) fn overlay_filters(cfg: &OverlayConfig) -> Vec<String> {
    let mut out = Vec::new();

    // Guide boxes over the un-padded image, using the guides config's
    // per-ratio colors so picker changes propagate here too.
    out.extend(guides_filters(&cfg.guides));

    if !cfg.metadata {
        return out;
    }

    // Border: pad with an equal black strip on ALL FOUR sides (ih/10 on
    // each). Equal borders keep the output visually balanced even when
    // the left/right strips have no text to carry. Must run AFTER the
    // guide boxes so the guides hug the image, not the padding.
    if cfg.border {
        out.push("pad=iw+2*(ih/10):ih+2*(ih/10):(ih/10):(ih/10):color=black".to_string());
    }

    // One drawtext per corner. Timecode slots bypass the literal-text
    // escape path so the `%{frame_num}` expansion survives.
    let corners: [(&OverlaySlotKind, &str); 4] = [
        (&cfg.top_left, "tl"),
        (&cfg.top_right, "tr"),
        (&cfg.bottom_left, "bl"),
        (&cfg.bottom_right, "br"),
    ];
    for (slot, corner) in corners {
        match slot {
            OverlaySlotKind::None => {}
            OverlaySlotKind::Filename => {
                if !cfg.filename.is_empty() {
                    out.push(overlay_drawtext(
                        &escape_drawtext_literal(&cfg.filename),
                        corner,
                        &cfg.color,
                        cfg.opacity,
                        cfg.border,
                        cfg.font_scale,
                    ));
                }
            }
            OverlaySlotKind::Timecode => {
                // `%{frame_num}` is an ffmpeg expansion — must not be
                // escaped. The literal braces are fine inside single
                // quotes.
                out.push(overlay_drawtext(
                    "%{frame_num}",
                    corner,
                    &cfg.color,
                    cfg.opacity,
                    cfg.border,
                    cfg.font_scale,
                ));
            }
            OverlaySlotKind::Custom => {
                let t = cfg.custom_text.trim();
                if !t.is_empty() {
                    out.push(overlay_drawtext(
                        &escape_drawtext_literal(t),
                        corner,
                        &cfg.color,
                        cfg.opacity,
                        cfg.border,
                        cfg.font_scale,
                    ));
                }
            }
            OverlaySlotKind::Custom2 => {
                let t = cfg.custom_text_2.trim();
                if !t.is_empty() {
                    out.push(overlay_drawtext(
                        &escape_drawtext_literal(t),
                        corner,
                        &cfg.color,
                        cfg.opacity,
                        cfg.border,
                        cfg.font_scale,
                    ));
                }
            }
        }
    }

    out
}

/// Build one drawtext filter for a given corner. `text_expr` must
/// already be escaped for drawtext's `text=` value (call
/// [`escape_drawtext_literal`] for user strings; pass expansions like
/// `%{frame_num}` verbatim). When `border` is true, x positions are
/// pulled inward by the border width (`h/12` in post-pad coordinates)
/// so text lands on the image rather than in the left/right black
/// strips of the equal-border pad.
fn overlay_drawtext(
    text_expr: &str,
    corner: &str,
    color: &str,
    opacity: f32,
    border: bool,
    font_scale: f32,
) -> String {
    // Everything scales off `s`: fontsize (smaller divisor = larger text),
    // vertical margin (same), horizontal pixel pad, and the drawtext box
    // border width. Clamped so extreme slider values don't produce filter
    // strings that ffmpeg rejects (e.g. `fontsize=h/0.00`).
    let s = font_scale.clamp(0.3, 4.0);
    let font_div = 25.0 / s;
    let y_margin_div = 30.0 / s;
    let x_pad = ((12.0 * s).round() as u32).max(1);
    let box_bw = ((6.0 * s).round() as u32).max(1);
    // Border strip is a fixed fraction of the padded frame (`h/12` in
    // post-pad coords), so its thickness doesn't scale with font size —
    // only the inner text margin (`x_pad`) inside that strip does.
    let (x, y) = if border {
        match corner {
            "tl" => (format!("h/12+{x_pad}"), format!("h/{y_margin_div:.2}")),
            "tr" => (format!("w-h/12-tw-{x_pad}"), format!("h/{y_margin_div:.2}")),
            "bl" => (format!("h/12+{x_pad}"), format!("h-th-h/{y_margin_div:.2}")),
            "br" => (format!("w-h/12-tw-{x_pad}"), format!("h-th-h/{y_margin_div:.2}")),
            _ => (format!("h/12+{x_pad}"), format!("h/{y_margin_div:.2}")),
        }
    } else {
        match corner {
            "tl" => (format!("{x_pad}"), format!("h/{y_margin_div:.2}")),
            "tr" => (format!("w-tw-{x_pad}"), format!("h/{y_margin_div:.2}")),
            "bl" => (format!("{x_pad}"), format!("h-th-h/{y_margin_div:.2}")),
            "br" => (format!("w-tw-{x_pad}"), format!("h-th-h/{y_margin_div:.2}")),
            _ => (format!("{x_pad}"), format!("h/{y_margin_div:.2}")),
        }
    };
    let a = opacity.clamp(0.0, 1.0);
    // Route the user-controlled color through the same whitelist
    // `color_with_alpha` uses, so a malformed value can't inject extra
    // `:k=v` pairs into the drawtext arg list. The resulting `0x…` /
    // named-color string contains no `:` `,` `@`, all of which would
    // otherwise be filter-grammar separators here.
    let color_clean = sanitize_color(color);
    format!(
        "drawtext=fontfile='C\\:/Windows/Fonts/consola.ttf':text='{text}':fontcolor={color}@{a:.2}:fontsize=h/{font_div:.2}:x={x}:y={y}:box=1:boxcolor=black@{box_a:.2}:boxborderw={box_bw}",
        text = text_expr,
        color = color_clean,
        a = a,
        x = x,
        y = y,
        box_a = (a * 0.55).clamp(0.0, 1.0),
    )
}

/// Escape a literal string for drawtext `text='...'`. We wrap text in
/// single quotes in the filter, so we escape: backslash, single-quote,
/// colon (ffmpeg param separator), percent (format expansion), comma
/// (filter-graph separator).
fn escape_drawtext_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            ':' => out.push_str("\\:"),
            '%' => out.push_str("\\%"),
            ',' => out.push_str("\\,"),
            _ => out.push(c),
        }
    }
    out
}

fn dither_arg(d: &Dither, bayer_scale: Option<u32>) -> String {
    match d {
        Dither::Bayer => format!("dither=bayer:bayer_scale={}", bayer_scale.unwrap_or(3)),
        Dither::FloydSteinberg => "dither=floyd_steinberg".into(),
        Dither::Sierra2 => "dither=sierra2".into(),
        Dither::Sierra24a => "dither=sierra2_4a".into(),
        Dither::None => "dither=none".into(),
    }
}

/// Parse ffmpeg bitrate strings like "128k", "2M", "500000" into kbit/s.
fn parse_kbps(s: &str) -> u32 {
    let t = s.trim();
    let (num, suffix) = t.split_at(t.len().saturating_sub(1));
    match suffix {
        "k" | "K" => num.parse::<u32>().unwrap_or(0),
        "m" | "M" => num.parse::<u32>().unwrap_or(0).saturating_mul(1000),
        _ => {
            // assume raw bits/s
            (t.parse::<u64>().unwrap_or(0) / 1000) as u32
        }
    }
}

/// Compute the target video bitrate (kbit/s) to hit `target_mb` given a clip
/// duration and audio bitrate. Applies a 5% safety margin for container
/// overhead. Floored at 64 kbit/s so ffmpeg doesn't crash.
fn target_video_kbps(target_mb: u32, duration_s: f64, audio_kbps: u32) -> u32 {
    if duration_s <= 0.1 {
        return 64;
    }
    let total_kbits = (target_mb as f64) * 8.0 * 1024.0; // 1 MB = 1024 KB of data here
    let total_kbps = total_kbits / duration_s * 0.95;
    let v = total_kbps - audio_kbps as f64;
    v.max(64.0) as u32
}

#[derive(Serialize, Clone, Debug)]
pub struct ProgressEvent {
    pub file_index: usize,
    pub total_files: usize,
    pub input: String,
    pub stage: String, // "palette" | "encode" | "done" | "error"
    pub percent: Option<f32>,
    pub message: Option<String>,
}

pub fn encode_file(
    ffmpeg: &Path,
    input: &EncodeInput,
    preset: &Preset,
    settings: &Settings,
    duration_s: Option<f64>,
    file_index: usize,
    total_files: usize,
    mut on_progress: impl FnMut(ProgressEvent),
) -> Result<PathBuf> {
    let out = output_path(input, preset);
    let verbosity = settings.verbosity.clone().unwrap_or_else(|| "warning".into());
    let target_mb = preset.target_max_mb;
    let input_display = input.display();

    match preset.format {
        Format::Gif => {
            // Target size for GIFs is handled by iterating: encode, measure,
            // shrink width by sqrt(target/actual) * 0.9 if over budget.
            // Up to MAX_ATTEMPTS tries so we never spin forever.
            const MAX_ATTEMPTS: u32 = 4;

            let mut width_override = preset.width;
            for attempt in 1..=MAX_ATTEMPTS {
                encode_gif_once(
                    ffmpeg,
                    input,
                    preset,
                    width_override,
                    &verbosity,
                    &out,
                    duration_s,
                    file_index,
                    total_files,
                    &mut on_progress,
                    if attempt == 1 {
                        None
                    } else {
                        Some(format!("Retry {} — fitting into {} MB", attempt, target_mb.unwrap_or(0)))
                    },
                )?;

                // Success condition: no target, or file within budget.
                let Some(target_mb_v) = target_mb else { break };
                let actual_bytes = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
                let target_bytes: u64 = (target_mb_v as u64) * 1024 * 1024;
                if actual_bytes == 0 || actual_bytes <= target_bytes || attempt == MAX_ATTEMPTS {
                    break;
                }

                // Shrink width for next pass. Starting width is either the
                // explicit width_override or a fallback (500 is a reasonable
                // GIF default).
                let current_w = width_override.unwrap_or(500) as f64;
                let ratio = (target_bytes as f64 / actual_bytes as f64).sqrt();
                let new_w = (current_w * ratio * 0.9).max(120.0) as u32;
                if new_w >= current_w as u32 {
                    break; // can't make progress
                }
                width_override = Some(new_w);
            }
        }
        Format::Mp4 => {
            let filter = build_filter_chain(preset);
            let codec = if preset.use_cuda.unwrap_or(false) { "h264_nvenc" } else { "libx264" };
            let preset_speed = preset.preset_speed.clone().unwrap_or_else(|| "medium".into());
            let crf = preset.crf.unwrap_or(23);
            let abr = preset.audio_bitrate.clone().unwrap_or_else(|| "128k".into());

            // Target-size override: compute explicit video bitrate from
            // duration + audio budget. Wins over both CRF and an explicit
            // video_bitrate field.
            let computed_vbr: Option<String> = match (target_mb, duration_s) {
                (Some(mb), Some(dur)) => {
                    let a_kbps = parse_kbps(&abr);
                    let v_kbps = target_video_kbps(mb, dur, a_kbps);
                    Some(format!("{v_kbps}k"))
                }
                _ => None,
            };

            let stage_msg = if let Some(ref vbr) = computed_vbr {
                format!("Encoding MP4 ({codec}) · {vbr} for {} MB target", target_mb.unwrap_or(0))
            } else {
                format!("Encoding MP4 ({codec})")
            };
            on_progress(ProgressEvent {
                file_index,
                total_files,
                input: input_display.clone(),
                stage: "encode".into(),
                percent: None,
                message: Some(stage_msg),
            });

            let mut cmd = Command::new(ffmpeg);
            cmd.args(["-v", &verbosity, "-y", "-hide_banner"]);
            // Modify-tool trim: insert `-ss <start> -to <end>` BEFORE
            // -i so ffmpeg input-seeks and stops decoding at `end`.
            // Much cheaper than filter-based `trim=` (which would
            // decode the whole clip and discard frames after the
            // fact). Only emitted when the input is a real file
            // (sequences don't support seek the same way).
            if matches!(input, EncodeInput::File(_)) {
                if let Some(s) = preset.modify_trim_start_sec {
                    cmd.args(["-ss", &format!("{:.3}", s)]);
                }
                if let Some(e) = preset.modify_trim_end_sec {
                    cmd.args(["-to", &format!("{:.3}", e)]);
                }
            }
            for a in input.input_args() {
                cmd.arg(a);
            }
            // Watermark vs. simple -vf path. When the Overlay tool's
            // watermark step is active, swap to -filter_complex so we
            // can pull the PNG in as a second input and composite it
            // on top of the user's normal filter chain. Otherwise the
            // single-input -vf path is identical to what it was before.
            if let Some(ref wm) = preset.watermark {
                cmd.args(["-i", &wm.path]);
                let inner = if filter.is_empty() { "null".to_string() } else { filter.clone() };
                let complex = format!(
                    "[1:v]scale={w}:{h}:flags=lanczos,format=rgba,colorchannelmixer=aa={op:.3}[wm];\
                     [0:v]{inner}[vid];\
                     [vid][wm]overlay=0:0[out]",
                    w = wm.clip_w,
                    h = wm.clip_h,
                    op = wm.opacity,
                    inner = inner
                );
                cmd.args(["-filter_complex", &complex]);
                cmd.args(["-map", "[out]"]);
                // Keep the main input's audio stream if present. The `?`
                // makes the map optional, so a silent clip (no audio
                // stream) doesn't fail the encode.
                cmd.args(["-map", "0:a?"]);
            } else if !filter.is_empty() {
                cmd.args(["-vf", &filter]);
            }
            cmd.args(["-c:v", codec, "-preset", &preset_speed]);
            if let Some(ref br) = computed_vbr {
                // target-size mode: cap with maxrate/bufsize so we actually fit
                let v_kbps: u32 = br.trim_end_matches('k').parse().unwrap_or(1000);
                let maxrate = format!("{}k", v_kbps * 110 / 100);
                let bufsize = format!("{}k", v_kbps * 2);
                cmd.args(["-b:v", br, "-maxrate", &maxrate, "-bufsize", &bufsize]);
            } else if let Some(ref br) = preset.video_bitrate {
                cmd.args(["-b:v", br]);
            } else {
                cmd.args(["-crf", &crf.to_string()]);
            }
            // Modify tool's "Remove audio" wins over every other
            // audio path: we ask ffmpeg to drop the stream entirely
            // with `-an`, which short-circuits the AAC re-encode and
            // any audio filters (areverse) we'd otherwise add. Cheaper
            // than encoding silence and gives a smaller output file.
            if preset.modify_remove_audio.unwrap_or(false) {
                cmd.arg("-an");
            } else {
                cmd.args(["-c:a", "aac", "-b:a", &abr]);
                // Modify-tool reverse: also reverse the audio so the
                // backwards video has backwards sound. Applied as a
                // separate -af filter chain on the audio stream because
                // build_filter_chain only constructs video (-vf) filters.
                // No-op when there's no audio stream — ffmpeg silently
                // skips audio filters in that case.
                if preset.modify_reverse.unwrap_or(false) {
                    cmd.args(["-af", "areverse"]);
                }
            }
            // `-pix_fmt yuv420p` is load-bearing for Windows Explorer's
            // thumbnail service — RGB24/RGBA sources (PNG sequences, EXR
            // renders) otherwise encode as yuv444p, which the shell
            // thumbnailer can't decode and renders as a corrupt frame.
            // yuv420p is the universal-compat default and harmless for
            // normal video inputs too.
            cmd.args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"]);
            // Image sequences have no audio track — skip the AAC encoder
            // so ffmpeg doesn't log a spurious warning, and so the output
            // stream layout exactly matches what the encoder produced.
            if matches!(input, EncodeInput::Sequence { .. }) {
                cmd.arg("-an");
            }
            cmd.args(["-progress", "pipe:1"]).arg(&out);
            run_with_progress_cleanup(cmd, duration_s, file_index, total_files, &input_display, "encode", &out, &mut on_progress)?;
        }
        Format::Image => {
            // Image preset on a non-image input is almost always user
            // error — invoking a "JPEG 85%" preset on a video would
            // either fail in ffmpeg or quietly produce a one-frame
            // poster, neither of which is clearly desirable. We refuse
            // up front rather than guess.
            //
            // (Future enhancement: a "Poster from video" preset that
            // explicitly extracts the first frame. That can ship as a
            // standalone preset/tool when there's a real demand.)
            if let EncodeInput::File(p) = input {
                if !is_image_path(p) {
                    bail!(
                        "This preset outputs a still image, but the input \
                         '{}' is not an image. Use a video preset (MP4 / GIF) \
                         for video inputs.",
                        p.display()
                    );
                }
            }

            let codec = preset.image_codec.clone().unwrap_or(crate::presets::ImageCodec::Png);
            let strip_meta = preset.strip_metadata.unwrap_or(false);
            let q_native = preset.image_quality.unwrap_or(codec.default_quality());

            // Reuse the video filter-chain builder for resize/crop/
            // greyscale/timecode — the same -vf graph works for stills
            // (every "video" filter in our chain is a per-frame op
            // that has no opinion about whether there's only one frame).
            let filter = build_filter_chain(preset);

            on_progress(ProgressEvent {
                file_index,
                total_files,
                input: input_display.clone(),
                stage: "encode".into(),
                percent: None,
                message: Some(format!("Encoding {}", codec.ext().to_ascii_uppercase())),
            });

            let mut cmd = Command::new(ffmpeg);
            cmd.args(["-v", &verbosity, "-y", "-hide_banner"]);
            for a in input.input_args() {
                cmd.arg(a);
            }
            // Same watermark vs -vf branching as the MP4 path — see
            // the long comment above. The Image branch never has an
            // audio stream to map, so the trailing `-map 0:a?` is
            // omitted here.
            if let Some(ref wm) = preset.watermark {
                cmd.args(["-i", &wm.path]);
                let inner = if filter.is_empty() { "null".to_string() } else { filter.clone() };
                let complex = format!(
                    "[1:v]scale={w}:{h}:flags=lanczos,format=rgba,colorchannelmixer=aa={op:.3}[wm];\
                     [0:v]{inner}[vid];\
                     [vid][wm]overlay=0:0[out]",
                    w = wm.clip_w,
                    h = wm.clip_h,
                    op = wm.opacity,
                    inner = inner
                );
                cmd.args(["-filter_complex", &complex]);
                cmd.args(["-map", "[out]"]);
            } else if !filter.is_empty() {
                cmd.args(["-vf", &filter]);
            }
            // -frames:v 1 caps the output to a single frame. Belt-and-
            // suspenders: still-image inputs already imply one frame,
            // but if a user ever points an image preset at an image
            // sequence (via the Sequence tool) this prevents a
            // multi-frame APNG/AVIS from being silently produced.
            cmd.args(["-frames:v", "1"]);

            match codec {
                crate::presets::ImageCodec::Png => {
                    // libpng. Compression level 0-9; 0 is fastest +
                    // largest, 9 is slowest + smallest. Quality is
                    // lossless either way.
                    let level = q_native.min(9).to_string();
                    cmd.args(["-c:v", "png", "-compression_level", &level]);
                }
                crate::presets::ImageCodec::Jpeg => {
                    // mjpeg encoder. Native q:v scale is 2-31 with
                    // LOWER = better. We expose 1-100 in the UI for
                    // photographer familiarity, then map back here.
                    // The mapping is linear over 31..2 — q_ui=100 →
                    // q:v=2, q_ui=1 → q:v=31. Clamp into the valid
                    // range so out-of-range stored values still encode.
                    let q_ui = q_native.clamp(1, 100) as f32;
                    let qv = (31.0 - (q_ui - 1.0) * 29.0 / 99.0).round() as u32;
                    let qv = qv.clamp(2, 31).to_string();
                    // pix_fmt yuvj420p forces full-range JPEG, which is
                    // what almost every viewer expects from a .jpg.
                    // libavcodec's mjpeg defaults to limited range
                    // otherwise and produces washed-out output on some
                    // decoders.
                    cmd.args([
                        "-c:v", "mjpeg",
                        "-q:v", &qv,
                        "-pix_fmt", "yuvj420p",
                    ]);
                }
                crate::presets::ImageCodec::Webp => {
                    // libwebp. Quality 0-100 native, no remapping.
                    let q = q_native.min(100).to_string();
                    cmd.args([
                        "-c:v", "libwebp",
                        "-quality", &q,
                        // Disable -lossless so quality has effect; we
                        // could expose lossless WebP via a future
                        // boolean if anyone asks.
                        "-lossless", "0",
                    ]);
                }
                crate::presets::ImageCodec::Avif => {
                    // libaom-av1 still-image. CRF 0-63 native, lower=better.
                    let crf = q_native.min(63).to_string();
                    cmd.args([
                        "-c:v", "libaom-av1",
                        "-crf", &crf,
                        // still-picture flag tells the encoder this is
                        // a one-frame stream and to write the AVIF
                        // sequence header accordingly. Without it some
                        // decoders (Photos.app on iOS, certain CDNs)
                        // refuse to display the file.
                        "-still-picture", "1",
                    ]);
                }
            }

            // Strip metadata (EXIF / GPS / camera serial) when the
            // preset asks for it. -map_metadata -1 drops the global
            // metadata block; for most image-codec containers that's
            // sufficient. JPEG also has the per-stream APP1 marker
            // which mjpeg's encoder strips by default in this config.
            if strip_meta {
                cmd.args(["-map_metadata", "-1"]);
            }

            cmd.arg(&out);
            // run_with_progress is overkill for a one-frame encode
            // (no `out_time_ms` to scrub against), but it gives us
            // consistent error handling + cancellation. The progress
            // bar will jump from "encoding" straight to "done" without
            // intermediate ticks, which is fine for sub-second encodes.
            run_with_progress_cleanup(
                cmd,
                None,
                file_index,
                total_files,
                &input_display,
                "encode",
                &out,
                &mut on_progress,
            )?;
        }
    }

    on_progress(ProgressEvent {
        file_index,
        total_files,
        input: input_display,
        stage: "done".into(),
        percent: Some(1.0),
        message: Some(out.display().to_string()),
    });

    Ok(out)
}

/// Watchdog timeout for `run_with_progress`. If ffmpeg goes this long
/// without emitting a new line on its `-progress pipe:1` stdout, we
/// assume it's stalled (e.g. infinite filter-source feeding into a
/// stack filter that never EOFs) and kill the process.
///
/// 90s is generous: ffmpeg's `-progress` cadence is once per second
/// in normal operation, so even on a heavily-throttled CPU or a very
/// slow encoder we'd expect lines every few seconds at worst. A
/// 90s gap is a clear signal of "hung", not "slow".
const FFMPEG_STALL_TIMEOUT: Duration = Duration::from_secs(90);

/// How often `run_with_progress` wakes up between received lines to
/// check for cancellation + stall. 1s is fine-grained enough that a
/// user clicking "cancel" sees the ffmpeg child die within a second,
/// without burning measurable CPU on the polling itself.
const FFMPEG_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Process-wide encode-cancel flag. Set by `request_cancel()` (called
/// from the `cancel_encode` Tauri command when the user clicks ✕ on
/// the progress window) and cleared by `reset_cancel()` at the top of
/// every encode entrypoint. `run_with_progress` polls it once per
/// second and kills the ffmpeg child + bails when it sees `true`.
///
/// Singleton (rather than threaded through every encode function) for
/// two reasons:
///   1. Only one encode is in flight at a time in practice — the
///      progress window is modal-ish and starts the encode on dialog
///      close, no parallel jobs from the same UI.
///   2. The encode functions live deep inside `ffmpeg.rs` and don't
///      have access to Tauri state; threading an `Arc<AtomicBool>`
///      through ~14 call sites is a lot of plumbing for no real
///      benefit over a static.
static CANCEL: OnceLock<AtomicBool> = OnceLock::new();

fn cancel_flag() -> &'static AtomicBool {
    CANCEL.get_or_init(|| AtomicBool::new(false))
}

/// Request that any in-flight ffmpeg encode abort ASAP. Exposed to
/// Tauri via `commands::cancel_encode`.
pub fn request_cancel() {
    cancel_flag().store(true, Ordering::SeqCst);
}

/// Clear the cancel flag. Called at the top of every encode-command
/// entrypoint so a previous cancellation doesn't immediately abort
/// the new job.
pub fn reset_cancel() {
    cancel_flag().store(false, Ordering::SeqCst);
}

pub fn is_cancelled() -> bool {
    cancel_flag().load(Ordering::SeqCst)
}

/// Best-effort delete of a partial / invalid output file. No-op if
/// the file doesn't exist or the delete fails (e.g. another process
/// still has it open). Called from encode entrypoints when
/// `run_with_progress` returns an error — covers user cancellation,
/// the stall watchdog, and any ffmpeg-internal failure that left a
/// truncated file on disk (a partial MP4 without its moov atom is
/// just confusing junk; deleting is friendlier than leaving it).
pub fn cleanup_partial_output(out: &Path) {
    if out.exists() {
        let _ = std::fs::remove_file(out);
    }
}

fn run_with_progress(
    mut cmd: Command,
    duration_s: Option<f64>,
    file_index: usize,
    total_files: usize,
    input_display: &str,
    stage: &str,
    on_progress: &mut impl FnMut(ProgressEvent),
) -> Result<()> {
    // Capture stderr (was Stdio::null()) so that when ffmpeg fails we
    // can include the actual error in the bail!() message. Without
    // this the user sees a bare exit code like "0xdfaba7bb" and we
    // have nothing to diagnose with. A background thread drains the
    // pipe so it doesn't fill up and block ffmpeg's writes.
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_console(&mut cmd);
    let mut child = cmd.spawn().context("spawning ffmpeg")?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;

    let stderr_thread = std::thread::spawn(move || {
        BufReader::new(stderr)
            .lines()
            .map_while(|l| l.ok())
            .collect::<Vec<String>>()
    });

    // Spawn the stdout reader on its own thread, piping lines through a
    // channel. That lets the main thread `recv_timeout` for the stall
    // watchdog — if ffmpeg goes silent we can detect it and kill the
    // child, rather than blocking forever on `reader.lines()`.
    //
    // The channel auto-disconnects when the reader thread finishes
    // (ffmpeg closed its stdout / exited cleanly), which is how we
    // distinguish "graceful end of output" from "stalled".
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let stdout_thread = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
            if tx.send(line).is_err() {
                // Receiver dropped (main thread bailed) — stop reading.
                break;
            }
        }
    });

    // The main loop polls `rx` once per `FFMPEG_POLL_INTERVAL` (1s).
    // Each iteration we either get a line (progress / log noise), hit
    // a poll-tick timeout (check cancel + stall), or see the channel
    // disconnect (child exited and reader thread finished).
    //
    // Cancel and stall are decoupled: the stall counter resets on
    // every received line, so a slow-but-still-progressing encode
    // never trips it. Cancel checks every tick regardless.
    let mut stalled = false;
    let mut cancelled = false;
    let mut last_progress = Instant::now();
    loop {
        match rx.recv_timeout(FFMPEG_POLL_INTERVAL) {
            Ok(line) => {
                last_progress = Instant::now();
                if let Some(rest) = line.strip_prefix("out_time_ms=") {
                    if let (Ok(us), Some(total)) = (rest.trim().parse::<i64>(), duration_s) {
                        let s = us as f64 / 1_000_000.0;
                        let pct = (s / total).clamp(0.0, 1.0) as f32;
                        on_progress(ProgressEvent {
                            file_index,
                            total_files,
                            input: input_display.to_string(),
                            stage: stage.into(),
                            percent: Some(pct),
                            message: None,
                        });
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // User cancelled — kill the child IMMEDIATELY. We
                // don't wait for it to gracefully shut down; ffmpeg
                // doesn't really do graceful interrupt anyway, and
                // any partial output is going to be cleaned up by
                // the caller via `cleanup_partial_output`.
                if is_cancelled() {
                    let _ = child.kill();
                    cancelled = true;
                    break;
                }
                // No output for FFMPEG_STALL_TIMEOUT → stalled.
                // Verify the child is still alive before declaring
                // a stall (narrow race: could have exited just as
                // the timeout fired and we haven't reaped it yet).
                if last_progress.elapsed() > FFMPEG_STALL_TIMEOUT {
                    match child.try_wait() {
                        Ok(Some(_)) => break,           // already exited; fall through
                        Ok(None) => {                    // still running, no output → hung
                            let _ = child.kill();
                            stalled = true;
                            break;
                        }
                        Err(_) => break,                 // can't query; treat as exited
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let _ = stdout_thread.join();
    let status = child.wait()?;
    let stderr_lines = stderr_thread.join().unwrap_or_default();

    if cancelled {
        // User asked to abort. Specific error message so callers know
        // this was an intentional cancel (vs. a real failure) — useful
        // for surfacing a different UI state ("Cancelled" rather than
        // "Failed with errors"). The caller is expected to delete the
        // partial output file via `cleanup_partial_output`.
        bail!("Encode cancelled.");
    }

    if stalled {
        // Watchdog killed it. Surface the stall plus whatever stderr
        // existed up to that point — sometimes ffmpeg complains
        // verbosely before going silent (e.g. "filter graph: input
        // pad not connected") and the tail still helps.
        let tail = stderr_lines
            .iter()
            .rev()
            .take(15)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let summary = if tail.is_empty() {
            "(no stderr before stall)".to_string()
        } else {
            tail
        };
        bail!(
            "ffmpeg stalled — no progress for {}s, killed by watchdog\n--- last stderr lines ---\n{}",
            FFMPEG_STALL_TIMEOUT.as_secs(),
            summary
        );
    }

    if !status.success() {
        // Show the last ~15 lines of stderr in the error — that's
        // usually where ffmpeg prints the actual reason (filter graph
        // parse error, missing codec, etc.). Earlier lines are mostly
        // banner / probe noise. Falls back to a placeholder if stderr
        // was empty (rare; means ffmpeg crashed before writing anything).
        let tail = stderr_lines
            .iter()
            .rev()
            .take(15)
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let summary = if tail.is_empty() {
            "(no stderr captured)".to_string()
        } else {
            tail
        };
        bail!("ffmpeg exited with status {status}\n--- last stderr lines ---\n{summary}");
    }
    Ok(())
}

/// `run_with_progress` plus best-effort cleanup of `out` on any error.
/// The cleanup covers three cases — user cancellation, stall watchdog
/// kill, and any ffmpeg-internal failure that left a truncated file on
/// disk. Callers that write to a specific output path should use this
/// wrapper instead of bare `run_with_progress` so an aborted encode
/// doesn't leave a partial / invalid file behind.
#[allow(clippy::too_many_arguments)]
fn run_with_progress_cleanup(
    cmd: Command,
    duration_s: Option<f64>,
    file_index: usize,
    total_files: usize,
    input_display: &str,
    stage: &str,
    out: &Path,
    on_progress: &mut impl FnMut(ProgressEvent),
) -> Result<()> {
    match run_with_progress(
        cmd, duration_s, file_index, total_files, input_display, stage, on_progress,
    ) {
        Ok(()) => Ok(()),
        Err(e) => {
            cleanup_partial_output(out);
            Err(e)
        }
    }
}

/// One GIF encode pass (palettegen + paletteuse). `width_override` lets the
/// caller shrink the output between iterations when hitting a size target.
#[allow(clippy::too_many_arguments)]
fn encode_gif_once(
    ffmpeg: &Path,
    input: &EncodeInput,
    preset: &Preset,
    width_override: Option<u32>,
    verbosity: &str,
    out: &Path,
    duration_s: Option<f64>,
    file_index: usize,
    total_files: usize,
    on_progress: &mut impl FnMut(ProgressEvent),
    extra_msg: Option<String>,
) -> Result<()> {
    let input_display = input.display();
    let palette_colors = preset.palette_colors.unwrap_or(128);
    let dither = preset.dither.clone().unwrap_or(Dither::Bayer);

    // Build filter chain honoring the width override.
    let mut parts: Vec<String> = Vec::new();
    if let Some(fps) = preset.fps {
        parts.push(format!("fps={fps}"));
    }
    if let Some(ref c) = preset.crop {
        parts.push(crop_expr(c).to_string());
    }
    match (width_override.or(preset.width), preset.height) {
        (Some(w), Some(h)) => parts.push(format!(
            "scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2"
        )),
        (Some(w), None) => parts.push(format!("scale={w}:-2:flags=lanczos")),
        (None, Some(h)) => parts.push(format!("scale=-2:{h}:flags=lanczos")),
        (None, None) => {}
    }
    if preset.grayscale.unwrap_or(false) {
        // Runs before palettegen so the generated palette contains only
        // grey tones — avoids spurious colored dithering when the
        // source happens to have a few stray non-grey pixels.
        parts.push("format=gray".to_string());
    }
    if let Some(ref g) = preset.guides {
        parts.extend(guides_filters(g));
    }
    if let Some(ref o) = preset.overlay {
        parts.extend(overlay_filters(o));
    }
    if preset.timecode.unwrap_or(false) {
        parts.push(timecode_filter());
    }
    let filter = parts.join(",");

    // Pass 1: palette
    //
    // Previous versions wrote the palette next to the output file. That
    // breaks on read-only source folders (rare) and races with cloud sync
    // clients (common — OneDrive/Dropbox briefly lock newly-created files
    // in watched folders, so the first encode after a sync event fails
    // while the second succeeds because the file is already known). Stage
    // under LOCALAPPDATA instead, with pid + timestamp to avoid two
    // concurrent encodes stomping each other's palette.
    let palette_tmp = {
        let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        crate::paths::tmp_dir()
            .unwrap_or_else(|_| std::env::temp_dir())
            .join(format!("{stem}.{}.{nonce}.palette.png", std::process::id()))
    };
    let mut filter_p1 = filter.clone();
    if !filter_p1.is_empty() {
        filter_p1.push(',');
    }
    filter_p1.push_str(&format!(
        "palettegen=max_colors={palette_colors}:stats_mode=full"
    ));

    on_progress(ProgressEvent {
        file_index,
        total_files,
        input: input_display.clone(),
        stage: "palette".into(),
        percent: None,
        message: Some(
            extra_msg
                .clone()
                .unwrap_or_else(|| "Generating palette".into()),
        ),
    });

    let mut palette_cmd = Command::new(ffmpeg);
    palette_cmd.args(["-v", verbosity, "-y"]);
    // Modify-tool trim. See the MP4 branch in encode_file for the
    // long-form comment. The palette pass needs the same -ss/-to as
    // the encode pass so palette generation samples the trimmed
    // range, not the full clip.
    if matches!(input, EncodeInput::File(_)) {
        if let Some(s) = preset.modify_trim_start_sec {
            palette_cmd.args(["-ss", &format!("{:.3}", s)]);
        }
        if let Some(e) = preset.modify_trim_end_sec {
            palette_cmd.args(["-to", &format!("{:.3}", e)]);
        }
    }
    for a in input.input_args() {
        palette_cmd.arg(a);
    }
    palette_cmd
        .args(["-vf", &filter_p1])
        .arg(&palette_tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut palette_cmd);
    // Delete the palette on every exit from this function (success, error,
    // or panic unwind) so we don't leak temp PNGs across crashed encodes.
    struct PaletteGuard(PathBuf);
    impl Drop for PaletteGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _palette_guard = PaletteGuard(palette_tmp.clone());

    let status = palette_cmd
        .status()
        .context("spawning ffmpeg for palette pass")?;
    if !status.success() {
        bail!("palette pass failed");
    }

    // Pass 2: apply palette
    let filter_complex = format!(
        "{filter}[x];[x][1:v]paletteuse={dither}",
        filter = if filter.is_empty() {
            "[0:v]null".to_string()
        } else {
            format!("[0:v]{filter}")
        },
        dither = dither_arg(&dither, preset.bayer_scale),
    );

    on_progress(ProgressEvent {
        file_index,
        total_files,
        input: input_display.clone(),
        stage: "encode".into(),
        percent: None,
        message: Some(extra_msg.unwrap_or_else(|| "Encoding GIF".into())),
    });

    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-v", verbosity, "-y", "-hide_banner"]);
    // Same Modify-trim input-seek pair as the palette pass above —
    // must match so both passes see the same frames.
    if matches!(input, EncodeInput::File(_)) {
        if let Some(s) = preset.modify_trim_start_sec {
            cmd.args(["-ss", &format!("{:.3}", s)]);
        }
        if let Some(e) = preset.modify_trim_end_sec {
            cmd.args(["-to", &format!("{:.3}", e)]);
        }
    }
    for a in input.input_args() {
        cmd.arg(a);
    }
    cmd.arg("-i")
        .arg(&palette_tmp)
        .args(["-filter_complex", &filter_complex])
        .args(["-progress", "pipe:1"])
        .arg(out);
    run_with_progress_cleanup(
        cmd,
        duration_s,
        file_index,
        total_files,
        &input_display,
        "encode",
        out,
        on_progress,
    )?;

    // `_palette_guard` drops here and removes the temp palette.
    Ok(())
}

pub fn probe_duration(ffmpeg: &Path, input: &Path) -> Option<f64> {
    // Derive ffprobe from ffmpeg path
    let probe = ffmpeg.with_file_name("ffprobe.exe");
    let probe = if probe.exists() { probe } else { return None };
    let mut probe_cmd = Command::new(probe);
    probe_cmd
        .args(["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1"])
        .arg(input)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut probe_cmd);
    let out = probe_cmd.output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.trim().parse::<f64>().ok()
}

/// Shape of the first-file probe that feeds the Merge tool's ad-hoc
/// preset. All fields are best-effort — missing values fall back to
/// sensible defaults in [`derive_merge_preset`].
#[derive(Debug, Clone, Default)]
pub struct VideoProbe {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<u32>,
}

/// Probe the first video stream of `input` for dimensions + fps. Used by
/// Merge to build an output that matches the first file in the selection.
/// Returns `VideoProbe::default()` (all-None) if ffprobe isn't available
/// or the file has no video stream we can read — the caller falls back
/// to reasonable defaults.
pub fn probe_video(ffmpeg: &Path, input: &Path) -> VideoProbe {
    let probe = ffmpeg.with_file_name("ffprobe.exe");
    if !probe.exists() {
        return VideoProbe::default();
    }
    let mut cmd = Command::new(&probe);
    cmd.args([
        "-v", "error",
        "-select_streams", "v:0",
        "-show_entries", "stream=width,height,avg_frame_rate,r_frame_rate",
        "-of", "default=nw=1",
    ])
    .arg(input)
    .stdin(Stdio::null())
    .stderr(Stdio::null());
    hide_console(&mut cmd);
    let Ok(out) = cmd.output() else { return VideoProbe::default() };
    let text = String::from_utf8_lossy(&out.stdout);

    let mut p = VideoProbe::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "width" => p.width = v.trim().parse().ok(),
            "height" => p.height = v.trim().parse().ok(),
            // `avg_frame_rate` wins when present (actual playback rate);
            // fall back to `r_frame_rate` (declared rate) if we only saw
            // that one. GIF files typically only publish r_frame_rate.
            "avg_frame_rate" | "r_frame_rate" => {
                if p.fps.is_none() {
                    if let Some((num, den)) = v.trim().split_once('/') {
                        let n: f64 = num.parse().unwrap_or(0.0);
                        let d: f64 = den.parse().unwrap_or(0.0);
                        if d > 0.0 && n > 0.0 {
                            p.fps = Some((n / d).round() as u32);
                        }
                    } else if let Ok(n) = v.trim().parse::<f64>() {
                        if n > 0.0 {
                            p.fps = Some(n.round() as u32);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    p
}

/// Probe whether `input` has at least one audio stream. Used by the
/// merge-via-concat-filter path to decide whether to splice audio into
/// the concat graph. Conservative: returns `false` if ffprobe is
/// missing or the call fails, so the fallback (video-only merge)
/// always runs rather than silently dropping to a broken audio graph.
fn has_audio_stream(ffmpeg: &Path, input: &Path) -> bool {
    let probe = ffmpeg.with_file_name("ffprobe.exe");
    if !probe.exists() {
        return false;
    }
    let mut cmd = Command::new(&probe);
    cmd.args([
        "-v", "error",
        "-select_streams", "a:0",
        "-show_entries", "stream=codec_type",
        "-of", "default=nw=1:nk=1",
    ])
    .arg(input)
    .stdin(Stdio::null())
    .stderr(Stdio::null());
    hide_console(&mut cmd);
    let Ok(out) = cmd.output() else { return false };
    String::from_utf8_lossy(&out.stdout).trim() == "audio"
}

/// Merge N inputs into one MP4 using ffmpeg's **concat filter**
/// (`-filter_complex concat=n=N:v=1:a=?`) rather than the concat
/// demuxer. The filter re-encodes every input through a shared
/// normalization chain (scale→pad→setsar→fps→format=yuv420p) so
/// mismatched resolutions / framerates / pixel formats / codecs stop
/// being a silent failure. The demuxer required all inputs to share
/// those properties; when they didn't, ffmpeg would keep only the
/// first file's stream and produce a truncated output — which was the
/// 0.3.33 merge bug report ("output was only the first video; merging
/// to similar file formats worked fine").
///
/// Target width / height / fps are taken from `target_w`/`h`/`fps`
/// (caller typically probes the first input). All inputs are scaled to
/// fit and padded to match, preserving aspect ratio. Audio is concat'd
/// only if **every** input has an audio stream — otherwise the output
/// is silent. Mixed audio/no-audio selections aren't worth the
/// complexity of synthesizing silence to match.
#[allow(clippy::too_many_arguments)]
pub fn encode_merge_filter(
    ffmpeg: &Path,
    files: &[PathBuf],
    output: &Path,
    target_w: u32,
    target_h: u32,
    target_fps: u32,
    crf: u32,
    preset_speed: &str,
    audio_bitrate: &str,
    verbosity: &str,
    duration_s: Option<f64>,
    mut on_progress: impl FnMut(ProgressEvent),
) -> Result<()> {
    if files.len() < 2 {
        bail!("merge requires at least two inputs");
    }
    let n = files.len();
    let all_have_audio = files.iter().all(|p| has_audio_stream(ffmpeg, p));

    // Build the filter_complex graph. Each input gets normalized to
    // [v{i}] (and [a{i}] when audio is included); the final concat
    // node stitches them into [v]/[a].
    let mut graph = String::new();
    for i in 0..n {
        if i > 0 {
            graph.push(';');
        }
        graph.push_str(&format!(
            "[{i}:v]scale={w}:{h}:force_original_aspect_ratio=decrease,\
             pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,\
             setsar=1,fps={fps},format=yuv420p[v{i}]",
            i = i, w = target_w, h = target_h, fps = target_fps,
        ));
        if all_have_audio {
            // aresample with async=1 nudges each input's audio to line
            // up with the concat filter's PTS expectations — otherwise
            // tiny drift at boundaries causes concat to log
            // "Timestamps are unset" and occasionally drop samples.
            graph.push_str(&format!(
                ";[{i}:a]aresample=async=1:first_pts=0[a{i}]",
                i = i
            ));
        }
    }
    graph.push(';');
    for i in 0..n {
        graph.push_str(&format!("[v{i}]"));
        if all_have_audio {
            graph.push_str(&format!("[a{i}]"));
        }
    }
    let audio_flag = if all_have_audio { 1 } else { 0 };
    graph.push_str(&format!(
        "concat=n={n}:v=1:a={audio_flag}[v]"
    ));
    if all_have_audio {
        graph.push_str("[a]");
    }

    on_progress(ProgressEvent {
        file_index: 1,
        total_files: 1,
        input: format!("merge: {} files", n),
        stage: "encode".into(),
        percent: None,
        message: Some(format!(
            "Encoding MP4 — {target_w}x{target_h}@{target_fps}, {n} inputs{}",
            if all_have_audio { " (with audio)" } else { " (silent)" }
        )),
    });

    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-v", verbosity, "-y", "-hide_banner"]);
    for p in files {
        cmd.arg("-i").arg(p);
    }
    cmd.args(["-filter_complex", &graph]);
    cmd.args(["-map", "[v]"]);
    if all_have_audio {
        cmd.args(["-map", "[a]"]);
    }
    cmd.args([
        "-c:v", "libx264",
        "-preset", preset_speed,
        "-crf", &crf.to_string(),
        "-pix_fmt", "yuv420p",
        "-movflags", "+faststart",
    ]);
    if all_have_audio {
        cmd.args(["-c:a", "aac", "-b:a", audio_bitrate]);
    }
    cmd.args(["-progress", "pipe:1"]).arg(output);

    let input_display = format!("merge: {} files", n);
    run_with_progress_cleanup(cmd, duration_s, 1, 1, &input_display, "encode", output, &mut on_progress)?;

    on_progress(ProgressEvent {
        file_index: 1,
        total_files: 1,
        input: input_display,
        stage: "done".into(),
        percent: Some(1.0),
        message: Some(output.display().to_string()),
    });
    Ok(())
}

/// Build an ad-hoc [`Preset`] for the Merge tool by probing the first
/// file. Format comes from the first file's extension; dimensions and
/// fps from ffprobe; quality knobs from built-in defaults that match
/// each format's "looks right" baseline (CRF 23 / medium for MP4,
/// 128-color bayer for GIF).
///
/// The returned preset's `suffix` is `_merged` so the output lands as
/// `<first-stem>_merged.<ext>` next to the first file.
pub fn derive_merge_preset(ffmpeg: &Path, first: &Path) -> Preset {
    use crate::presets::{Dither, Format};

    let ext = first
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_ascii_lowercase();
    let format = if ext == "gif" { Format::Gif } else { Format::Mp4 };

    let probe = probe_video(ffmpeg, first);

    Preset {
        id: "__merge__".into(),
        name: "Merge".into(),
        enabled: true,
        format,
        // Suffix is empty because encode_merge constructs the output
        // name itself (`<first-stem>_merged`). Leaving it blank keeps
        // output_path from double-appending.
        suffix: String::new(),
        width: probe.width,
        height: probe.height,
        fps: probe.fps,
        crop: None,
        // GIF defaults — ignored when format=Mp4.
        palette_colors: Some(128),
        dither: Some(Dither::Bayer),
        bayer_scale: Some(3),
        // MP4 defaults — ignored when format=Gif.
        crf: Some(23),
        preset_speed: Some("medium".into()),
        video_bitrate: None,
        audio_bitrate: Some("128k".into()),
        use_cuda: Some(false),
        target_max_mb: None,
        image_codec: None,
        image_quality: None,
        strip_metadata: None,
        grayscale: None,
        timecode: None,
        guides: None,
        overlay: None,
        crop_rect: None,
        modify_flip_h: None,
        modify_flip_v: None,
        modify_reverse: None,
        modify_overwrite: None,
        modify_remove_audio: None,
        modify_rotate: None,
        modify_trim_start_sec: None,
        modify_trim_end_sec: None,
        watermark: None,
        icon: None,
        order: 0,
    }
}

/// Build an ad-hoc [`Preset`] for the Greyscale tool by probing the
/// input. Format comes from the file's extension; dimensions and fps
/// from ffprobe; quality knobs from the same "looks right" baseline the
/// Merge tool uses (CRF 23 / medium for MP4, 128-color bayer for GIF).
///
/// Image inputs (PNG / JPEG / WebP / AVIF / BMP / TIFF) take a
/// dedicated image branch — output keeps the same codec as the input
/// so a JPEG → desaturated JPEG, a PNG → desaturated PNG, etc.
///
/// Suffix is `_gray` so the output lands next to the source without
/// overwriting it: `<stem>_gray.<ext>`.
pub fn derive_grayscale_preset(ffmpeg: &Path, input: &Path) -> Preset {
    use crate::presets::{Dither, Format, ImageCodec};

    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_ascii_lowercase();

    // Image branch: greyscale a still image. We mirror the input's
    // codec so the user gets back the same file type they handed in
    // (a JPEG → JPEG, a PNG → PNG). For obscure formats we don't have
    // a native ImageCodec for (BMP, TIFF), fall back to PNG so we at
    // least produce a lossless output.
    if is_image_path(input) {
        let codec = match ext.as_str() {
            "jpg" | "jpeg" => ImageCodec::Jpeg,
            "webp" => ImageCodec::Webp,
            "avif" => ImageCodec::Avif,
            // png, bmp, tif, tiff — anything else lands here.
            _ => ImageCodec::Png,
        };
        return Preset {
            id: "__grayscale__".into(),
            name: "Greyscale".into(),
            enabled: true,
            format: Format::Image,
            suffix: "_gray".into(),
            width: None,
            height: None,
            fps: None,
            crop: None,
            palette_colors: None,
            dither: None,
            bayer_scale: None,
            crf: None,
            preset_speed: None,
            video_bitrate: None,
            audio_bitrate: None,
            use_cuda: None,
            target_max_mb: None,
            image_codec: Some(codec.clone()),
            image_quality: Some(codec.default_quality()),
            // Preserve user's original metadata on greyscale — this is
            // a "transform an image" operation, not a "share-ready"
            // operation. Image presets the user creates explicitly
            // can opt into stripping; the Greyscale TOOL leaves it.
            strip_metadata: Some(false),
            grayscale: Some(true),
            timecode: None,
            guides: None,
            overlay: None,
            crop_rect: None,
            modify_flip_h: None,
            modify_flip_v: None,
            modify_reverse: None,
            modify_overwrite: None,
            modify_remove_audio: None,
            modify_rotate: None,
            modify_trim_start_sec: None,
            modify_trim_end_sec: None,
            watermark: None,
            icon: None,
            order: 0,
        };
    }

    let probe = probe_video(ffmpeg, input);
    let format = if ext == "gif" { Format::Gif } else { Format::Mp4 };

    Preset {
        id: "__grayscale__".into(),
        name: "Greyscale".into(),
        enabled: true,
        format,
        suffix: "_gray".into(),
        width: probe.width,
        height: probe.height,
        fps: probe.fps,
        crop: None,
        palette_colors: Some(128),
        dither: Some(Dither::Bayer),
        bayer_scale: Some(3),
        crf: Some(23),
        preset_speed: Some("medium".into()),
        video_bitrate: None,
        audio_bitrate: Some("128k".into()),
        use_cuda: Some(false),
        target_max_mb: None,
        image_codec: None,
        image_quality: None,
        strip_metadata: None,
        grayscale: Some(true),
        timecode: None,
        guides: None,
        overlay: None,
        crop_rect: None,
        modify_flip_h: None,
        modify_flip_v: None,
        modify_reverse: None,
        modify_overwrite: None,
        modify_remove_audio: None,
        modify_rotate: None,
        modify_trim_start_sec: None,
        modify_trim_end_sec: None,
        watermark: None,
        icon: None,
        order: 0,
    }
}

/// Build an ad-hoc [`Preset`] for the Overlay tool. Dims are left None
/// so no scale filter runs — the overlay filters are
/// layered onto the source at its native size. Suffix `_overlay`.
///
/// Image inputs go through a dedicated image branch with codec
/// matched to the input (JPEG → JPEG, PNG → PNG, etc.) so overlay
/// burns into a still image of the same type rather than an
/// unexpected video clip.
pub fn derive_overlay_preset(ffmpeg: &Path, input: &Path, cfg: OverlayConfig) -> Preset {
    use crate::presets::{Dither, Format, ImageCodec, WatermarkSpec};

    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_ascii_lowercase();

    // Build the per-encode WatermarkSpec once and reuse for both
    // branches. Empty when the user hasn't enabled the toggle, hasn't
    // picked a path, or the picked file doesn't exist on disk —
    // failing silently here would be the wrong call (the user
    // explicitly asked for a watermark), so we'd rather not produce
    // a Spec than produce one pointing at nothing. The encode
    // dispatcher logs a clear error if a Spec is present and the
    // path goes missing between probe + invoke.
    let watermark = if cfg.watermark_enabled && !cfg.watermark_path.trim().is_empty() {
        let path = cfg.watermark_path.trim();
        if std::path::Path::new(path).is_file() {
            let (w, h) = probe_dimensions(ffmpeg, input).unwrap_or((1920, 1080));
            Some(WatermarkSpec {
                path: path.to_string(),
                opacity: cfg.watermark_opacity.clamp(0.0, 1.0),
                clip_w: w,
                clip_h: h,
            })
        } else {
            None
        }
    } else {
        None
    };

    if is_image_path(input) {
        let codec = match ext.as_str() {
            "jpg" | "jpeg" => ImageCodec::Jpeg,
            "webp" => ImageCodec::Webp,
            "avif" => ImageCodec::Avif,
            _ => ImageCodec::Png,
        };
        return Preset {
            id: "__overlay__".into(),
            name: "Overlay".into(),
            enabled: true,
            format: Format::Image,
            suffix: "_overlay".into(),
            width: None,
            height: None,
            fps: None,
            crop: None,
            palette_colors: None,
            dither: None,
            bayer_scale: None,
            crf: None,
            preset_speed: None,
            video_bitrate: None,
            audio_bitrate: None,
            use_cuda: None,
            target_max_mb: None,
            image_codec: Some(codec.clone()),
            image_quality: Some(codec.default_quality()),
            strip_metadata: Some(false),
            grayscale: None,
            timecode: None,
            guides: None,
            overlay: Some(cfg),
            crop_rect: None,
            modify_flip_h: None,
            modify_flip_v: None,
            modify_reverse: None,
            modify_overwrite: None,
            modify_remove_audio: None,
            modify_rotate: None,
            modify_trim_start_sec: None,
            modify_trim_end_sec: None,
            watermark: watermark.clone(),
            icon: None,
            order: 0,
        };
    }

    let format = if ext == "gif" { Format::Gif } else { Format::Mp4 };
    let probe = probe_video(ffmpeg, input);

    Preset {
        id: "__overlay__".into(),
        name: "Overlay".into(),
        enabled: true,
        format,
        suffix: "_overlay".into(),
        width: None,
        height: None,
        fps: probe.fps,
        crop: None,
        palette_colors: Some(128),
        dither: Some(Dither::Bayer),
        bayer_scale: Some(3),
        crf: Some(20),
        preset_speed: Some("medium".into()),
        video_bitrate: None,
        audio_bitrate: Some("128k".into()),
        use_cuda: Some(false),
        target_max_mb: None,
        image_codec: None,
        image_quality: None,
        strip_metadata: None,
        grayscale: None,
        timecode: None,
        guides: None,
        overlay: Some(cfg),
        crop_rect: None,
        modify_flip_h: None,
        modify_flip_v: None,
        modify_reverse: None,
        modify_overwrite: None,
        modify_remove_audio: None,
        modify_rotate: None,
        modify_trim_start_sec: None,
        modify_trim_end_sec: None,
        watermark: watermark.clone(),
        icon: None,
        order: 0,
    }
}

/// Total frame count for the first video stream of `input`. Used by the
/// Trim tool to translate user-entered "strip N from end" into an
/// absolute end_frame for the `trim` filter (filter wants an absolute
/// upper bound, not a relative one).
///
/// First tries `nb_frames` from the metadata — that's instant and works
/// for most MP4s. Falls back to `-count_packets nb_read_packets`, which
/// decodes far enough to count, and is what makes this work reliably on
/// GIFs and on MP4s whose `nb_frames` is missing or wrong (variable
/// frame rate, certain Apple-encoded clips). Returns `None` if both
/// attempts fail — caller should treat the trim as a no-op or error
/// rather than silently producing a zero-length file.
pub fn probe_total_frames(ffmpeg: &Path, input: &Path) -> Option<u64> {
    let probe = ffmpeg.with_file_name("ffprobe.exe");
    if !probe.exists() {
        return None;
    }

    // Fast path: `nb_frames` from the stream header. Reliable on most
    // CFR MP4s; comes back as "N/A" on GIFs and VFR clips.
    let mut cmd = Command::new(&probe);
    cmd.args([
        "-v", "error",
        "-select_streams", "v:0",
        "-show_entries", "stream=nb_frames",
        "-of", "default=nw=1:nk=1",
    ])
    .arg(input)
    .stdin(Stdio::null())
    .stderr(Stdio::null());
    hide_console(&mut cmd);
    if let Ok(out) = cmd.output() {
        let s = String::from_utf8_lossy(&out.stdout);
        let trimmed = s.trim();
        if trimmed != "N/A" && !trimmed.is_empty() {
            if let Ok(n) = trimmed.parse::<u64>() {
                if n > 0 {
                    return Some(n);
                }
            }
        }
    }

    // Fallback: count packets. Slower (decodes/demuxes the whole stream)
    // but works on GIFs and on MP4s missing `nb_frames`.
    let mut cmd = Command::new(&probe);
    cmd.args([
        "-v", "error",
        "-count_packets",
        "-select_streams", "v:0",
        "-show_entries", "stream=nb_read_packets",
        "-of", "default=nw=1:nk=1",
    ])
    .arg(input)
    .stdin(Stdio::null())
    .stderr(Stdio::null());
    hide_console(&mut cmd);
    let out = cmd.output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let n: u64 = s.trim().parse().ok()?;
    if n > 0 {
        Some(n)
    } else {
        None
    }
}

/// Compute the half-open kept frame intervals `[start, end)` for one
/// input given user-entered start/end strip counts and an optional
/// middle-range cut. Returns an empty `Vec` when the requested settings
/// would leave nothing.
///
/// Semantics:
///   * `start_frames` / `end_frames` shrink the outer interval from
///     `[0, total_frames)` down to `[start_frames, total_frames-end_frames)`.
///   * `remove_range = Some((rm_from, rm_to))` is INCLUSIVE on both
///     ends — `rm_from=50, rm_to=80` removes 31 frames (50…80). We
///     translate to half-open `[rm_from, rm_to+1)` internally to make
///     the interval algebra cleaner.
///   * The cut is clipped to the outer interval, so passing a range
///     entirely outside the kept region is a no-op (one interval out)
///     and a partially-overlapping range trims one end of the result
///     instead of producing a phantom hole.
fn compute_kept_intervals(
    total_frames: u64,
    start_frames: u64,
    end_frames: u64,
    remove_range: Option<(u64, u64)>,
) -> Vec<(u64, u64)> {
    if start_frames + end_frames >= total_frames {
        return Vec::new();
    }
    let outer_start = start_frames;
    let outer_end = total_frames - end_frames; // exclusive
    let Some((rm_from, rm_to)) = remove_range else {
        return vec![(outer_start, outer_end)];
    };
    if rm_to < rm_from {
        // User supplied an inverted range — treat as no cut rather than
        // erroring, since the dialog can't always intercept it (paste,
        // deferred validation).
        return vec![(outer_start, outer_end)];
    }
    let rm_lo = rm_from.max(outer_start);
    let rm_hi_excl = (rm_to + 1).min(outer_end);
    if rm_lo >= outer_end || rm_hi_excl <= outer_start || rm_lo >= rm_hi_excl {
        return vec![(outer_start, outer_end)];
    }
    let mut out = Vec::new();
    if outer_start < rm_lo {
        out.push((outer_start, rm_lo));
    }
    if rm_hi_excl < outer_end {
        out.push((rm_hi_excl, outer_end));
    }
    out
}

/// Build a video filter chain that keeps only frames inside `intervals`
/// and re-times them to start at PTS=0. For one interval we use
/// `trim`+`setpts=PTS-STARTPTS` (low overhead, the standard idiom). For
/// two or more we use `select` with an OR'd list of `between(n,A,B-1)`
/// clauses, plus `setpts=N/FRAME_RATE/TB` to renumber the surviving
/// frame timestamps from scratch (without this, the dropped-frame gaps
/// stay in the timeline and downstream filters see jumps).
///
/// Comma-as-arg-separator inside ffmpeg filter expressions has to be
/// escaped as `\,` — otherwise `between(n,5,10)` parses as three
/// arguments to `select`. The escape in the `format!` template is
/// `\\,`.
fn build_video_chop_filter(intervals: &[(u64, u64)]) -> String {
    if intervals.len() == 1 {
        let (a, b) = intervals[0];
        return format!("trim=start_frame={a}:end_frame={b},setpts=PTS-STARTPTS");
    }
    let exprs: Vec<String> = intervals
        .iter()
        .map(|(a, b)| format!("between(n\\,{}\\,{})", a, b - 1))
        .collect();
    format!("select='{}',setpts=N/FRAME_RATE/TB", exprs.join("+"))
}

/// Audio counterpart of [`build_video_chop_filter`]. Frame indices are
/// translated to seconds via `frames / fps` so cuts line up with the
/// video at the boundary frames. The `aselect`/`between(t,…)` form
/// works on container timestamps; `asetpts=N/SR/TB` rewrites them to
/// the kept span's local time.
fn build_audio_chop_filter(intervals: &[(u64, u64)], fps: u32) -> String {
    if intervals.len() == 1 {
        let (a, b) = intervals[0];
        let start_s = a as f64 / fps as f64;
        let end_s = b as f64 / fps as f64;
        return format!("atrim=start={start_s:.6}:end={end_s:.6},asetpts=PTS-STARTPTS");
    }
    let exprs: Vec<String> = intervals
        .iter()
        .map(|(a, b)| {
            let start_s = *a as f64 / fps as f64;
            let end_s = *b as f64 / fps as f64;
            format!("between(t\\,{start_s:.6}\\,{end_s:.6})")
        })
        .collect();
    format!("aselect='{}',asetpts=N/SR/TB", exprs.join("+"))
}

/// Frame-accurate trim: for each input, strip `start_frames` from the
/// front and `end_frames` from the back, write the result alongside the
/// source as `<stem>_trimmed.<ext>`. Per-file independent — every input
/// receives the same pair of values applied to its own timeline, so a
/// 3-file selection produces 3 outputs.
///
/// `remove_range`, when `Some((rm_from, rm_to))`, also excises the
/// frame range `[rm_from, rm_to]` (inclusive both ends) from the
/// middle. Combinable with `start_frames`/`end_frames` — e.g. strip 5
/// from each end AND cut frames 50-80 in one pass produces a single
/// output joining the two surviving spans.
///
/// Internally each input collapses to a list of half-open kept
/// intervals `[(start, end), ...]`. The single-interval case (no
/// middle cut) uses ffmpeg's `trim`/`atrim` filters — well-trodden,
/// minimum filter overhead. Two-interval cases (middle cut splits the
/// keep region) switch to `select`/`aselect` which take an arbitrary
/// boolean expression over frame number / timestamp, so multiple
/// non-contiguous spans concatenate naturally.
///
/// Stream-copy isn't an option here: trimming on arbitrary frame
/// boundaries crosses GOPs, so we re-encode at a near-lossless
/// baseline (CRF 17 / preset=slow / 256k AAC for MP4, 255-color
/// sierra2_4a-dithered palette for GIF) — Trim should feel seamless,
/// not size-optimized. Audio, when present, is trimmed in seconds
/// derived from `frames / fps` so video and audio stay in sync at
/// frame boundaries.
pub fn encode_trim_files(
    ffmpeg: &Path,
    files: &[PathBuf],
    start_frames: u32,
    end_frames: u32,
    remove_range: Option<(u32, u32)>,
    settings: &Settings,
    mut on_progress: impl FnMut(ProgressEvent),
) -> Result<()> {
    if files.is_empty() {
        bail!("Trim needs at least one file");
    }
    // Trim is intrinsically a video operation — there are no frames
    // to trim from a still image. Refuse with a clear message rather
    // than letting ffmpeg produce a 0-frame file.
    if files.iter().all(|p| is_image_path(p)) {
        bail!(
            "Trim only works on videos and animated GIFs. Still images \
             have no frames to remove. Use the Custom dialog or an \
             image preset to re-encode them."
        );
    }
    let total = files.len();
    let verbosity = settings.verbosity.clone().unwrap_or_else(|| "warning".into());

    for (idx, input) in files.iter().enumerate() {
        let file_index = idx + 1;
        let input_display = input.display().to_string();
        let ext = input
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp4")
            .to_ascii_lowercase();
        let is_gif = ext == "gif";

        let total_frames = probe_total_frames(ffmpeg, input);
        let probe = probe_video(ffmpeg, input);
        let fps = probe.fps.unwrap_or(30).max(1);

        let Some(total_frames) = total_frames else {
            on_progress(ProgressEvent {
                file_index,
                total_files: total,
                input: input_display.clone(),
                stage: "error".into(),
                percent: None,
                message: Some("Could not read frame count from this file.".into()),
            });
            continue;
        };
        // Compute the half-open kept intervals [start, end) (exclusive
        // upper bound, matching ffmpeg's `trim` filter semantics).
        let intervals = compute_kept_intervals(
            total_frames,
            start_frames as u64,
            end_frames as u64,
            remove_range.map(|(a, b)| (a as u64, b as u64)),
        );
        if intervals.is_empty() {
            on_progress(ProgressEvent {
                file_index,
                total_files: total,
                input: input_display.clone(),
                stage: "error".into(),
                percent: None,
                message: Some(format!(
                    "Trim would leave nothing — file has {total_frames} frames, requested settings remove all of them.",
                )),
            });
            continue;
        }
        let kept_frames: u64 = intervals.iter().map(|(a, b)| b - a).sum();
        let kept_duration_s = kept_frames as f64 / fps as f64;
        let is_multi = intervals.len() > 1;

        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
            .to_string();
        let dir = input.parent().unwrap_or(Path::new(".")).to_path_buf();
        let base = dir.join(format!("{stem}_trimmed.{ext}"));
        let out = unique_output_path(&base);

        let has_audio = !is_gif && has_audio_stream(ffmpeg, input);

        if is_gif {
            // GIF: two-pass with palette. Trim before palettegen so the
            // palette is built from the kept frames only — otherwise
            // colors that only existed in trimmed-away frames could
            // win a slot they're never going to use.
            on_progress(ProgressEvent {
                file_index,
                total_files: total,
                input: input_display.clone(),
                stage: "palette".into(),
                percent: None,
                message: Some("Generating palette".into()),
            });

            let palette_tmp = {
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                paths::tmp_dir()
                    .unwrap_or_else(|_| std::env::temp_dir())
                    .join(format!(
                        "{stem}.{}.{nonce}.trim.palette.png",
                        std::process::id()
                    ))
            };
            struct PaletteGuard(PathBuf);
            impl Drop for PaletteGuard {
                fn drop(&mut self) {
                    let _ = std::fs::remove_file(&self.0);
                }
            }
            let _palette_guard = PaletteGuard(palette_tmp.clone());

            // Single-interval (no middle cut) → use trim, which is
            // simpler and well-tested. Multi-interval → use select with
            // an OR'd list of `between(n, A, B-1)` clauses, then re-time
            // the surviving frames with setpts. Both end with a clean
            // `[0:v]<filter>` chain that downstream filters consume.
            let video_chop = build_video_chop_filter(&intervals);
            // Trim is meant to feel lossless — bump GIF quality to the
            // top of the palette (the maximum a GIF can carry; the
            // remaining 256th slot is reserved for transparency).
            // `stats_mode=full` builds the palette from every kept
            // frame instead of representative ones, which matters for
            // animations whose colors shift over time. The size cost
            // is real but Trim isn't the place to optimize size — the
            // quality presets are.
            let pal_filter = format!("[0:v]{video_chop},palettegen=max_colors=255:stats_mode=full");

            let mut pal_cmd = Command::new(ffmpeg);
            pal_cmd.args(["-v", &verbosity, "-y"]);
            pal_cmd.arg("-i").arg(input);
            pal_cmd
                .args(["-filter_complex", &pal_filter])
                .arg(&palette_tmp)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            hide_console(&mut pal_cmd);
            let status = pal_cmd
                .status()
                .context("spawning ffmpeg for trim palette")?;
            if !status.success() {
                bail!("trim palette pass failed");
            }

            on_progress(ProgressEvent {
                file_index,
                total_files: total,
                input: input_display.clone(),
                stage: "encode".into(),
                percent: None,
                message: Some(format!(
                    "Encoding GIF — {kept_frames} frames (high quality){}",
                    if is_multi { ", middle cut" } else { "" }
                )),
            });

            // sierra2_4a is the highest-quality dither GIF supports —
            // smoother gradients and less visible pattern noise than
            // bayer at the cost of a slightly larger file. Trim wants
            // quality first, so use it here even though our other GIF
            // tools default to bayer for size.
            let p2 = format!(
                "[0:v]{video_chop}[v];[v][1:v]paletteuse=dither=sierra2_4a"
            );
            let mut cmd = Command::new(ffmpeg);
            cmd.args(["-v", &verbosity, "-y", "-hide_banner"]);
            cmd.arg("-i").arg(input);
            cmd.arg("-i").arg(&palette_tmp);
            cmd.args(["-filter_complex", &p2])
                .args(["-progress", "pipe:1"])
                .arg(&out);
            run_with_progress_cleanup(
                cmd,
                Some(kept_duration_s),
                file_index,
                total,
                &input_display,
                "encode",
                &out,
                &mut on_progress,
            )?;
        } else {
            on_progress(ProgressEvent {
                file_index,
                total_files: total,
                input: input_display.clone(),
                stage: "encode".into(),
                percent: None,
                message: Some(format!(
                    "Trimming MP4 — keeping {kept_frames} of {total_frames} frames (visually lossless{}{})",
                    if has_audio { " + audio" } else { "" },
                    if is_multi { ", middle cut" } else { "" }
                )),
            });

            // Build filter graph. For a single kept interval we emit
            // `trim`/`atrim`; for multiple intervals we emit
            // `select`/`aselect` over the union of frame-number /
            // timestamp ranges. The audio side translates the same
            // frame boundaries to seconds (frames / fps) so video and
            // audio stay aligned at every cut.
            let video_chop = build_video_chop_filter(&intervals);
            let mut graph = format!("[0:v]{video_chop}[v]");
            if has_audio {
                let audio_chop = build_audio_chop_filter(&intervals, fps);
                graph.push_str(&format!(";[0:a]{audio_chop}[a]"));
            }

            let mut cmd = Command::new(ffmpeg);
            cmd.args(["-v", &verbosity, "-y", "-hide_banner"]);
            cmd.arg("-i").arg(input);
            cmd.args(["-filter_complex", &graph])
                .args(["-map", "[v]"]);
            if has_audio {
                cmd.args(["-map", "[a]"]);
            }
            // Trim is "chop the ends, keep everything else" — quality
            // should be transparent. CRF 17 is below x264's
            // visually-lossless threshold (~18) so re-encoding round-
            // trips without obvious quality loss; preset=slow gives
            // better compression efficiency at that quality. yuv420p
            // stays for player compatibility (yuv444p breaks Quick-
            // Time and most consumer players). Audio jumps to 256k AAC
            // — transparent for stereo content and the size delta is
            // tiny next to the video.
            cmd.args([
                "-c:v", "libx264",
                "-preset", "slow",
                "-crf", "17",
                "-pix_fmt", "yuv420p",
                "-movflags", "+faststart",
            ]);
            if has_audio {
                cmd.args(["-c:a", "aac", "-b:a", "256k"]);
            } else {
                cmd.arg("-an");
            }
            cmd.args(["-progress", "pipe:1"]).arg(&out);
            run_with_progress_cleanup(
                cmd,
                Some(kept_duration_s),
                file_index,
                total,
                &input_display,
                "encode",
                &out,
                &mut on_progress,
            )?;
        }

        on_progress(ProgressEvent {
            file_index,
            total_files: total,
            input: input_display,
            stage: "done".into(),
            percent: Some(1.0),
            message: Some(out.display().to_string()),
        });
    }

    Ok(())
}

/// Side-by-side Compare: stack N inputs horizontally into one output.
/// Each input is scaled to the first file's height and normalized to
/// its fps so hstack sees uniform streams. Output format matches the
/// first file's extension (mp4 or gif). Audio is dropped — A/B review
/// is a visual-only workflow.
///
/// When ALL inputs are still images, we hand off to the image-stack
/// branch which produces a single still output of matching format
/// (PNG → PNG, JPEG → JPEG, etc.). Mixed image+video inputs go through
/// the video path and any image is treated as a one-frame clip — odd
/// but well-defined and rarely hit in practice.
pub fn encode_compare_files(
    ffmpeg: &Path,
    files: &[PathBuf],
    settings: &Settings,
    mut on_progress: impl FnMut(ProgressEvent),
) -> Result<PathBuf> {
    if files.len() < 2 {
        bail!("Compare needs at least two files");
    }
    let first = &files[0];
    let ext = first
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_ascii_lowercase();
    let is_gif = ext == "gif";

    // All-image branch: stack N stills into one still. Skips the
    // fps-normalization, duration tracking, and palette logic — those
    // are all video concerns. Handles its own output naming + emits.
    if files.iter().all(|p| is_image_path(p)) {
        return encode_compare_images(ffmpeg, files, settings, on_progress);
    }

    let probe = probe_video(ffmpeg, first);
    let height = probe.height.unwrap_or(720).max(120);
    let fps = probe.fps.unwrap_or(30);
    let n = files.len();

    // Normalize each stream then hstack. scale=-2:H keeps aspect; fps
    // resamples to a shared rate; setsar=1 avoids "SAR mismatch" errors
    // when inputs have different pixel aspect ratios.
    let mut norm = String::new();
    for i in 0..n {
        if i > 0 {
            norm.push(';');
        }
        norm.push_str(&format!(
            "[{i}:v]scale=-2:{height}:flags=lanczos,fps={fps},setsar=1[v{i}]"
        ));
    }
    let mut stacked = String::new();
    for i in 0..n {
        stacked.push_str(&format!("[v{i}]"));
    }
    stacked.push_str(&format!("hstack=inputs={n}"));

    let stem = first
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let base = first
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf()
        .join(format!("{stem}_compare.{ext}"));
    let out = unique_output_path(&base);

    // Duration for the progress bar = shortest input (hstack caps there).
    let duration = files
        .iter()
        .filter_map(|p| probe_duration(ffmpeg, p))
        .fold(f64::INFINITY, f64::min);
    let duration_opt = if duration.is_finite() { Some(duration) } else { None };

    let verbosity = settings.verbosity.clone().unwrap_or_else(|| "warning".into());
    let input_display = format!("compare: {stem}");
    let total_files = 1usize;
    let file_index = 1usize;

    if is_gif {
        on_progress(ProgressEvent {
            file_index,
            total_files,
            input: input_display.clone(),
            stage: "palette".into(),
            percent: None,
            message: Some("Generating palette".into()),
        });

        // Pass 1: palette from the hstacked stream.
        let palette_tmp = {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            paths::tmp_dir()
                .unwrap_or_else(|_| std::env::temp_dir())
                .join(format!("{stem}.{}.{nonce}.compare.palette.png", std::process::id()))
        };
        struct PaletteGuard(PathBuf);
        impl Drop for PaletteGuard {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _palette_guard = PaletteGuard(palette_tmp.clone());

        let filter_p1 = format!("{norm};{stacked},palettegen=max_colors=128:stats_mode=full");
        let mut pal_cmd = Command::new(ffmpeg);
        pal_cmd.args(["-v", &verbosity, "-y"]);
        for f in files {
            pal_cmd.arg("-i").arg(f);
        }
        pal_cmd
            .args(["-filter_complex", &filter_p1])
            .arg(&palette_tmp)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_console(&mut pal_cmd);
        let status = pal_cmd
            .status()
            .context("spawning ffmpeg for compare palette")?;
        if !status.success() {
            bail!("compare palette pass failed");
        }

        // Pass 2: hstack + paletteuse. The palette is the last -i input.
        let palette_idx = n;
        let filter_p2 = format!(
            "{norm};{stacked}[vh];[vh][{palette_idx}:v]paletteuse=dither=bayer:bayer_scale=3",
            norm = norm,
            stacked = stacked,
            palette_idx = palette_idx,
        );

        on_progress(ProgressEvent {
            file_index,
            total_files,
            input: input_display.clone(),
            stage: "encode".into(),
            percent: None,
            message: Some("Encoding GIF".into()),
        });

        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-v", &verbosity, "-y", "-hide_banner"]);
        for f in files {
            cmd.arg("-i").arg(f);
        }
        cmd.arg("-i").arg(&palette_tmp);
        cmd.args(["-filter_complex", &filter_p2])
            .args(["-progress", "pipe:1"])
            .args(["-shortest"])
            .arg(&out);
        run_with_progress_cleanup(
            cmd,
            duration_opt,
            file_index,
            total_files,
            &input_display,
            "encode",
            &out,
            &mut on_progress,
        )?;
    } else {
        on_progress(ProgressEvent {
            file_index,
            total_files,
            input: input_display.clone(),
            stage: "encode".into(),
            percent: None,
            message: Some("Encoding MP4 compare".into()),
        });

        let filter = format!("{norm};{stacked}[vh]");
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-v", &verbosity, "-y", "-hide_banner"]);
        for f in files {
            cmd.arg("-i").arg(f);
        }
        cmd.args(["-filter_complex", &filter])
            .args(["-map", "[vh]"])
            .args(["-c:v", "libx264", "-preset", "medium", "-crf", "20"])
            .args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .args(["-an"])
            .args(["-shortest"])
            .args(["-progress", "pipe:1"])
            .arg(&out);
        run_with_progress_cleanup(
            cmd,
            duration_opt,
            file_index,
            total_files,
            &input_display,
            "encode",
            &out,
            &mut on_progress,
        )?;
    }

    on_progress(ProgressEvent {
        file_index,
        total_files,
        input: input_display,
        stage: "done".into(),
        percent: Some(1.0),
        message: Some(out.display().to_string()),
    });
    Ok(out)
}

/// Two layout modes for `encode_compare_grid_files`.
///
///   * **Grid** — uniform cells inheriting the first clip's aspect.
///     Each input is letterboxed / pillarboxed inside its cell.
///     Empty trailing slots filled with black. Looks like a regular
///     contact sheet.
///   * **Mosaic** — masonry packing. Column width is fixed, but each
///     clip retains its native aspect at that width — so a portrait
///     clip becomes a tall cell, a landscape clip becomes a short
///     cell. Clips greedy-fill columns shortest-first, minimising
///     wasted space. Pinterest-style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridLayout {
    Grid,
    Mosaic,
}

/// One cell on the output canvas — either a real input clip or a
/// solid-black filler (for trailing empty Grid slots / short-column
/// bottoms in Mosaic). Built by `compute_placements` and consumed by
/// the filter-graph builder + `xstack` layout string.
struct Placement {
    /// `Some(i)` = use input `i`'s video. `None` = generate a black
    /// `color=` source of the placement's size.
    input_idx: Option<usize>,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// Stack N≥2 clips (videos and/or images) into a `cols`-wide grid.
///
/// Output type depends on inputs:
///   * Any video present → MP4 output. Static images among the inputs
///     are looped to match the shortest video's duration via the
///     `-loop 1 -framerate <fps> -t <dur>` input prefix.
///   * All-images selection → single still output, codec matched to
///     the first input (PNG / JPEG / WebP / AVIF / TIFF).
///
/// Order is by filename ascending — sorted in the Tauri command before
/// it gets here, but we sort again here as defence-in-depth so direct
/// callers (CLI, tests) get the same predictable ordering.
///
/// Output filename: `<first-stem>_grid.<ext>`, deduped against existing
/// files via `unique_output_path`. `ext` is `mp4` for the video path,
/// otherwise the canonicalised first-input extension.
pub fn encode_compare_grid_files(
    ffmpeg: &Path,
    files: &[PathBuf],
    cols: u32,
    layout: GridLayout,
    settings: &Settings,
    mut on_progress: impl FnMut(ProgressEvent),
) -> Result<PathBuf> {
    use crate::presets::ImageCodec;

    if files.len() < 2 {
        bail!("Compare grid needs at least two files");
    }

    // Defence-in-depth sort: encode_compare_grid in commands.rs already
    // sorts, but CLI / test entrypoints might not.
    let mut files: Vec<PathBuf> = files.to_vec();
    files.sort_by_key(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });

    let cols = cols.max(1) as usize;
    let n = files.len();

    // Probe every input up front. Real videos go through probe_video
    // (dims + fps). Stills go through probe_dimensions (dims only).
    // We need each clip's intrinsic aspect for the Mosaic packing, and
    // the all-images vs mixed detection for the codec-pick later.
    struct InputInfo {
        width: u32,
        height: u32,
        is_image: bool,
    }
    let inputs: Vec<InputInfo> = files
        .iter()
        .map(|p| {
            if is_image_path(p) {
                let (w, h) = probe_dimensions(ffmpeg, p).unwrap_or((1920, 1080));
                InputInfo {
                    width: w.max(2),
                    height: h.max(2),
                    is_image: true,
                }
            } else {
                let pr = probe_video(ffmpeg, p);
                InputInfo {
                    width: pr.width.unwrap_or(1280).max(2),
                    height: pr.height.unwrap_or(720).max(2),
                    is_image: false,
                }
            }
        })
        .collect();

    let all_images = inputs.iter().all(|i| i.is_image);
    let first = &files[0];
    let first_w = inputs[0].width;
    let first_h = inputs[0].height;

    // Framerate comes from the first VIDEO clip (skipping over any
    // leading image inputs). Default to 30fps if all inputs are images.
    let fps = files
        .iter()
        .enumerate()
        .find(|(i, _)| !inputs[*i].is_image)
        .and_then(|(_, p)| probe_video(ffmpeg, p).fps)
        .unwrap_or(30);

    // Shortest VIDEO duration — used to clamp image-loop duration in
    // mixed mode AND to bake into the `color=` filler's `d=` so the
    // grid terminates cleanly. None when all-images (we're producing
    // a still then; no time dimension to worry about).
    let video_duration: Option<f64> = if all_images {
        None
    } else {
        let d = files
            .iter()
            .enumerate()
            .filter(|(i, _)| !inputs[*i].is_image)
            .filter_map(|(_, p)| probe_duration(ffmpeg, p))
            .fold(f64::INFINITY, f64::min);
        if d.is_finite() {
            Some(d)
        } else {
            None
        }
    };
    // Fallback so image-loop `-t` always has a finite value (corrupt
    // metadata, exotic format that probe_duration can't parse). 5s is
    // arbitrary but reasonable for "look at these images briefly".
    let image_loop_duration = video_duration.unwrap_or(5.0);

    // Compute placements for the chosen layout. Both modes return a
    // Vec<Placement> the rest of the function consumes uniformly.
    let placements: Vec<Placement> = match layout {
        GridLayout::Grid => {
            // Uniform cells inheriting first clip's aspect.
            let cell_w = (first_w / cols as u32).max(2) & !1;
            let cell_h = ((cell_w as u64 * first_h as u64 / first_w as u64) as u32).max(2) & !1;
            let rows = (n + cols - 1) / cols;
            let total_slots = cols * rows;
            (0..total_slots)
                .map(|i| {
                    let col = (i % cols) as u32;
                    let row = (i / cols) as u32;
                    Placement {
                        input_idx: if i < n { Some(i) } else { None },
                        x: col * cell_w,
                        y: row * cell_h,
                        w: cell_w,
                        h: cell_h,
                    }
                })
                .collect()
        }
        GridLayout::Mosaic => {
            // Masonry: fixed column width, per-clip scaled height
            // (aspect-preserving). Greedy assignment of each clip to
            // the column with smallest current cumulative height —
            // standard masonry-pack heuristic. Trailing column-bottoms
            // get black fillers so the final canvas is rectangular.
            let col_w = (first_w / cols as u32).max(2) & !1;
            let mut col_y = vec![0u32; cols];
            let mut placements: Vec<Placement> = Vec::with_capacity(n + cols);
            for (i, input) in inputs.iter().enumerate() {
                let scaled_h = ((col_w as u64 * input.height as u64 / input.width as u64)
                    as u32)
                    .max(2)
                    & !1;
                let (min_col, &min_h) = col_y
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, &h)| h)
                    .unwrap();
                placements.push(Placement {
                    input_idx: Some(i),
                    x: min_col as u32 * col_w,
                    y: min_h,
                    w: col_w,
                    h: scaled_h,
                });
                col_y[min_col] += scaled_h;
            }
            let max_h = *col_y.iter().max().unwrap_or(&0);
            for col in 0..cols {
                if col_y[col] < max_h {
                    placements.push(Placement {
                        input_idx: None,
                        x: col as u32 * col_w,
                        y: col_y[col],
                        w: col_w,
                        h: max_h - col_y[col],
                    });
                }
            }
            placements
        }
    };

    let canvas_w = placements.iter().map(|p| p.x + p.w).max().unwrap_or(2);
    let canvas_h = placements.iter().map(|p| p.y + p.h).max().unwrap_or(2);

    // Build the filter graph. For each Placement:
    //   * input_idx=Some(i): scale [i:v] to (w,h). Grid mode adds a
    //     pad= so the source's aspect is preserved within the cell;
    //     Mosaic uses the placement dimensions directly (per-clip
    //     aspect was already baked in by the packer).
    //   * input_idx=None: synthesize a `color=` black source at (w,h).
    //
    // fps= is included in the chain only for video output. All-images
    // mode produces a single still — keeping fps in the chain there
    // would force unnecessary frame duplication during the encode.
    let mut filter_parts: Vec<String> = Vec::new();
    let fps_suffix = if all_images {
        String::new()
    } else {
        format!(",fps={fps}")
    };
    let filler_color_suffix = if all_images {
        String::new()
    } else {
        // d= clamps the filler to the shortest real-input duration
        // (avoiding the infinite-source hang). r= matches the shared
        // framerate. omitted entirely in all-images mode since the
        // still output is one frame regardless.
        let dur = match video_duration {
            Some(d) if d > 0.0 => format!(":d={:.3}", d),
            _ => String::new(),
        };
        format!(":r={fps}{dur}")
    };
    for (pi, p) in placements.iter().enumerate() {
        match p.input_idx {
            Some(i) => {
                let inner = match layout {
                    GridLayout::Grid => format!(
                        "scale={w}:{h}:force_original_aspect_ratio=decrease,\
                         pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black",
                        w = p.w,
                        h = p.h,
                    ),
                    GridLayout::Mosaic => {
                        // Per-placement dimensions already match the
                        // clip's aspect; no padding needed.
                        format!("scale={w}:{h}", w = p.w, h = p.h)
                    }
                };
                filter_parts.push(format!(
                    "[{i}:v]{inner},setsar=1{fps_suffix}[p{pi}]"
                ));
            }
            None => {
                filter_parts.push(format!(
                    "color=c=black:s={w}x{h}{filler_color_suffix},setsar=1[p{pi}]",
                    w = p.w,
                    h = p.h,
                ));
            }
        }
    }

    let layout_str = placements
        .iter()
        .map(|p| format!("{}_{}", p.x, p.y))
        .collect::<Vec<_>>()
        .join("|");
    let stacked = (0..placements.len())
        .map(|i| format!("[p{i}]"))
        .collect::<String>();
    // shortest=1 on xstack is a second guard against any input still
    // being infinite somehow. With duration-clamped fillers this is
    // usually redundant, but cheap to keep.
    let filter = format!(
        "{};{}xstack=inputs={}:layout={}:shortest=1[vh]",
        filter_parts.join(";"),
        stacked,
        placements.len(),
        layout_str
    );

    // Output filename + extension. All-images grids keep the first
    // input's image format; mixed/video always emit MP4.
    let stem = first
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");
    let first_ext_raw = first
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_ascii_lowercase();
    let (out_ext, image_codec): (String, Option<ImageCodec>) = if all_images {
        let codec = match first_ext_raw.as_str() {
            "jpg" | "jpeg" => ImageCodec::Jpeg,
            "webp" => ImageCodec::Webp,
            "avif" => ImageCodec::Avif,
            _ => ImageCodec::Png,
        };
        // Normalise extension to the codec's canonical form so
        // "Pic.JPEG" → "Pic_grid.jpg" instead of "Pic_grid.JPEG".
        (codec.ext().to_string(), Some(codec))
    } else {
        ("mp4".to_string(), None)
    };
    let base = first
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf()
        .join(format!("{stem}_grid.{out_ext}"));
    let out = unique_output_path(&base);

    let verbosity = settings.verbosity.clone().unwrap_or_else(|| "warning".into());
    let input_display = format!("compare-grid: {stem}");
    let layout_name = match layout {
        GridLayout::Grid => "Grid",
        GridLayout::Mosaic => "Mosaic",
    };
    let mode_label = if all_images { "still" } else { "video" };

    on_progress(ProgressEvent {
        file_index: 1,
        total_files: 1,
        input: input_display.clone(),
        stage: "encode".into(),
        percent: None,
        message: Some(format!(
            "Encoding {layout_name} {mode_label} ({canvas_w}×{canvas_h})"
        )),
    });

    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-v", &verbosity, "-y", "-hide_banner"]);

    // Per-input args: image inputs in MIXED mode need -loop 1
    // -framerate -t to become finite video streams. In all-images mode
    // images are stills, no loop/duration needed. Real videos always
    // pass through with just -i.
    for (i, p) in files.iter().enumerate() {
        if inputs[i].is_image && !all_images {
            cmd.args([
                "-loop",
                "1",
                "-framerate",
                &fps.to_string(),
                "-t",
                &format!("{:.3}", image_loop_duration),
            ]);
        }
        cmd.arg("-i").arg(p);
    }

    cmd.args(["-filter_complex", &filter])
        .args(["-map", "[vh]"]);

    if let Some(codec) = image_codec {
        // Still output: cap to a single frame, swap in the right
        // image codec (PNG/JPEG/WebP/AVIF). No -shortest needed —
        // -frames:v 1 stops the encode after one frame regardless.
        cmd.args(["-frames:v", "1"]);
        append_image_codec_args(&mut cmd, &codec);
    } else {
        cmd.args(["-c:v", "libx264", "-preset", "medium", "-crf", "20"])
            .args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .args(["-an"])
            .args(["-shortest"]);
    }

    cmd.args(["-progress", "pipe:1"]).arg(&out);

    run_with_progress_cleanup(
        cmd,
        video_duration,
        1,
        1,
        &input_display,
        "encode",
        &out,
        &mut on_progress,
    )?;

    on_progress(ProgressEvent {
        file_index: 1,
        total_files: 1,
        input: input_display,
        stage: "done".into(),
        percent: Some(1.0),
        message: Some(out.display().to_string()),
    });
    Ok(out)
}

/// Per-codec args + extension for "encode this image with reasonable
/// defaults" — used by the image-only tools (Invert, MakeSquare,
/// Compare). Keeps the per-tool encode functions from each
/// reinventing the codec switch. Quality values match the encode_file
/// image branch's "high quality" baseline.
fn append_image_codec_args(cmd: &mut Command, codec: &crate::presets::ImageCodec) {
    use crate::presets::ImageCodec;
    match codec {
        ImageCodec::Png => {
            cmd.args(["-c:v", "png", "-compression_level", "6"]);
        }
        ImageCodec::Jpeg => {
            cmd.args(["-c:v", "mjpeg", "-q:v", "3", "-pix_fmt", "yuvj420p"]);
        }
        ImageCodec::Webp => {
            cmd.args(["-c:v", "libwebp", "-quality", "85", "-lossless", "0"]);
        }
        ImageCodec::Avif => {
            cmd.args(["-c:v", "libaom-av1", "-crf", "24", "-still-picture", "1"]);
        }
    }
}

/// Map an input path's extension to one of our supported `ImageCodec`
/// variants. BMP / TIFF (and anything else) fall back to PNG so we
/// always end up with a lossless-or-better output rather than failing.
fn image_codec_from_ext(path: &Path) -> crate::presets::ImageCodec {
    use crate::presets::ImageCodec;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => ImageCodec::Jpeg,
        "webp" => ImageCodec::Webp,
        "avif" => ImageCodec::Avif,
        _ => ImageCodec::Png,
    }
}

/// True if the codec's container can carry an alpha channel. JPEG and
/// the (rare) MJPEG variants can't. Used by MakeSquare to decide
/// whether to honor a "transparent" fill request or upgrade the
/// output container to PNG.
fn codec_supports_alpha(codec: &crate::presets::ImageCodec) -> bool {
    use crate::presets::ImageCodec;
    !matches!(codec, ImageCodec::Jpeg)
}

/// Probe the top-left pixel of `input`, returning it as `(r, g, b)`
/// each in 0..=255. Used by MakeSquare's `EdgeColor` fill mode to
/// pick a pad color that matches the image's actual edge.
///
/// Implementation: feed input to ffmpeg with a 1×1 crop at (0, 0),
/// rgb24 format, single frame, and have it write three raw bytes to
/// stdout. The bytes ARE the pixel, no decoding gymnastics needed.
fn probe_corner_color(ffmpeg: &Path, input: &Path) -> Option<(u8, u8, u8)> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-v", "error", "-y"])
        .arg("-i")
        .arg(input)
        .args(["-vf", "crop=1:1:0:0,format=rgb24"])
        .args(["-frames:v", "1", "-f", "rawvideo", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped());
    hide_console(&mut cmd);
    let out = cmd.output().ok()?;
    if out.stdout.len() >= 3 {
        Some((out.stdout[0], out.stdout[1], out.stdout[2]))
    } else {
        None
    }
}

/// Invert tool: per-image color-channel invert with optional binary
/// clamp. Refuses video inputs with a clear error rather than letting
/// `negate` produce an unexpected video clip.
///
/// Without `clamp`, the filter is just `negate` — RGB channels are
/// inverted (`out = 255 - in`), alpha is preserved as-is.
///
/// With `clamp`, we follow `negate` with a `geq` pass that thresholds
/// every channel (R, G, B, AND alpha) to either 0 or 255 at midpoint
/// 127. The result is a strict 1-bit-per-channel image — useful for
/// cleaning up alpha masks or layer-mask PNGs where compression
/// artifacts and anti-aliased edges have introduced grey "noise".
/// `geq` is per-pixel-evaluated and slow for huge inputs, but for
/// typical mask-sized images it's fine.
pub fn encode_invert_files(
    ffmpeg: &Path,
    files: &[PathBuf],
    clamp: bool,
    settings: &Settings,
    mut on_progress: impl FnMut(ProgressEvent),
) -> Result<()> {
    if files.is_empty() {
        bail!("Invert needs at least one file");
    }
    if !files.iter().all(|p| is_image_path(p)) {
        bail!(
            "Invert only works on still images (PNG / JPEG / WebP / \
             AVIF / BMP / TIFF). Video files have no single frame to \
             invert; for video, use a Greyscale preset or a custom \
             ffmpeg pipeline."
        );
    }

    let total = files.len();
    let verbosity = settings
        .verbosity
        .clone()
        .unwrap_or_else(|| "warning".into());

    // Filter graph: `negate` for the invert; with `clamp`, follow
    // with a per-channel threshold via `geq`. The geq expression
    // names (`r(X,Y)`, `g(X,Y)`, etc.) reference source pixel values
    // at the current output coords; ffmpeg keeps things in-place so
    // `negate,geq=...` reads the negated pixel, not the original.
    let filter = if clamp {
        "negate,geq=\
         r='if(gt(r(X\\,Y)\\,127)\\,255\\,0)':\
         g='if(gt(g(X\\,Y)\\,127)\\,255\\,0)':\
         b='if(gt(b(X\\,Y)\\,127)\\,255\\,0)':\
         a='if(gt(alpha(X\\,Y)\\,127)\\,255\\,0)'"
            .to_string()
    } else {
        "negate".to_string()
    };

    for (idx, input) in files.iter().enumerate() {
        let file_index = idx + 1;
        let input_display = input.display().to_string();
        let codec = image_codec_from_ext(input);
        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        let dir = input.parent().unwrap_or(Path::new(".")).to_path_buf();
        let base = dir.join(format!("{stem}_inverted.{}", codec.ext()));
        let out = unique_output_path(&base);

        on_progress(ProgressEvent {
            file_index,
            total_files: total,
            input: input_display.clone(),
            stage: "encode".into(),
            percent: None,
            message: Some(format!(
                "Inverting {}{}",
                codec.ext().to_ascii_uppercase(),
                if clamp { " (clamped to 0/255)" } else { "" }
            )),
        });

        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-v", &verbosity, "-y", "-hide_banner"])
            .arg("-i")
            .arg(input)
            .args(["-vf", &filter])
            .args(["-frames:v", "1"]);
        append_image_codec_args(&mut cmd, &codec);
        cmd.arg(&out);

        run_with_progress_cleanup(
            cmd,
            None,
            file_index,
            total,
            &input_display,
            "encode",
            &out,
            &mut on_progress,
        )?;

        on_progress(ProgressEvent {
            file_index,
            total_files: total,
            input: input_display,
            stage: "done".into(),
            percent: Some(1.0),
            message: Some(out.display().to_string()),
        });
    }

    Ok(())
}

/// Make-Square tool: per-image pad to a square whose side equals the
/// longer edge of the source. `fill_mode` decides what the new pixels
/// are filled with:
///
///   * `Transparent` → `black@0` pad. Output codec is forced to
///     something that supports alpha; if the input is JPEG, output
///     becomes PNG so the transparency actually survives.
///   * `EdgeColor` → sample the top-left pixel of the input via
///     `probe_corner_color`, pad with that. Output keeps the input's
///     codec. If the probe fails (rare — only happens if ffmpeg
///     can't decode the file at all), fall back to white so we still
///     produce a useful result rather than erroring.
pub fn encode_make_square_files(
    ffmpeg: &Path,
    files: &[PathBuf],
    fill_mode: crate::presets::MakeSquareFillMode,
    settings: &Settings,
    mut on_progress: impl FnMut(ProgressEvent),
) -> Result<()> {
    use crate::presets::{ImageCodec, MakeSquareFillMode};

    if files.is_empty() {
        bail!("Make Square needs at least one file");
    }
    if !files.iter().all(|p| is_image_path(p)) {
        bail!(
            "Make Square only works on still images (PNG / JPEG / WebP \
             / AVIF / BMP / TIFF). Video files don't have a single \
             aspect ratio to pad; for video, use a crop-aspect MP4 preset \
             instead."
        );
    }

    let total = files.len();
    let verbosity = settings
        .verbosity
        .clone()
        .unwrap_or_else(|| "warning".into());

    for (idx, input) in files.iter().enumerate() {
        let file_index = idx + 1;
        let input_display = input.display().to_string();

        // Probe the source so we can compute the longer edge and
        // build the `pad` filter. Without dimensions we can't square
        // anything, so a probe failure is fatal for that one file —
        // surface it as an error event and continue with the rest.
        let probe = probe_video(ffmpeg, input);
        let (Some(src_w), Some(src_h)) = (probe.width, probe.height) else {
            on_progress(ProgressEvent {
                file_index,
                total_files: total,
                input: input_display.clone(),
                stage: "error".into(),
                percent: None,
                message: Some(
                    "Could not read image dimensions; skipping.".into(),
                ),
            });
            continue;
        };
        let side = src_w.max(src_h);

        // Already square? Skip the encode pass — the output would be
        // bit-identical and the user clicking "Make Square" on a
        // square image probably means "make sure this stays square",
        // which is satisfied by a no-op + a "done" event.
        if src_w == src_h {
            on_progress(ProgressEvent {
                file_index,
                total_files: total,
                input: input_display.clone(),
                stage: "done".into(),
                percent: Some(1.0),
                message: Some(format!(
                    "{} is already {src_w}x{src_h} — nothing to do.",
                    input.file_name().and_then(|n| n.to_str()).unwrap_or("(file)")
                )),
            });
            continue;
        }

        // Resolve the output codec. For Transparent fill, we MUST end
        // up at a codec that carries alpha — JPEG inputs get bumped
        // to PNG, others keep their native format.
        let input_codec = image_codec_from_ext(input);
        let codec = match fill_mode {
            MakeSquareFillMode::Transparent if !codec_supports_alpha(&input_codec) => {
                ImageCodec::Png
            }
            _ => input_codec,
        };

        // Pick the pad color string. ffmpeg's pad accepts named
        // colors and `0xRRGGBB[@A]` hex literals; we go hex for
        // determinism. EdgeColor falls back to white if probing
        // fails — a visible-but-neutral background is better than
        // erroring out per-file.
        let pad_color = match fill_mode {
            MakeSquareFillMode::Transparent => "black@0".to_string(),
            MakeSquareFillMode::EdgeColor => probe_corner_color(ffmpeg, input)
                .map(|(r, g, b)| format!("0x{r:02X}{g:02X}{b:02X}"))
                .unwrap_or_else(|| "white".to_string()),
        };

        // For Transparent fill the source must arrive at the pad
        // filter as RGBA (otherwise alpha is dropped in YUV
        // intermediates). The `format=rgba` filter is harmless even
        // when the input already has alpha — ffmpeg recognises the
        // no-op.
        let filter = match fill_mode {
            MakeSquareFillMode::Transparent => format!(
                "format=rgba,pad={side}:{side}:({side}-iw)/2:({side}-ih)/2:color={pad_color}"
            ),
            MakeSquareFillMode::EdgeColor => format!(
                "pad={side}:{side}:({side}-iw)/2:({side}-ih)/2:color={pad_color}"
            ),
        };

        let stem = input
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        let dir = input.parent().unwrap_or(Path::new(".")).to_path_buf();
        let base = dir.join(format!("{stem}_square.{}", codec.ext()));
        let out = unique_output_path(&base);

        on_progress(ProgressEvent {
            file_index,
            total_files: total,
            input: input_display.clone(),
            stage: "encode".into(),
            percent: None,
            message: Some(format!(
                "Padding to {side}x{side} ({}) → {}",
                match fill_mode {
                    MakeSquareFillMode::Transparent => "transparent".to_string(),
                    MakeSquareFillMode::EdgeColor => format!("edge color {pad_color}"),
                },
                codec.ext().to_ascii_uppercase()
            )),
        });

        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-v", &verbosity, "-y", "-hide_banner"])
            .arg("-i")
            .arg(input)
            .args(["-vf", &filter])
            .args(["-frames:v", "1"]);
        append_image_codec_args(&mut cmd, &codec);
        cmd.arg(&out);

        run_with_progress_cleanup(
            cmd,
            None,
            file_index,
            total,
            &input_display,
            "encode",
            &out,
            &mut on_progress,
        )?;

        on_progress(ProgressEvent {
            file_index,
            total_files: total,
            input: input_display,
            stage: "done".into(),
            percent: Some(1.0),
            message: Some(out.display().to_string()),
        });
    }

    Ok(())
}

/// Probe just the (width, height) of the first video stream / image
/// stream of `input`. Used by the Crop dialog to do display↔source
/// pixel coordinate mapping. Returns `None` only when ffprobe
/// genuinely can't read the file — caller should treat that as an
/// "unsupported file" error.
pub fn probe_dimensions(ffmpeg: &Path, input: &Path) -> Option<(u32, u32)> {
    let p = probe_video(ffmpeg, input);
    match (p.width, p.height) {
        (Some(w), Some(h)) => Some((w, h)),
        _ => None,
    }
}

/// Extract one preview frame at `time_seconds` into `out_path` as a
/// JPEG. Used by the Crop dialog as a fallback when WebView2 can't
/// decode the source format natively (ProRes / DNxHD / weird MKVs).
/// Quality is medium-ish — preview frames don't need to be
/// pristine, and a smaller file means it loads instantly.
pub fn extract_preview_frame(
    ffmpeg: &Path,
    input: &Path,
    time_seconds: f64,
    out_path: &Path,
) -> Result<()> {
    let mut cmd = Command::new(ffmpeg);
    // -ss BEFORE -i = fast keyframe seek (input-side). For preview
    // frames we trade a few ms of seek inaccuracy for ~10x speed.
    cmd.args(["-v", "error", "-y"])
        .args(["-ss", &format!("{time_seconds}")])
        .arg("-i")
        .arg(input)
        .args(["-frames:v", "1"])
        .args(["-q:v", "5"])
        .arg(out_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut cmd);
    let status = cmd.status().context("spawning ffmpeg for preview frame")?;
    if !status.success() {
        bail!("ffmpeg preview-frame extraction failed");
    }
    Ok(())
}

/// Bundle of transform options the Modify dialog can request. Any
/// combination is valid as long as at least one transform is
/// active — `commands::encode_modify` enforces that up front.
#[derive(Debug, Clone)]
pub struct ModifySpec {
    /// Crop rect in source pixels (x, y, w, h). `None` means no
    /// crop — only the other transforms run.
    pub crop_rect: Option<(u32, u32, u32, u32)>,
    pub flip_h: bool,
    pub flip_v: bool,
    pub reverse: bool,
    /// Clockwise rotation in degrees. Only 0 / 90 / 180 / 270 are
    /// honoured; anything else is treated as 0 by `build_filter_chain`.
    pub rotate: u32,
    /// Drop the audio stream entirely. Forwarded into the derived
    /// preset and consumed by the MP4 encode branch (`-an` instead
    /// of the AAC re-encode + any `-af` audio filters).
    pub remove_audio: bool,
    /// Trim range in seconds. `None` on either end means "don't seek
    /// that side" — both `None` is the no-trim default. Set by the
    /// Modify dialog's two draggable handles overlaying the scrub
    /// timeline. Ignored for image inputs.
    pub trim_start_sec: Option<f32>,
    pub trim_end_sec: Option<f32>,
    /// Replace the source file with the encoded output. Implemented
    /// as encode-to-temp + atomic rename so a failure leaves the
    /// source untouched.
    pub overwrite: bool,
}

/// Modify tool: per-file rectangular crop + optional flip / reverse
/// / overwrite. Routes through `encode_file` with a derived preset
/// that carries the transform flags via `#[serde(skip)]` Preset
/// fields. The actual filter chain is assembled in
/// `build_filter_chain`.
///
/// For overwrite, we encode to a temp file alongside the source,
/// then atomically rename over the source. A mid-encode failure
/// leaves the temp file behind for manual recovery and the source
/// untouched.
pub fn encode_modify_files(
    ffmpeg: &Path,
    files: &[PathBuf],
    spec: ModifySpec,
    settings: &Settings,
    mut on_progress: impl FnMut(ProgressEvent),
) -> Result<()> {
    if files.is_empty() {
        bail!("Modify needs at least one file");
    }
    let total = files.len();

    for (idx, input) in files.iter().enumerate() {
        let file_index = idx + 1;
        let input_display = input.display().to_string();

        // Clamp the crop rect into THIS input's bounds. The dialog
        // built the rect against the FIRST file's dimensions; if a
        // later file is smaller we'd otherwise emit a filter that
        // reads pixels outside the frame and ffmpeg would error.
        //
        // Also force W and H to be even. x264 + yuv420p (our MP4
        // encode path) reject odd dimensions outright with "height
        // not divisible by 2" — and the failure mode is non-obvious
        // because it triggers only when the user crops one edge by
        // an odd number of pixels (leaving the other at the default
        // makes an odd cropW the easy way to hit it). One-pixel cost
        // on the freehand selection is invisible; keeps GIF / image
        // outputs identical too since they already accept even sizes.
        let clamped_rect = spec.crop_rect.map(|(rx, ry, rw, rh)| {
            let (src_w, src_h) = probe_dimensions(ffmpeg, input).unwrap_or((rx + rw, ry + rh));
            let cx = rx.min(src_w.saturating_sub(1));
            let cy = ry.min(src_h.saturating_sub(1));
            let cw = rw.min(src_w.saturating_sub(cx)).max(1);
            let ch = rh.min(src_h.saturating_sub(cy)).max(1);
            let cw = (cw & !1).max(2);
            let ch = (ch & !1).max(2);
            (cx, cy, cw, ch)
        });

        let preset = derive_modify_preset(
            ffmpeg,
            input,
            clamped_rect,
            spec.flip_h,
            spec.flip_v,
            spec.reverse,
            spec.remove_audio,
            spec.rotate,
            spec.trim_start_sec,
            spec.trim_end_sec,
        );

        let mut bits: Vec<String> = Vec::new();
        if let Some((x, y, w, h)) = clamped_rect {
            bits.push(format!("crop {w}x{h} at ({x}, {y})"));
        }
        if spec.rotate == 90 || spec.rotate == 180 || spec.rotate == 270 {
            bits.push(format!("rotate-{}", spec.rotate));
        }
        if spec.flip_h { bits.push("flip-h".into()); }
        if spec.flip_v { bits.push("flip-v".into()); }
        if spec.reverse { bits.push("reverse".into()); }
        if spec.remove_audio { bits.push("remove-audio".into()); }
        if spec.trim_start_sec.is_some() || spec.trim_end_sec.is_some() {
            let s = spec.trim_start_sec.unwrap_or(0.0);
            let e_str = spec
                .trim_end_sec
                .map(|e| format!("{:.2}", e))
                .unwrap_or_else(|| "end".to_string());
            bits.push(format!("trim {s:.2}–{e_str}s"));
        }
        let summary = if bits.is_empty() { "encoding".into() } else { bits.join(" + ") };

        on_progress(ProgressEvent {
            file_index,
            total_files: total,
            input: input_display.clone(),
            stage: "encode".into(),
            percent: None,
            message: Some(format!("Modify: {summary}")),
        });

        let encode_input = EncodeInput::File(input.clone());
        let duration = encode_input.duration_hint(ffmpeg);

        // Build the output preset's expected destination. encode_file
        // calls `output_path(input, preset)` internally; we need to
        // know what that landed at when we want to rename it over the
        // source. For overwrite we rewrite the suffix to a unique temp
        // marker and rename in this function after encode.
        let final_path = if spec.overwrite {
            input.clone()
        } else {
            output_path(&encode_input, &preset)
        };

        // Override the preset suffix with a unique temp tag when
        // overwriting so encode_file writes alongside the source
        // without clobbering it. We rename onto the source path
        // after encode succeeds.
        let mut preset_for_encode = preset.clone();
        let tmp_path: Option<PathBuf> = if spec.overwrite {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            preset_for_encode.suffix = format!("._modify_tmp_{nonce}");
            Some(output_path(&encode_input, &preset_for_encode))
        } else {
            None
        };

        let result = encode_file(
            ffmpeg,
            &encode_input,
            &preset_for_encode,
            settings,
            duration,
            file_index,
            total,
            |ev| on_progress(ev),
        );
        if let Err(e) = result {
            // Clean up the half-written temp on overwrite failure so
            // the source folder doesn't fill up with .modify_tmp_*
            // files over time.
            if let Some(ref tmp) = tmp_path {
                let _ = std::fs::remove_file(tmp);
            }
            on_progress(ProgressEvent {
                file_index,
                total_files: total,
                input: input_display,
                stage: "error".into(),
                percent: None,
                message: Some(e.to_string()),
            });
            continue;
        }

        // Overwrite path: rename the temp over the source. Rust's
        // `fs::rename` on Windows uses MoveFileExW with
        // REPLACE_EXISTING semantics, so this is atomic from any
        // observer's perspective: the source either still has its
        // old bytes, or has the new ones, never an empty/partial
        // state.
        if let Some(tmp) = tmp_path {
            if let Err(e) = std::fs::rename(&tmp, &final_path) {
                let _ = std::fs::remove_file(&tmp);
                on_progress(ProgressEvent {
                    file_index,
                    total_files: total,
                    input: input_display.clone(),
                    stage: "error".into(),
                    percent: None,
                    message: Some(format!(
                        "Encode succeeded but overwrite failed: {e}. The original file is unchanged.",
                    )),
                });
                continue;
            }
            on_progress(ProgressEvent {
                file_index,
                total_files: total,
                input: input_display,
                stage: "done".into(),
                percent: Some(1.0),
                message: Some(format!("{} (overwritten)", final_path.display())),
            });
        }
    }

    Ok(())
}

/// Build a per-file preset for the Modify tool. Format mirrors the
/// input (gif → gif, mp4-ish → mp4, image → image of matching
/// codec). The transforms ride along on Preset's skip-serialized
/// fields (`crop_rect`, `modify_flip_h`, `modify_flip_v`,
/// `modify_reverse`, `modify_remove_audio`, `modify_rotate`) that
/// `build_filter_chain` and the encode dispatcher read.
#[allow(clippy::too_many_arguments)]
pub fn derive_modify_preset(
    ffmpeg: &Path,
    input: &Path,
    crop_rect: Option<(u32, u32, u32, u32)>,
    flip_h: bool,
    flip_v: bool,
    reverse: bool,
    remove_audio: bool,
    rotate: u32,
    trim_start_sec: Option<f32>,
    trim_end_sec: Option<f32>,
) -> Preset {
    use crate::presets::{Dither, Format, ImageCodec};

    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_ascii_lowercase();

    if is_image_path(input) {
        let codec = image_codec_from_ext(input);
        return Preset {
            id: "__modify__".into(),
            name: "Modify".into(),
            enabled: true,
            format: Format::Image,
            suffix: "_modified".into(),
            width: None,
            height: None,
            fps: None,
            crop: None,
            palette_colors: None,
            dither: None,
            bayer_scale: None,
            crf: None,
            preset_speed: None,
            video_bitrate: None,
            audio_bitrate: None,
            use_cuda: None,
            target_max_mb: None,
            image_codec: Some(codec.clone()),
            image_quality: Some(codec.default_quality()),
            strip_metadata: Some(false),
            grayscale: None,
            timecode: None,
            guides: None,
            overlay: None,
            crop_rect: crop_rect,
            modify_flip_h: Some(flip_h),
            modify_flip_v: Some(flip_v),
            modify_reverse: Some(reverse),
            modify_overwrite: None,
            // Images have no audio track, but we plumb the flag
            // through anyway so the field stays a single source of
            // truth no matter what branch ran.
            modify_remove_audio: Some(remove_audio),
            modify_rotate: Some(rotate),
            modify_trim_start_sec: trim_start_sec,
            modify_trim_end_sec: trim_end_sec,
            watermark: None,
            icon: None,
            order: 0,
        };
    }

    let format = if ext == "gif" { Format::Gif } else { Format::Mp4 };
    let probe = probe_video(ffmpeg, input);
    Preset {
        id: "__modify__".into(),
        name: "Modify".into(),
        enabled: true,
        format,
        suffix: "_modified".into(),
        // Don't pre-resize — the user expects the crop dimensions
        // to be the output dimensions exactly. Width/height left
        // None means the chain doesn't insert a `scale=...` after
        // the `crop=...`.
        width: None,
        height: None,
        fps: probe.fps,
        crop: None,
        palette_colors: Some(128),
        dither: Some(Dither::Bayer),
        bayer_scale: Some(3),
        // Visually-lossless baseline — Crop is "preserve everything,
        // just remove pixels outside the rect".
        crf: Some(18),
        preset_speed: Some("medium".into()),
        video_bitrate: None,
        audio_bitrate: Some("192k".into()),
        use_cuda: Some(false),
        target_max_mb: None,
        image_codec: None,
        image_quality: None,
        strip_metadata: None,
        grayscale: None,
        timecode: None,
        guides: None,
        overlay: None,
        crop_rect: crop_rect,
        modify_flip_h: Some(flip_h),
        modify_flip_v: Some(flip_v),
        modify_reverse: Some(reverse),
        modify_overwrite: None,
        modify_remove_audio: Some(remove_audio),
        modify_rotate: Some(rotate),
        modify_trim_start_sec: trim_start_sec,
        modify_trim_end_sec: trim_end_sec,
        watermark: None,
        icon: None,
        order: 0,
    }
}

/// Image-only Compare: stack N stills horizontally into one still.
/// Output format matches the first input's codec (JPEG → JPEG, PNG →
/// PNG, etc.). Falls back to PNG for unrecognised extensions
/// (BMP/TIFF) so we always produce something the user can open.
///
/// Skips everything the video path needs (fps normalization, duration
/// scrubbing, palette generation) — for stills they'd be either
/// useless or wrong.
fn encode_compare_images(
    ffmpeg: &Path,
    files: &[PathBuf],
    settings: &Settings,
    mut on_progress: impl FnMut(ProgressEvent),
) -> Result<PathBuf> {
    use crate::presets::ImageCodec;

    let first = &files[0];
    let n = files.len();
    let probe = probe_video(ffmpeg, first);
    // Pad height up to a sane minimum so very small inputs don't
    // produce a strip narrower than the file-format demands.
    let height = probe.height.unwrap_or(720).max(120);
    let stem = first
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");

    // Pick output codec from the first input's extension.
    let first_ext = first
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    let codec = match first_ext.as_str() {
        "jpg" | "jpeg" => ImageCodec::Jpeg,
        "webp" => ImageCodec::Webp,
        "avif" => ImageCodec::Avif,
        // png, bmp, tif, tiff — anything else.
        _ => ImageCodec::Png,
    };
    let out_ext = codec.ext();

    // Build the same scale+hstack graph as the video path, minus the
    // fps filter (no time domain on stills) and minus setsar (image
    // sources have square pixels by default).
    let mut norm = String::new();
    for i in 0..n {
        if i > 0 {
            norm.push(';');
        }
        norm.push_str(&format!("[{i}:v]scale=-2:{height}:flags=lanczos[v{i}]"));
    }
    let mut stacked = String::new();
    for i in 0..n {
        stacked.push_str(&format!("[v{i}]"));
    }
    stacked.push_str(&format!("hstack=inputs={n}"));

    let base = first
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf()
        .join(format!("{stem}_compare.{out_ext}"));
    let out = unique_output_path(&base);

    let verbosity = settings.verbosity.clone().unwrap_or_else(|| "warning".into());
    let input_display = format!("compare: {stem}");
    let total_files = 1usize;
    let file_index = 1usize;

    on_progress(ProgressEvent {
        file_index,
        total_files,
        input: input_display.clone(),
        stage: "encode".into(),
        percent: None,
        message: Some(format!(
            "Stacking {n} images → {}",
            out_ext.to_ascii_uppercase()
        )),
    });

    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-v", &verbosity, "-y", "-hide_banner"]);
    for f in files {
        cmd.arg("-i").arg(f);
    }
    let filter = format!("{norm};{stacked}[vh]");
    cmd.args(["-filter_complex", &filter])
        .args(["-map", "[vh]"])
        .args(["-frames:v", "1"]);

    // Per-codec output args, matching the encode_file image branch's
    // sensible defaults. We DO NOT pull from any user preset here —
    // Compare is a tool, not a preset, so it uses fixed quality.
    match codec {
        ImageCodec::Png => {
            cmd.args(["-c:v", "png", "-compression_level", "6"]);
        }
        ImageCodec::Jpeg => {
            // q:v 3 ≈ "high quality" (~ UI 90 on the 1-100 scale).
            cmd.args(["-c:v", "mjpeg", "-q:v", "3", "-pix_fmt", "yuvj420p"]);
        }
        ImageCodec::Webp => {
            cmd.args(["-c:v", "libwebp", "-quality", "85", "-lossless", "0"]);
        }
        ImageCodec::Avif => {
            cmd.args(["-c:v", "libaom-av1", "-crf", "24", "-still-picture", "1"]);
        }
    }

    cmd.arg(&out);
    run_with_progress_cleanup(
        cmd,
        None,
        file_index,
        total_files,
        &input_display,
        "encode",
        &out,
        &mut on_progress,
    )?;

    on_progress(ProgressEvent {
        file_index,
        total_files,
        input: input_display,
        stage: "done".into(),
        percent: Some(1.0),
        message: Some(out.display().to_string()),
    });
    Ok(out)
}

