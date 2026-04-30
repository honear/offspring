use serde::Serialize;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::bootstrap;
use crate::defaults;
use crate::ffmpeg::{self, EncodeInput, ProgressEvent};
use crate::integration;
use crate::paths;
use crate::presets::{self, GuidesConfig, OverlayConfig, Preset, Settings, TrimLast};
use crate::sequence;

#[derive(Serialize)]
pub struct FfmpegStatus {
    pub found: bool,
    pub path: Option<String>,
}

#[tauri::command]
pub fn list_presets() -> Result<Vec<Preset>, String> {
    presets::load_presets().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_presets(presets_in: Vec<Preset>) -> Result<(), String> {
    presets::save_presets(&presets_in).map_err(|e| e.to_string())?;
    let settings = presets::load_settings().unwrap_or_default();
    integration::sync_all(&presets_in, &settings).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn reset_presets_to_defaults() -> Result<Vec<Preset>, String> {
    let d = defaults::default_presets();
    presets::save_presets(&d).map_err(|e| e.to_string())?;
    let settings = presets::load_settings().unwrap_or_default();
    integration::sync_all(&d, &settings).map_err(|e| e.to_string())?;
    Ok(d)
}

#[tauri::command]
pub fn get_settings() -> Result<Settings, String> {
    presets::load_settings().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_settings(settings: Settings) -> Result<(), String> {
    presets::save_settings(&settings).map_err(|e| e.to_string())?;
    // Toggling sendto / modern-menu should take effect immediately rather
    // than at next first-run, so re-sync integrations against the new
    // settings now.
    let ps = presets::load_presets().unwrap_or_default();
    integration::sync_all(&ps, &settings).map_err(|e| e.to_string())?;
    Ok(())
}

/// Kick off an FFmpeg download in the background. Progress arrives on the
/// `ffmpeg-download` event with phase "downloading" | "extracting" | "done"
/// | "error". Returns immediately once the worker thread is spawned.
#[tauri::command]
pub fn download_ffmpeg(app: tauri::AppHandle) -> Result<(), String> {
    bootstrap::spawn_download(app);
    Ok(())
}

#[tauri::command]
pub fn ffmpeg_status() -> FfmpegStatus {
    let s = presets::load_settings().unwrap_or_default();
    match ffmpeg::resolve_ffmpeg(&s) {
        Ok(p) => FfmpegStatus { found: true, path: Some(p.display().to_string()) },
        Err(_) => FfmpegStatus { found: false, path: None },
    }
}

#[tauri::command]
pub fn get_custom_last() -> Result<Preset, String> {
    presets::load_custom_last().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_custom_last(preset: Preset) -> Result<(), String> {
    presets::save_custom_last(&preset).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_trim_last() -> Result<TrimLast, String> {
    presets::load_trim_last().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_trim_last(trim: TrimLast) -> Result<(), String> {
    presets::save_trim_last(&trim).map_err(|e| e.to_string())
}

/// Re-apply every shell integration (context menu, SendTo, modern menu)
/// from current on-disk state. Exposed so Settings UI can offer a
/// "Re-sync right-click menus" button when things drift — e.g. the user
/// deleted a shortcut manually and wants it back.
#[tauri::command]
pub fn sync_integrations() -> Result<(), String> {
    let ps = presets::load_presets().map_err(|e| e.to_string())?;
    let settings = presets::load_settings().unwrap_or_default();
    integration::sync_all(&ps, &settings).map_err(|e| e.to_string())
}

/// Kill + relaunch Explorer so it picks up a freshly-registered shell
/// extension. The frontend only calls this after the user confirms via
/// a dialog — `modern_menu::sync` never invokes it silently.
#[tauri::command]
pub fn restart_explorer() -> Result<(), String> {
    integration::modern_menu::restart_explorer().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_data_folder() -> Result<(), String> {
    let p = paths::data_dir().map_err(|e| e.to_string())?;
    // Resolve the absolute path to Explorer so a planted `explorer.exe`
    // in a PATH entry / current dir can't be invoked instead. Falls
    // back to the bare name if `%SystemRoot%` is unset, which keeps
    // this working on systems where the env var has been scrubbed.
    let exe = match std::env::var_os("SystemRoot") {
        Some(root) => {
            let candidate = std::path::PathBuf::from(root).join("explorer.exe");
            if candidate.exists() {
                candidate
            } else {
                std::path::PathBuf::from("explorer.exe")
            }
        }
        None => std::path::PathBuf::from("explorer.exe"),
    };
    std::process::Command::new(exe)
        .arg(&p)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn encode(
    app: tauri::AppHandle,
    files: Vec<String>,
    preset: Preset,
) -> Result<(), String> {
    let settings = presets::load_settings().unwrap_or_default();
    let ffmpeg_path = ffmpeg::resolve_ffmpeg(&settings).map_err(|e| e.to_string())?;

    // Sequence auto-detection: when the Sequence tool is enabled, map
    // each input to an EncodeInput. Image frames that match a sibling
    // run become Sequence variants (ffmpeg reads the whole numbered
    // pattern). Non-images and single frames pass through as File. We
    // also dedupe frames that belong to the same sequence so one
    // right-click on three frames of the same run doesn't trigger three
    // identical encodes.
    let tools = settings.tools.clone();
    // Preset-fps wins over the sequence tool's default. The preset
    // field is integer (Option<u32>); the tool default is float so
    // VFX rates like 23.976 / 29.97 are expressible for MP4 presets
    // that leave `fps` unset.
    let preset_fps: f32 = preset
        .fps
        .map(|f| f as f32)
        .unwrap_or(tools.sequence.default_fps);
    let raw_paths: Vec<std::path::PathBuf> = files.iter().map(std::path::PathBuf::from).collect();
    let collapsed = if tools.sequence.enabled {
        sequence::dedupe_sequence_frames(&raw_paths, tools.sequence.min_digits)
    } else {
        raw_paths
    };
    let inputs: Vec<EncodeInput> = collapsed
        .into_iter()
        .map(|p| {
            if tools.sequence.enabled {
                match sequence::detect(&p, tools.sequence.min_digits) {
                    Some(info) => EncodeInput::Sequence { info, fps: preset_fps },
                    None => EncodeInput::File(p),
                }
            } else {
                EncodeInput::File(p)
            }
        })
        .collect();
    let total = inputs.len();

    std::thread::spawn(move || {
        for (i, input) in inputs.iter().enumerate() {
            let duration = input.duration_hint(&ffmpeg_path);
            let app_cl = app.clone();
            let result = ffmpeg::encode_file(
                &ffmpeg_path,
                input,
                &preset,
                &settings,
                duration,
                i + 1,
                total,
                move |ev: ProgressEvent| {
                    let _ = app_cl.emit("encode-progress", ev);
                },
            );
            if let Err(e) = result {
                // Surface per-file failures so the UI can show an error state
                // rather than silently skipping them and reporting "Done".
                let _ = app.emit(
                    "encode-progress",
                    ProgressEvent {
                        file_index: i + 1,
                        total_files: total,
                        input: input.display(),
                        stage: "error".into(),
                        percent: None,
                        message: Some(e.to_string()),
                    },
                );
            }
        }
        let _ = app.emit("encode-finished", total);
    });

    Ok(())
}

/// Merge N inputs into one. For MP4 output we use ffmpeg's **concat
/// filter** (`-filter_complex concat=n=N:v=1:a=?`) with per-input
/// scale/pad/setsar/fps/format normalization, so mismatched inputs
/// (different resolutions, framerates, pixel formats, or codecs)
/// still produce a valid merged file. For GIF output we still use the
/// concat demuxer path via `encode_file` — GIF inputs come from the
/// same pipeline so they're usually consistent, and the demuxer path
/// preserves palette behavior.
///
/// Format (gif vs mp4) is inferred from the first file's extension
/// and dimensions/fps are probed from its first video stream. Inputs
/// are sorted by filename so order matches what the user sees in
/// Explorer. Output lives next to the first sorted file, named
/// `<first-stem>_merged.<ext>` (with `_NN` suffix if that exists).
#[tauri::command]
pub fn encode_merge(
    app: tauri::AppHandle,
    files: Vec<String>,
) -> Result<(), String> {
    if files.len() < 2 {
        return Err("Merge needs at least two files".into());
    }
    let settings = presets::load_settings().unwrap_or_default();
    let ffmpeg_path = ffmpeg::resolve_ffmpeg(&settings).map_err(|e| e.to_string())?;

    // Sort by filename so the merge order is predictable regardless of
    // which order Explorer happened to pass the selection in.
    let mut sorted: Vec<std::path::PathBuf> =
        files.iter().map(std::path::PathBuf::from).collect();
    sorted.sort_by(|a, b| {
        a.file_name()
            .unwrap_or_default()
            .cmp(b.file_name().unwrap_or_default())
    });

    let first = sorted.first().cloned().ok_or("no files after sort")?;
    let output_dir = first
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    let first_stem = first
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("merged")
        .to_string();
    let output_stem = format!("{first_stem}_merged");

    let first_ext = first
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_ascii_lowercase();
    let is_gif = first_ext == "gif";

    // Best-effort: sum ffprobe durations so the progress bar has a
    // denominator. If any probe fails we just pass None.
    let mut total_dur: Option<f64> = Some(0.0);
    for p in &sorted {
        match ffmpeg::probe_duration(&ffmpeg_path, p) {
            Some(d) => {
                if let Some(t) = total_dur.as_mut() {
                    *t += d;
                }
            }
            None => total_dur = None,
        }
    }

    if is_gif {
        // GIF path: keep the concat-demuxer flow. Works for matching
        // inputs (the common case when GIFs come from the same source
        // pipeline) and preserves palette behavior.
        let preset = ffmpeg::derive_merge_preset(&ffmpeg_path, &first);
        let list_path = paths::local_data_dir()
            .map_err(|e| e.to_string())?
            .join(format!("merge-list-{}.txt", std::process::id()));
        let list_body: String = sorted
            .iter()
            .map(|p| {
                let s = p.display().to_string();
                // ffmpeg's concat demuxer parses `\` as the escape
                // character inside single-quoted strings, so we have to
                // double any literal backslashes before they reach the
                // file. Then close-and-reopen the quote pair around any
                // `'`, and finally drop control characters that could
                // otherwise inject a new `file '...'` line into the
                // listing.
                let mut escaped = String::with_capacity(s.len());
                for c in s.chars() {
                    if (c as u32) < 0x20 {
                        // CR/LF/etc. shouldn't appear in Windows paths,
                        // but if a hostile filename reached us through
                        // some other channel, swallowing them keeps the
                        // listing single-line.
                        continue;
                    }
                    match c {
                        '\\' => escaped.push_str("\\\\"),
                        '\'' => escaped.push_str("'\\''"),
                        _ => escaped.push(c),
                    }
                }
                format!("file '{escaped}'\n")
            })
            .collect();
        std::fs::write(&list_path, list_body)
            .map_err(|e| format!("writing concat list: {e}"))?;

        let input = EncodeInput::Concat {
            list_path: list_path.clone(),
            output_dir,
            output_stem,
            total_duration_s: total_dur,
        };

        std::thread::spawn(move || {
            let app_cl = app.clone();
            let result = ffmpeg::encode_file(
                &ffmpeg_path,
                &input,
                &preset,
                &settings,
                total_dur,
                1,
                1,
                move |ev: ProgressEvent| {
                    let _ = app_cl.emit("encode-progress", ev);
                },
            );
            if let Err(e) = result {
                let _ = app.emit(
                    "encode-progress",
                    ProgressEvent {
                        file_index: 1,
                        total_files: 1,
                        input: input.display(),
                        stage: "error".into(),
                        percent: None,
                        message: Some(e.to_string()),
                    },
                );
            }
            let _ = std::fs::remove_file(&list_path);
            let _ = app.emit("encode-finished", 1usize);
        });
        return Ok(());
    }

    // MP4 path: concat filter with per-input normalization.
    let probe = ffmpeg::probe_video(&ffmpeg_path, &first);
    let target_w = probe.width.unwrap_or(1280);
    let target_h = probe.height.unwrap_or(720);
    let target_fps = probe.fps.unwrap_or(30);
    let output_path =
        ffmpeg::unique_output_path(&output_dir.join(format!("{output_stem}.mp4")));
    let input_display = format!("merge: {} files", sorted.len());
    let verbosity = settings
        .verbosity
        .clone()
        .unwrap_or_else(|| "warning".into());

    std::thread::spawn(move || {
        let app_cl = app.clone();
        let result = ffmpeg::encode_merge_filter(
            &ffmpeg_path,
            &sorted,
            &output_path,
            target_w,
            target_h,
            target_fps,
            23,
            "medium",
            "128k",
            &verbosity,
            total_dur,
            move |ev: ProgressEvent| {
                let _ = app_cl.emit("encode-progress", ev);
            },
        );
        if let Err(e) = result {
            let _ = app.emit(
                "encode-progress",
                ProgressEvent {
                    file_index: 1,
                    total_files: 1,
                    input: input_display,
                    stage: "error".into(),
                    percent: None,
                    message: Some(e.to_string()),
                },
            );
        }
        let _ = app.emit("encode-finished", 1usize);
    });

    Ok(())
}

/// Encode each input to a greyscale copy, inheriting format, dimensions
/// and fps from the source. Per-file — no concat. Runs through the same
/// `encode_file` pipeline as preset encodes, so all the usual machinery
/// (progress events, sequence auto-detect, pix_fmt fix) applies.
#[tauri::command]
pub fn encode_grayscale(
    app: tauri::AppHandle,
    files: Vec<String>,
) -> Result<(), String> {
    if files.is_empty() {
        return Err("Greyscale needs at least one file".into());
    }
    let settings = presets::load_settings().unwrap_or_default();
    let ffmpeg_path = ffmpeg::resolve_ffmpeg(&settings).map_err(|e| e.to_string())?;

    let tools = settings.tools.clone();
    let raw_paths: Vec<std::path::PathBuf> = files.iter().map(std::path::PathBuf::from).collect();
    let collapsed = if tools.sequence.enabled {
        sequence::dedupe_sequence_frames(&raw_paths, tools.sequence.min_digits)
    } else {
        raw_paths
    };

    // Derive a per-file preset so each input keeps its own format /
    // dimensions / fps rather than being forced to match the first.
    let jobs: Vec<(EncodeInput, Preset)> = collapsed
        .into_iter()
        .map(|p| {
            let preset = ffmpeg::derive_grayscale_preset(&ffmpeg_path, &p);
            let input = if tools.sequence.enabled {
                match sequence::detect(&p, tools.sequence.min_digits) {
                    Some(info) => EncodeInput::Sequence {
                        info,
                        fps: preset
                            .fps
                            .map(|f| f as f32)
                            .unwrap_or(tools.sequence.default_fps),
                    },
                    None => EncodeInput::File(p),
                }
            } else {
                EncodeInput::File(p)
            };
            (input, preset)
        })
        .collect();
    let total = jobs.len();

    std::thread::spawn(move || {
        for (i, (input, preset)) in jobs.iter().enumerate() {
            let duration = input.duration_hint(&ffmpeg_path);
            let app_cl = app.clone();
            let result = ffmpeg::encode_file(
                &ffmpeg_path,
                input,
                preset,
                &settings,
                duration,
                i + 1,
                total,
                move |ev: ProgressEvent| {
                    let _ = app_cl.emit("encode-progress", ev);
                },
            );
            if let Err(e) = result {
                let _ = app.emit(
                    "encode-progress",
                    ProgressEvent {
                        file_index: i + 1,
                        total_files: total,
                        input: input.display(),
                        stage: "error".into(),
                        percent: None,
                        message: Some(e.to_string()),
                    },
                );
            }
        }
        let _ = app.emit("encode-finished", total);
    });

    Ok(())
}

/// A/B Compare tool: stack N inputs horizontally into one output video
/// for visual comparison. Single-output, independent ffmpeg pipeline —
/// does not go through `encode_file`. Output is named
/// `<first-stem>_compare.<ext>` in the first file's directory.
#[tauri::command]
pub fn encode_compare(
    app: tauri::AppHandle,
    files: Vec<String>,
) -> Result<(), String> {
    if files.len() < 2 {
        return Err("Compare needs at least two files".into());
    }
    let settings = presets::load_settings().unwrap_or_default();
    let ffmpeg_path = ffmpeg::resolve_ffmpeg(&settings).map_err(|e| e.to_string())?;
    let paths: Vec<std::path::PathBuf> =
        files.iter().map(std::path::PathBuf::from).collect();

    std::thread::spawn(move || {
        let app_cl = app.clone();
        let result = ffmpeg::encode_compare_files(
            &ffmpeg_path,
            &paths,
            &settings,
            move |ev: ProgressEvent| {
                let _ = app_cl.emit("encode-progress", ev);
            },
        );
        if let Err(e) = result {
            let first_display = paths
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let _ = app.emit(
                "encode-progress",
                ProgressEvent {
                    file_index: 1,
                    total_files: 1,
                    input: first_display,
                    stage: "error".into(),
                    percent: None,
                    message: Some(e.to_string()),
                },
            );
        }
        let _ = app.emit("encode-finished", 1usize);
    });

    Ok(())
}

/// Overlay tool: per-file encode that burns corner text (filename /
/// timecode / custom) and optional border + aspect guides into the clip.
/// Goes through the regular `encode_file` path with a derived preset
/// whose `overlay` field carries the resolved config.
#[tauri::command]
pub fn encode_overlay(
    app: tauri::AppHandle,
    files: Vec<String>,
) -> Result<(), String> {
    if files.is_empty() {
        return Err("Overlay needs at least one file".into());
    }
    let settings = presets::load_settings().unwrap_or_default();
    let ffmpeg_path = ffmpeg::resolve_ffmpeg(&settings).map_err(|e| e.to_string())?;
    let ot = settings.tools.overlay.clone();

    let tools = settings.tools.clone();
    let raw_paths: Vec<std::path::PathBuf> = files.iter().map(std::path::PathBuf::from).collect();
    let collapsed = if tools.sequence.enabled {
        sequence::dedupe_sequence_frames(&raw_paths, tools.sequence.min_digits)
    } else {
        raw_paths
    };

    let jobs: Vec<(EncodeInput, Preset)> = collapsed
        .into_iter()
        .map(|p| {
            let stem = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let cfg = OverlayConfig {
                top_left: ot.top_left.to_kind(),
                top_right: ot.top_right.to_kind(),
                bottom_left: ot.bottom_left.to_kind(),
                bottom_right: ot.bottom_right.to_kind(),
                custom_text: ot.custom_text.clone(),
                custom_text_2: ot.custom_text_2.clone(),
                filename: stem,
                opacity: (ot.opacity.min(100) as f32) / 100.0,
                color: ot.color.clone(),
                border: ot.border,
                metadata: ot.metadata,
                font_scale: (ot.metadata_font_scale.clamp(30, 400) as f32) / 100.0,
                guides: GuidesConfig {
                    show_16_9: ot.guides && ot.show_16_9,
                    show_9_16: ot.guides && ot.show_9_16,
                    show_4_5: ot.guides && ot.show_4_5,
                    color_16_9: ot.color_16_9.clone(),
                    color_9_16: ot.color_9_16.clone(),
                    color_4_5: ot.color_4_5.clone(),
                    opacity: (ot.guides_opacity.min(100) as f32) / 100.0,
                },
            };
            let preset = ffmpeg::derive_overlay_preset(&ffmpeg_path, &p, cfg);
            let input = if tools.sequence.enabled {
                match sequence::detect(&p, tools.sequence.min_digits) {
                    Some(info) => EncodeInput::Sequence {
                        info,
                        fps: preset
                            .fps
                            .map(|f| f as f32)
                            .unwrap_or(tools.sequence.default_fps),
                    },
                    None => EncodeInput::File(p),
                }
            } else {
                EncodeInput::File(p)
            };
            (input, preset)
        })
        .collect();
    let total = jobs.len();

    std::thread::spawn(move || {
        for (i, (input, preset)) in jobs.iter().enumerate() {
            let duration = input.duration_hint(&ffmpeg_path);
            let app_cl = app.clone();
            let result = ffmpeg::encode_file(
                &ffmpeg_path,
                input,
                preset,
                &settings,
                duration,
                i + 1,
                total,
                move |ev: ProgressEvent| {
                    let _ = app_cl.emit("encode-progress", ev);
                },
            );
            if let Err(e) = result {
                let _ = app.emit(
                    "encode-progress",
                    ProgressEvent {
                        file_index: i + 1,
                        total_files: total,
                        input: input.display(),
                        stage: "error".into(),
                        percent: None,
                        message: Some(e.to_string()),
                    },
                );
            }
        }
        let _ = app.emit("encode-finished", total);
    });

    Ok(())
}

/// Trim tool: per-file frame-accurate trim. Strips `start_frames` from
/// the front and `end_frames` from the back of each input, and
/// optionally cuts the inclusive range `[remove_from, remove_to]` out
/// of the middle. Writes `<stem>_trimmed.<ext>` next to each source.
/// Format inherited per file; settings re-encoded at near-lossless
/// baseline (CRF 17 / preset slow / 256k AAC for MP4, 255-color
/// sierra2_4a-dithered palette for GIF). Audio, when present in MP4
/// inputs, is trimmed in sync at frame-derived second boundaries.
#[tauri::command]
pub fn encode_trim(
    app: tauri::AppHandle,
    files: Vec<String>,
    start_frames: u32,
    end_frames: u32,
    remove_from: Option<u32>,
    remove_to: Option<u32>,
) -> Result<(), String> {
    if files.is_empty() {
        return Err("Trim needs at least one file".into());
    }
    // Accept the middle range only when both endpoints are present and
    // form a non-inverted span; the dialog should never send a partial
    // range, but defaulting bad input to "no middle cut" is friendlier
    // than erroring.
    let remove_range: Option<(u32, u32)> = match (remove_from, remove_to) {
        (Some(a), Some(b)) if b >= a => Some((a, b)),
        _ => None,
    };
    if start_frames == 0 && end_frames == 0 && remove_range.is_none() {
        return Err("Nothing to trim — set start/end frames or a middle range.".into());
    }
    let settings = presets::load_settings().unwrap_or_default();
    let ffmpeg_path = ffmpeg::resolve_ffmpeg(&settings).map_err(|e| e.to_string())?;
    let paths_in: Vec<std::path::PathBuf> =
        files.iter().map(std::path::PathBuf::from).collect();
    let total = paths_in.len();

    std::thread::spawn(move || {
        let app_cl = app.clone();
        let result = ffmpeg::encode_trim_files(
            &ffmpeg_path,
            &paths_in,
            start_frames,
            end_frames,
            remove_range,
            &settings,
            move |ev: ProgressEvent| {
                let _ = app_cl.emit("encode-progress", ev);
            },
        );
        if let Err(e) = result {
            // Top-level failure (couldn't even start) — surface as a
            // single error event keyed to the first file so the UI
            // doesn't sit on "Preparing…" forever.
            let first_display = paths_in
                .first()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let _ = app.emit(
                "encode-progress",
                ProgressEvent {
                    file_index: 1,
                    total_files: total,
                    input: first_display,
                    stage: "error".into(),
                    percent: None,
                    message: Some(e.to_string()),
                },
            );
        }
        let _ = app.emit("encode-finished", total);
    });

    Ok(())
}

/// Compute a logical (x, y) that centers a `w × h` window on the cursor,
/// clamped to the monitor the cursor is on. Returns `None` if cursor or
/// monitor lookups fail — caller should then let Tauri choose (typically
/// upper-left of primary monitor). Handles per-monitor DPI correctly by
/// scaling cursor physical → monitor-local logical coords.
fn position_near_cursor(app: &tauri::AppHandle, w: f64, h: f64) -> Option<(f64, f64)> {
    let cursor = app.cursor_position().ok()?;
    let mon = app
        .monitor_from_point(cursor.x, cursor.y)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())?;
    let scale = mon.scale_factor();
    let cx = cursor.x / scale;
    let cy = cursor.y / scale;
    let mut lx = cx - w / 2.0;
    let mut ly = cy - h / 2.0;
    let mon_pos = mon.position();
    let mon_size = mon.size();
    let mon_lx = mon_pos.x as f64 / scale;
    let mon_ly = mon_pos.y as f64 / scale;
    let mon_lw = mon_size.width as f64 / scale;
    let mon_lh = mon_size.height as f64 / scale;
    let margin = 8.0;
    lx = lx.clamp(mon_lx + margin, mon_lx + mon_lw - w - margin);
    ly = ly.clamp(mon_ly + margin, mon_ly + mon_lh - h - margin);
    Some((lx, ly))
}

pub fn open_progress_window(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let (pw, ph) = (380.0, 140.0);
    // Trailing slash is important: svelte.config uses
    // `trailingSlash: 'always'` so each route is prerendered to
    // `build/<route>/index.html`, and the URL that SvelteKit's client router
    // normalises against its registered routes is `/progress/` — NOT
    // `/progress.html`, which 404s on the router even though the file is
    // present on disk.
    let mut b = WebviewWindowBuilder::new(app, "progress", WebviewUrl::App("progress/".into()))
        .title("Offspring — Encoding")
        .inner_size(pw, ph)
        .resizable(false)
        .always_on_top(true)
        .decorations(true);
    if let Some((x, y)) = position_near_cursor(app, pw, ph) {
        b = b.position(x, y);
    }
    b.build()?;
    Ok(())
}

pub fn open_custom_window(app: &tauri::AppHandle, files: Vec<String>) -> anyhow::Result<()> {
    let (pw, ph) = (500.0, 520.0);
    let mut b = WebviewWindowBuilder::new(app, "custom", WebviewUrl::App("custom/".into()))
        .title("Offspring — Custom")
        .inner_size(pw, ph)
        .min_inner_size(460.0, 440.0)
        .resizable(true);
    if let Some((x, y)) = position_near_cursor(app, pw, ph) {
        b = b.position(x, y);
    }
    let w = b.build()?;
    // Files are read by the frontend via a Tauri command.
    // Stash them in app state.
    app.manage_pending_files(files);
    let _ = w; // suppress unused
    Ok(())
}

/// Open the Trim mini dialog. Compact — just two number fields + the
/// file list. The window itself navigates to /progress/ once the user
/// clicks Trim, so we don't need a separate progress-window open here.
///
/// Focus dance at the end: when the shell-extension spawns offspring.exe
/// from Explorer, Explorer keeps the foreground by default and the new
/// dialog ends up behind it. Briefly toggling `always_on_top` bypasses
/// Windows' focus-stealing prevention long enough for `set_focus` to
/// actually move the foreground to us; we drop always-on-top right away
/// so the user can later put another window over the dialog if they
/// want to. Same trick the progress window uses (which keeps it on
/// permanently — cosmetic difference, no behavioral one).
pub fn open_trim_window(app: &tauri::AppHandle, files: Vec<String>) -> anyhow::Result<()> {
    let (pw, ph) = (440.0, 420.0);
    let mut b = WebviewWindowBuilder::new(app, "trim", WebviewUrl::App("trim/".into()))
        .title("Offspring — Trim")
        .inner_size(pw, ph)
        .min_inner_size(380.0, 320.0)
        .resizable(true)
        .focused(true);
    if let Some((x, y)) = position_near_cursor(app, pw, ph) {
        b = b.position(x, y);
    }
    let w = b.build()?;
    app.manage_pending_files(files);
    let _ = w.unminimize();
    let _ = w.set_always_on_top(true);
    let _ = w.set_focus();
    let _ = w.set_always_on_top(false);
    Ok(())
}

pub fn open_main_window(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let (pw, ph) = (880.0, 760.0);
    let mut b = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("".into()))
        .title("Offspring")
        .inner_size(pw, ph)
        .min_inner_size(720.0, 460.0)
        .resizable(true)
        // Tauri intercepts drag events at the native layer by default to
        // power file-drop. That swallows dragover/drop inside the webview,
        // which is what caused the "forbidden" cursor on preset reorder.
        // We don't use native file-drop in the main window — the settings
        // UI is pure HTML5 DnD — so disable the interception here.
        .disable_drag_drop_handler();
    if let Some((x, y)) = position_near_cursor(app, pw, ph) {
        b = b.position(x, y);
    }
    b.build()?;
    Ok(())
}

// App state for pending CLI inputs
#[derive(Default)]
pub struct PendingState {
    pub files: std::sync::Mutex<Vec<String>>,
    pub preset_id: std::sync::Mutex<Option<String>>,
    pub custom_preset: std::sync::Mutex<Option<Preset>>,
    /// True when the progress window should route to `encode_merge`
    /// instead of `encode`. Set by the `Merge` CLI dispatch path.
    pub merge: std::sync::Mutex<bool>,
    /// True when the progress window should route to `encode_grayscale`
    /// instead of `encode`. Set by the `Grayscale` CLI dispatch path.
    pub grayscale: std::sync::Mutex<bool>,
    /// True when the progress window should route to `encode_compare`.
    pub compare: std::sync::Mutex<bool>,
    /// True when the progress window should route to `encode_overlay`.
    pub overlay: std::sync::Mutex<bool>,
    /// True when the user invoked `offspring trim <files>` and the
    /// Trim mini-dialog should open. Distinct from a `trim` "is this
    /// an active trim encode" flag — encoding is driven directly by
    /// the dialog calling `encode_trim`, so we only need to remember
    /// the dialog-routing decision in pending state.
    pub trim_dialog: std::sync::Mutex<bool>,
}

pub trait AppHandleExt {
    fn manage_pending_files(&self, files: Vec<String>);
    fn manage_pending_preset(&self, preset_id: Option<String>);
    fn manage_pending_custom_preset(&self, preset: Option<Preset>);
    fn manage_pending_merge(&self, merge: bool);
    fn manage_pending_grayscale(&self, grayscale: bool);
    fn manage_pending_compare(&self, v: bool);
    fn manage_pending_overlay(&self, v: bool);
    fn manage_pending_trim_dialog(&self, v: bool);
}

impl AppHandleExt for tauri::AppHandle {
    fn manage_pending_files(&self, files: Vec<String>) {
        if let Some(state) = self.try_state::<PendingState>() {
            *state.files.lock().unwrap() = files;
        }
    }
    fn manage_pending_preset(&self, preset_id: Option<String>) {
        if let Some(state) = self.try_state::<PendingState>() {
            *state.preset_id.lock().unwrap() = preset_id;
        }
    }
    fn manage_pending_custom_preset(&self, preset: Option<Preset>) {
        if let Some(state) = self.try_state::<PendingState>() {
            *state.custom_preset.lock().unwrap() = preset;
        }
    }
    fn manage_pending_merge(&self, merge: bool) {
        if let Some(state) = self.try_state::<PendingState>() {
            *state.merge.lock().unwrap() = merge;
        }
    }
    fn manage_pending_grayscale(&self, grayscale: bool) {
        if let Some(state) = self.try_state::<PendingState>() {
            *state.grayscale.lock().unwrap() = grayscale;
        }
    }
    fn manage_pending_compare(&self, v: bool) {
        if let Some(state) = self.try_state::<PendingState>() {
            *state.compare.lock().unwrap() = v;
        }
    }
    fn manage_pending_overlay(&self, v: bool) {
        if let Some(state) = self.try_state::<PendingState>() {
            *state.overlay.lock().unwrap() = v;
        }
    }
    fn manage_pending_trim_dialog(&self, v: bool) {
        if let Some(state) = self.try_state::<PendingState>() {
            *state.trim_dialog.lock().unwrap() = v;
        }
    }
}

#[tauri::command]
pub fn get_pending_merge(state: tauri::State<'_, PendingState>) -> bool {
    *state.merge.lock().unwrap()
}

#[tauri::command]
pub fn get_pending_grayscale(state: tauri::State<'_, PendingState>) -> bool {
    *state.grayscale.lock().unwrap()
}

#[tauri::command]
pub fn get_pending_compare(state: tauri::State<'_, PendingState>) -> bool {
    *state.compare.lock().unwrap()
}

#[tauri::command]
pub fn get_pending_overlay(state: tauri::State<'_, PendingState>) -> bool {
    *state.overlay.lock().unwrap()
}

#[tauri::command]
pub fn get_pending_trim_dialog(state: tauri::State<'_, PendingState>) -> bool {
    *state.trim_dialog.lock().unwrap()
}

#[tauri::command]
pub fn get_pending_files(state: tauri::State<'_, PendingState>) -> Vec<String> {
    state.files.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_pending_preset_id(state: tauri::State<'_, PendingState>) -> Option<String> {
    state.preset_id.lock().unwrap().clone()
}

#[tauri::command]
pub fn get_pending_custom_preset(state: tauri::State<'_, PendingState>) -> Option<Preset> {
    state.custom_preset.lock().unwrap().clone()
}

/// Stash files + custom preset in app state ahead of navigating the
/// current webview to the progress route. Does NOT open a new window —
/// the Custom dialog reuses its own webview for the progress UI because
/// opening a second webview in the same Tauri instance is unreliable on
/// Windows WebView2 (blank second window). Returns immediately.
#[tauri::command]
pub fn prepare_custom_encode(
    app: tauri::AppHandle,
    files: Vec<String>,
    preset: Preset,
) -> Result<(), String> {
    app.manage_pending_files(files);
    app.manage_pending_preset(None);
    app.manage_pending_custom_preset(Some(preset));
    Ok(())
}

/// Stash files in app state ahead of the Trim dialog navigating its own
/// webview to /progress/. Persists the user's chosen frame counts plus
/// the optional middle-cut range to `trim_last.json` so the dialog
/// reopens with them next time. Mirrors `prepare_custom_encode`'s "no
/// second window" approach to dodge the Windows WebView2
/// blank-second-window bug.
#[tauri::command]
pub fn prepare_trim_encode(
    app: tauri::AppHandle,
    files: Vec<String>,
    start_frames: u32,
    end_frames: u32,
    remove_from: Option<u32>,
    remove_to: Option<u32>,
) -> Result<(), String> {
    app.manage_pending_files(files);
    app.manage_pending_preset(None);
    app.manage_pending_custom_preset(None);
    let _ = presets::save_trim_last(&TrimLast {
        start_frames,
        end_frames,
        remove_from,
        remove_to,
    });
    Ok(())
}
