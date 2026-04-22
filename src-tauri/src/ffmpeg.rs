use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use crate::paths;
use crate::presets::{Crop, Dither, Format, Preset, Settings};

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

pub fn resolve_ffmpeg(settings: &Settings) -> Result<PathBuf> {
    if let Some(ref s) = settings.ffmpeg_path {
        let p = PathBuf::from(s);
        if p.exists() {
            return Ok(p);
        }
    }
    let managed = paths::ffmpeg_managed_path()?;
    if managed.exists() {
        return Ok(managed);
    }
    // Fall back to PATH lookup
    if let Some(p) = which("ffmpeg") {
        return Ok(p);
    }
    bail!("ffmpeg.exe not found. Install via app settings or add to PATH.")
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

pub fn output_path(input: &Path, preset: &Preset) -> PathBuf {
    let ext = match preset.format {
        Format::Gif => "gif",
        Format::Mp4 => "mp4",
    };
    let parent = input.parent().unwrap_or(Path::new("."));
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    parent.join(format!("{stem}{}.{ext}", preset.suffix))
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
    if let Some(fps) = preset.fps {
        parts.push(format!("fps={fps}"));
    }
    if let Some(ref c) = preset.crop {
        parts.push(crop_expr(c).to_string());
    }
    if let Some(s) = scale_expr(preset) {
        parts.push(s);
    }
    parts.join(",")
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
    input: &Path,
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
                input: input.display().to_string(),
                stage: "encode".into(),
                percent: None,
                message: Some(stage_msg),
            });

            let mut cmd = Command::new(ffmpeg);
            cmd.args(["-v", &verbosity, "-y", "-hide_banner", "-i"]).arg(input);
            if !filter.is_empty() {
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
            cmd.args(["-c:a", "aac", "-b:a", &abr]);
            cmd.args(["-movflags", "+faststart"]);
            cmd.args(["-progress", "pipe:1"]).arg(&out);
            run_with_progress(cmd, duration_s, file_index, total_files, input, "encode", &mut on_progress)?;
        }
    }

    on_progress(ProgressEvent {
        file_index,
        total_files,
        input: input.display().to_string(),
        stage: "done".into(),
        percent: Some(1.0),
        message: Some(out.display().to_string()),
    });

    Ok(out)
}

fn run_with_progress(
    mut cmd: Command,
    duration_s: Option<f64>,
    file_index: usize,
    total_files: usize,
    input: &Path,
    stage: &str,
    on_progress: &mut impl FnMut(ProgressEvent),
) -> Result<()> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    hide_console(&mut cmd);
    let mut child = cmd.spawn().context("spawning ffmpeg")?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
    let reader = BufReader::new(stdout);

    for line in reader.lines().map_while(|l| l.ok()) {
        if let Some(rest) = line.strip_prefix("out_time_ms=") {
            if let (Ok(us), Some(total)) = (rest.trim().parse::<i64>(), duration_s) {
                let s = us as f64 / 1_000_000.0;
                let pct = (s / total).clamp(0.0, 1.0) as f32;
                on_progress(ProgressEvent {
                    file_index,
                    total_files,
                    input: input.display().to_string(),
                    stage: stage.into(),
                    percent: Some(pct),
                    message: None,
                });
            }
        }
    }

    let status = child.wait()?;
    if !status.success() {
        bail!("ffmpeg exited with status {status}");
    }
    Ok(())
}

/// One GIF encode pass (palettegen + paletteuse). `width_override` lets the
/// caller shrink the output between iterations when hitting a size target.
#[allow(clippy::too_many_arguments)]
fn encode_gif_once(
    ffmpeg: &Path,
    input: &Path,
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
    let filter = parts.join(",");

    // Pass 1: palette
    let palette_tmp = out.with_extension("palette.png");
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
        input: input.display().to_string(),
        stage: "palette".into(),
        percent: None,
        message: Some(
            extra_msg
                .clone()
                .unwrap_or_else(|| "Generating palette".into()),
        ),
    });

    let mut palette_cmd = Command::new(ffmpeg);
    palette_cmd
        .args(["-v", verbosity, "-y", "-i"])
        .arg(input)
        .args(["-vf", &filter_p1])
        .arg(&palette_tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut palette_cmd);
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
        input: input.display().to_string(),
        stage: "encode".into(),
        percent: None,
        message: Some(extra_msg.unwrap_or_else(|| "Encoding GIF".into())),
    });

    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-v", verbosity, "-y", "-hide_banner", "-i"])
        .arg(input)
        .arg("-i")
        .arg(&palette_tmp)
        .args(["-filter_complex", &filter_complex])
        .args(["-progress", "pipe:1"])
        .arg(out);
    run_with_progress(
        cmd,
        duration_s,
        file_index,
        total_files,
        input,
        "encode",
        on_progress,
    )?;

    let _ = std::fs::remove_file(&palette_tmp);
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
