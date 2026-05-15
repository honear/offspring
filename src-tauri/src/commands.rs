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
    /// Human-readable reason when `found` is false. Populated from the
    /// `resolve_ffmpeg` error so the Settings pane can show *why* the
    /// resolution failed (e.g. "you set a path that isn't ffmpeg.exe")
    /// instead of a generic "not found" — important when the user has
    /// an explicit override that turns out to be invalid.
    pub error: Option<String>,
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
///
/// Studio build: this still routes through bootstrap.rs, but the
/// studio version of `spawn_download` emits an error event and bails.
/// The frontend hides the "Download FFmpeg" button in studio anyway
/// (see `get_build_variant`), so users shouldn't see this fire.
#[tauri::command]
pub fn download_ffmpeg(app: tauri::AppHandle) -> Result<(), String> {
    bootstrap::spawn_download(app);
    Ok(())
}

/// Returns the build variant — "standard" or "studio" — so the UI
/// can hide capabilities that aren't present in the running binary.
/// This is the runtime signal that pairs with the Cargo `studio`
/// feature flag: standard binaries return "standard"; binaries
/// compiled with `--features studio` return "studio".
#[tauri::command]
pub fn get_build_variant() -> &'static str {
    if cfg!(feature = "studio") { "studio" } else { "standard" }
}

#[tauri::command]
pub fn ffmpeg_status() -> FfmpegStatus {
    let s = presets::load_settings().unwrap_or_default();
    match ffmpeg::resolve_ffmpeg(&s) {
        Ok(p) => FfmpegStatus {
            found: true,
            path: Some(p.display().to_string()),
            error: None,
        },
        Err(e) => FfmpegStatus {
            found: false,
            path: None,
            error: Some(e.to_string()),
        },
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
#[cfg(windows)]
#[tauri::command]
pub fn restart_explorer() -> Result<(), String> {
    integration::modern_menu::restart_explorer().map_err(|e| e.to_string())
}

/// macOS stub for restart_explorer. The modern right-click menu
/// concept doesn't exist on macOS (no MSIX, no Explorer handler
/// cache to invalidate), so the frontend should never call this —
/// the studio-style get_build_variant + UI gating handles that.
/// Stub returns Ok so a stray call doesn't surface as an error.
#[cfg(not(windows))]
#[tauri::command]
pub fn restart_explorer() -> Result<(), String> {
    Ok(())
}

/// In-app "Set up Windows 11 modern menu" — for users who unchecked
/// the optional modern-menu component at install time, or who are a
/// second user on a shared PC.
///
/// Imports the shipped cert into `Cert:\CurrentUser\TrustedPeople`,
/// then registers whichever MSIX package(s) the current split-layout
/// setting calls for. No admin rights — everything is user-scope.
/// The frontend should call `restart_explorer` afterwards (gated on
/// the usual confirm dialog) so the entries appear immediately.
#[cfg(windows)]
#[tauri::command]
pub fn setup_modern_menu() -> Result<(), String> {
    integration::modern_menu::trust_cert_user_scope().map_err(|e| e.to_string())?;
    let presets_list = presets::load_presets().unwrap_or_default();
    let mut settings = presets::load_settings().unwrap_or_default();
    // Force-enable the modern-menu setting so the next save_settings or
    // sync call won't immediately tear down what we just registered.
    // Persisting it here means a restart picks up the right state too.
    if settings.modern_menu_enabled != Some(true) {
        settings.modern_menu_enabled = Some(true);
        let _ = presets::save_settings(&settings);
    }
    integration::modern_menu::sync(&presets_list, &settings).map_err(|e| e.to_string())
}

/// macOS stub for setup_modern_menu. Same reasoning as
/// restart_explorer — no Windows-style MSIX modern menu exists on
/// Mac. The Mac-side equivalent (NSServices) is registered via
/// Info.plist at install time, not at runtime through this API.
#[cfg(not(windows))]
#[tauri::command]
pub fn setup_modern_menu() -> Result<(), String> {
    Err("The Windows 11 modern menu is Windows-only. On macOS, Offspring registers a Services entry via Info.plist at install time.".into())
}

/// Resolve `%SystemRoot%\explorer.exe`, falling back to the bare
/// name on systems where `SystemRoot` is scrubbed. Used by the
/// folder-opening commands; pulled out so we don't repeat the
/// path-planting defense logic for each new entry.
#[cfg(windows)]
fn explorer_path() -> std::path::PathBuf {
    match std::env::var_os("SystemRoot") {
        Some(root) => {
            let candidate = std::path::PathBuf::from(root).join("explorer.exe");
            if candidate.exists() {
                candidate
            } else {
                std::path::PathBuf::from("explorer.exe")
            }
        }
        None => std::path::PathBuf::from("explorer.exe"),
    }
}

/// Open the user's Offspring data folder in the OS's native file
/// browser. Windows: spawn Explorer. macOS: spawn `open` (which uses
/// Finder by default). Same UX outcome — a window appears showing
/// the contents of `%APPDATA%\Offspring` / `~/Library/Application Support/Offspring`.
#[tauri::command]
pub fn open_data_folder() -> Result<(), String> {
    let p = paths::data_dir().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        std::process::Command::new(explorer_path())
            .arg(&p)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("/usr/bin/open")
            .arg(&p)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Open the folder containing the debug log (`%LOCALAPPDATA%\Offspring`)
/// in Explorer with `debug.log` selected. The `/select,<path>` switch
/// asks Explorer to open the parent and highlight the file rather
/// than just navigating to the directory — gives the user a
/// one-click path to "show me the log so I can copy or delete it".
///
/// If the log file doesn't exist yet (no encode has logged anything)
/// we fall back to opening the parent directory, otherwise Explorer
/// shows an empty/default view.
/// Open the folder containing the debug log
/// (`%LOCALAPPDATA%\Offspring` on Windows, `~/Library/Application Support/Offspring`
/// on macOS) in the native file browser with `debug.log` selected
/// when it exists. Windows uses Explorer's `/select,<path>` switch;
/// macOS uses `open -R <path>` (Reveal in Finder) for the same UX.
#[tauri::command]
pub fn open_log_folder() -> Result<(), String> {
    let log_path = crate::debug_log::log_path()
        .ok_or_else(|| "Could not resolve debug log path.".to_string())?;
    let parent_dir = log_path
        .parent()
        .ok_or_else(|| "Log path has no parent directory.".to_string())?;
    // Make sure the parent exists before asking the OS to open it -
    // `local_data_dir()` already creates Offspring/ but the folder
    // could still be missing if the user wiped it manually.
    let _ = std::fs::create_dir_all(parent_dir);

    #[cfg(windows)]
    {
        let exe = explorer_path();
        if log_path.exists() {
            // /select,<path> highlights the file in its parent folder.
            // The leading comma is required and there's no space after
            // it - that's how Explorer parses the switch.
            std::process::Command::new(exe)
                .arg(format!("/select,{}", log_path.display()))
                .spawn()
                .map_err(|e| e.to_string())?;
        } else {
            std::process::Command::new(exe)
                .arg(parent_dir)
                .spawn()
                .map_err(|e| e.to_string())?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        // `open -R` reveals a file in Finder (highlights it in its
        // parent). Bare `open` on a directory opens the dir itself.
        if log_path.exists() {
            std::process::Command::new("/usr/bin/open")
                .args(["-R", &log_path.display().to_string()])
                .spawn()
                .map_err(|e| e.to_string())?;
        } else {
            std::process::Command::new("/usr/bin/open")
                .arg(parent_dir)
                .spawn()
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Hand off an https/http URL to the user's default browser via the
/// OS's URL association. Used for the "Second March" credit link in
/// the topbar — the `tauri-plugin-opener` JS API was silently failing
/// in WebView2 even with `opener:allow-open-url` granted, and
/// debugging that turned out to be a worse use of time than just
/// doing the dispatch ourselves.
///
/// Implementation:
///   - Windows: `cmd /c start "" <url>`. The empty `""` is the window
///     title arg that `start` expects when the second token is quoted;
///     without it `start "https://…"` interprets the URL as the title
///     and opens nothing.
///   - macOS: `/usr/bin/open <url>`. macOS's URL dispatcher routes to
///     the user's default browser.
///
/// We refuse anything that isn't `http://` or `https://` so this can't
/// be coerced into running a local file or a wacky URI scheme.
#[tauri::command]
pub fn open_external_url(url: String) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(format!("refusing non-http(s) URL: {url}"));
    }
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("/usr/bin/open")
            .arg(&url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Abort any in-flight ffmpeg encode ASAP. Sets the process-wide
/// cancel flag in `ffmpeg.rs`; the next `run_with_progress` poll tick
/// (≤1s) kills the child process and bails. The encode function then
/// cleans up the partial output file via `cleanup_partial_output` so
/// the user isn't left with a 0-byte / truncated .mp4 / .gif / etc.
///
/// Called by the progress window when the user clicks ✕ during an
/// active encode (the window's close handler routes through here
/// before invoking close()). Safe to call when no encode is running —
/// the flag is just a boolean, and the next encode resets it on entry.
#[tauri::command]
pub fn cancel_encode() {
    ffmpeg::request_cancel();
}

/// Open a native file picker filtered to image formats and return the
/// chosen path (or `None` if the user cancelled). Used by the Overlay
/// tool's "Add watermark" toggle so users get a real Explorer file
/// browser instead of having to paste a path.
///
/// Filter set covers the formats `encode_file`'s watermark step can
/// actually decode with alpha: PNG, WebP, TIFF, AVIF. JPEG is omitted
/// on purpose — it has no alpha channel, so picking one would produce
/// a solid-rectangle watermark instead of a logo cutout.
#[tauri::command]
pub async fn pick_watermark_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .add_filter("Watermark (with alpha)", &["png", "webp", "tif", "tiff", "avif"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    let picked = rx.recv().map_err(|e| e.to_string())?;
    Ok(picked.and_then(|p| p.into_path().ok().map(|pb| pb.to_string_lossy().into_owned())))
}

#[tauri::command]
pub fn encode(
    app: tauri::AppHandle,
    files: Vec<String>,
    preset: Preset,
) -> Result<(), String> {
    ffmpeg::reset_cancel();
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
    // Refuse image-only selections. Concatenating stills isn't well-
    // defined (strip? grid? animation?) and is best left to the
    // Compare tool (horizontal stack) or future dedicated tools. If
    // the selection mixes images and videos we let it through — the
    // video path can handle stills as one-frame inputs.
    if files
        .iter()
        .all(|f| ffmpeg::is_image_path(std::path::Path::new(f)))
    {
        return Err(
            "Merge concatenates videos in time, which doesn't apply to \
             still images. Try the Compare tool to stack images side-by-\
             side instead."
                .into(),
        );
    }
    ffmpeg::reset_cancel();
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
    ffmpeg::reset_cancel();
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
    ffmpeg::reset_cancel();
    let settings = presets::load_settings().unwrap_or_default();
    let ffmpeg_path = ffmpeg::resolve_ffmpeg(&settings).map_err(|e| e.to_string())?;
    let mut paths: Vec<std::path::PathBuf> =
        files.iter().map(std::path::PathBuf::from).collect();
    // Sort by filename so output order is predictable regardless of
    // Explorer's click-order. Users naturally expect v01 → v02 → v03
    // to stack left-to-right in that order, not whatever sequence
    // Explorer happened to enumerate them in. Case-insensitive so
    // "Clip_A.mp4" and "clip_b.mp4" sort together cleanly.
    paths.sort_by_key(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });

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

/// Compare-grid tool: arrange N>=3 clips into a `cols`-wide grid
/// using either Grid (preserve aspect, pad cells) or Mosaic (scale +
/// crop to fill cells) layout. Routed to from the right-click Compare
/// entry when the selection is 3+ files — the grid dialog window
/// opens, the user picks `cols` + `layout`, and the dialog navigates
/// to /progress/ with those params baked into the URL.
///
/// The 2-file case still goes through `encode_compare` (no dialog) so
/// the historical side-by-side behavior is byte-for-byte unchanged.
#[tauri::command]
pub fn encode_compare_grid(
    app: tauri::AppHandle,
    files: Vec<String>,
    cols: u32,
    layout: String,
) -> Result<(), String> {
    if files.len() < 2 {
        return Err("Compare grid needs at least two files".into());
    }
    let cols = cols.max(1).min(files.len() as u32);
    let layout_enum = match layout.as_str() {
        "mosaic" => ffmpeg::GridLayout::Mosaic,
        _ => ffmpeg::GridLayout::Grid,
    };
    ffmpeg::reset_cancel();
    let settings = presets::load_settings().unwrap_or_default();
    let ffmpeg_path = ffmpeg::resolve_ffmpeg(&settings).map_err(|e| e.to_string())?;
    let mut paths: Vec<std::path::PathBuf> =
        files.iter().map(std::path::PathBuf::from).collect();
    // Sort by filename — see encode_compare for rationale. Order
    // matters MORE in the grid because the cells fill row-by-row
    // left-to-right, so any ambiguity in input order shows up as
    // "this clip isn't where I expected" in the output grid.
    paths.sort_by_key(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    });

    std::thread::spawn(move || {
        let app_cl = app.clone();
        let result = ffmpeg::encode_compare_grid_files(
            &ffmpeg_path,
            &paths,
            cols,
            layout_enum,
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
    ffmpeg::reset_cancel();
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
                // Watermark fields. derive_overlay_preset checks the
                // path exists on disk and probes the clip's dimensions
                // to bake into the filter; if any check fails the
                // resulting Preset.watermark stays None and the
                // overlay encode skips the watermark step silently.
                watermark_enabled: ot.watermark_enabled,
                watermark_path: ot.watermark_path.clone(),
                watermark_opacity: (ot.watermark_opacity.min(100) as f32) / 100.0,
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

/// Invert tool: per-image color invert with optional binary clamp.
/// Image-only — refuses video inputs. The `clamp` setting comes from
/// `settings.tools.invert.clamp` so the right-click flow doesn't need
/// to ferry it through the CLI / IPC layer.
#[tauri::command]
pub fn encode_invert(
    app: tauri::AppHandle,
    files: Vec<String>,
) -> Result<(), String> {
    if files.is_empty() {
        return Err("Invert needs at least one file".into());
    }
    ffmpeg::reset_cancel();
    let settings = presets::load_settings().unwrap_or_default();
    let ffmpeg_path = ffmpeg::resolve_ffmpeg(&settings).map_err(|e| e.to_string())?;
    let clamp = settings.tools.invert.clamp;
    let paths_in: Vec<std::path::PathBuf> =
        files.iter().map(std::path::PathBuf::from).collect();
    let total = paths_in.len();

    std::thread::spawn(move || {
        let app_cl = app.clone();
        let result = ffmpeg::encode_invert_files(
            &ffmpeg_path,
            &paths_in,
            clamp,
            &settings,
            move |ev: ProgressEvent| {
                let _ = app_cl.emit("encode-progress", ev);
            },
        );
        if let Err(e) = result {
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

/// Make-Square tool: pad shorter edge of each image to match the
/// longer one. Image-only. Fill mode (transparent vs. edge-color)
/// comes from `settings.tools.make_square.fill_mode`.
#[tauri::command]
pub fn encode_make_square(
    app: tauri::AppHandle,
    files: Vec<String>,
) -> Result<(), String> {
    if files.is_empty() {
        return Err("Make Square needs at least one file".into());
    }
    ffmpeg::reset_cancel();
    let settings = presets::load_settings().unwrap_or_default();
    let ffmpeg_path = ffmpeg::resolve_ffmpeg(&settings).map_err(|e| e.to_string())?;
    let fill_mode = settings.tools.make_square.fill_mode.clone();
    let paths_in: Vec<std::path::PathBuf> =
        files.iter().map(std::path::PathBuf::from).collect();
    let total = paths_in.len();

    std::thread::spawn(move || {
        let app_cl = app.clone();
        let result = ffmpeg::encode_make_square_files(
            &ffmpeg_path,
            &paths_in,
            fill_mode,
            &settings,
            move |ev: ProgressEvent| {
                let _ = app_cl.emit("encode-progress", ev);
            },
        );
        if let Err(e) = result {
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

/// Modify tool: per-file rectangular crop + optional flip /
/// reverse / overwrite. Same set of transforms is applied to every
/// file (crop rect clamped per-file at encode time so mixed-size
/// selections don't fail).
///
/// `crop_w == 0` or `crop_h == 0` means "no crop" — encoder skips
/// the crop filter and applies only the other transforms.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn encode_modify(
    app: tauri::AppHandle,
    files: Vec<String>,
    crop_x: u32,
    crop_y: u32,
    crop_w: u32,
    crop_h: u32,
    flip_h: bool,
    flip_v: bool,
    reverse: bool,
    remove_audio: bool,
    rotate: u32,
    trim_start_sec: f32,
    trim_end_sec: f32,
    overwrite: bool,
) -> Result<(), String> {
    if files.is_empty() {
        return Err("Modify needs at least one file".into());
    }
    let crop_rect = if crop_w > 0 && crop_h > 0 {
        Some((crop_x, crop_y, crop_w, crop_h))
    } else {
        None
    };
    // Normalise rotate to one of 0/90/180/270 so the filter chain
    // never has to defend against bogus values from the frontend.
    let rotate = match rotate {
        90 | 180 | 270 => rotate,
        _ => 0,
    };
    let rotated = rotate != 0;
    // Trim is "active" iff start > 0 or end < clip duration. The
    // frontend can't know the clip duration without a probe, so it
    // sends BOTH start and end and the backend treats start == 0 +
    // end == 0 as "no trim". Negative values are nonsense → treated
    // the same. We only forward Some when the value is meaningful.
    let trim_start_opt = if trim_start_sec > 0.0 {
        Some(trim_start_sec)
    } else {
        None
    };
    let trim_end_opt = if trim_end_sec > 0.0 {
        Some(trim_end_sec)
    } else {
        None
    };
    let trimmed = trim_start_opt.is_some() || trim_end_opt.is_some();
    // At least one transform must be active or we'd be doing a
    // pointless re-encode of the source. "Remove audio", any
    // non-zero rotation, and a real trim each count on their own —
    // every one is a meaningful change the user explicitly asked
    // for. Frontend disables the button in this case but
    // defense-in-depth.
    if crop_rect.is_none() && !flip_h && !flip_v && !reverse && !remove_audio && !rotated && !trimmed
    {
        return Err(
            "Nothing to modify — pick at least one transform (crop / rotate / flip / reverse / trim / remove audio).".into(),
        );
    }
    ffmpeg::reset_cancel();
    let settings = presets::load_settings().unwrap_or_default();
    let ffmpeg_path = ffmpeg::resolve_ffmpeg(&settings).map_err(|e| e.to_string())?;
    let paths_in: Vec<std::path::PathBuf> =
        files.iter().map(std::path::PathBuf::from).collect();
    let total = paths_in.len();

    std::thread::spawn(move || {
        let app_cl = app.clone();
        let result = ffmpeg::encode_modify_files(
            &ffmpeg_path,
            &paths_in,
            ffmpeg::ModifySpec {
                crop_rect,
                flip_h,
                flip_v,
                reverse,
                rotate,
                remove_audio,
                trim_start_sec: trim_start_opt,
                trim_end_sec: trim_end_opt,
                overwrite,
            },
            &settings,
            move |ev: ProgressEvent| {
                let _ = app_cl.emit("encode-progress", ev);
            },
        );
        if let Err(e) = result {
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

/// Stash files in app state ahead of the Modify dialog navigating
/// to /progress/. Mirrors `prepare_trim_encode` — no second window
/// is opened; the dialog reuses its own webview to dodge the
/// Windows WebView2 blank-second-window bug.
#[tauri::command]
pub fn prepare_modify_encode(
    app: tauri::AppHandle,
    files: Vec<String>,
) -> Result<(), String> {
    app.manage_pending_files(files);
    app.manage_pending_preset(None);
    app.manage_pending_custom_preset(None);
    Ok(())
}

/// Stash files for the Compare-grid dialog ahead of its progress-page
/// navigation. Same contract as `prepare_modify_encode`: store the
/// file list in app state so `get_pending_files` from the progress
/// route can read it after the dialog's `goto("/progress/?…")` call.
#[tauri::command]
pub fn prepare_compare_grid_encode(
    app: tauri::AppHandle,
    files: Vec<String>,
) -> Result<(), String> {
    app.manage_pending_files(files);
    app.manage_pending_preset(None);
    app.manage_pending_custom_preset(None);
    Ok(())
}

/// Probe (width, height) of a single file. Used by the Crop dialog
/// to set up display ↔ source pixel coordinate mapping. Returns
/// `None` when the file can't be probed; the dialog treats that as
/// an unsupported-input error.
#[tauri::command]
pub fn probe_dimensions(path: String) -> Result<Option<(u32, u32)>, String> {
    ffmpeg::reset_cancel();
    let settings = presets::load_settings().unwrap_or_default();
    let ffmpeg_path = ffmpeg::resolve_ffmpeg(&settings).map_err(|e| e.to_string())?;
    Ok(ffmpeg::probe_dimensions(&ffmpeg_path, std::path::Path::new(&path)))
}

/// Extract a preview frame at `time_seconds` to a JPEG in the app's
/// temp dir, returning the path. The Crop dialog calls this when its
/// `<video>` element fails to decode the source natively (ProRes,
/// DNxHD, weird MKVs) — falls back to displaying the still.
///
/// Cleanup: the temp file gets a per-process-id-and-nonce name and
/// lives in `%LOCALAPPDATA%\Offspring\tmp\`. The OS reaps the dir on
/// reboot via standard temp-cleanup; we don't bother tracking
/// individual files because they're tiny and short-lived.
#[tauri::command]
pub fn extract_preview_frame(
    path: String,
    time_seconds: f64,
) -> Result<String, String> {
    ffmpeg::reset_cancel();
    let settings = presets::load_settings().unwrap_or_default();
    let ffmpeg_path = ffmpeg::resolve_ffmpeg(&settings).map_err(|e| e.to_string())?;

    let tmp_dir = paths::tmp_dir().map_err(|e| e.to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let out = tmp_dir.join(format!(
        "preview-{}-{nonce}.jpg",
        std::process::id()
    ));

    ffmpeg::extract_preview_frame(
        &ffmpeg_path,
        std::path::Path::new(&path),
        time_seconds,
        &out,
    )
    .map_err(|e| e.to_string())?;

    Ok(out.to_string_lossy().into_owned())
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
    ffmpeg::reset_cancel();
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
    //
    // `.visible(false)` builds the OS window hidden so WebView2 has time
    // to fetch + execute the Svelte bundle before anything appears on
    // screen. The progress route calls `getCurrentWindow().show()` from
    // its `onMount` once Svelte has rendered its first frame, killing
    // the brief blank-window flash that used to precede the encoder UI.
    let mut b = WebviewWindowBuilder::new(app, "progress", WebviewUrl::App("progress/".into()))
        .title("Offspring — Encoding")
        .inner_size(pw, ph)
        .resizable(false)
        .always_on_top(true)
        .decorations(true)
        .visible(false);
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
        .resizable(true)
        // See `open_progress_window` for why this is hidden-by-default.
        // The Custom route reveals itself in `onMount` after first paint.
        .visible(false);
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
/// The window is built invisible so the Svelte route mounts before the
/// OS window appears (avoids a blank-frame flash). The frontend's
/// `onMount` shows it AND performs the always-on-top → set_focus →
/// always-on-top-off focus dance — when the shell-extension spawns
/// offspring.exe from Explorer, Explorer keeps the foreground by
/// default and the new dialog ends up behind it. Briefly toggling
/// `always_on_top` bypasses Windows' focus-stealing prevention long
/// enough for `set_focus` to actually move the foreground to us; we
/// drop always-on-top right away so the user can later put another
/// window over the dialog if they want to.
/// Open the Modify mini dialog. Bigger than Trim/Custom because it
/// has to fit a media preview plus the crop overlay UI plus the
/// numeric inputs plus the transform toggles. Resizable so users
/// with high-DPI displays or large source media can scale up. The
/// window is built invisible so the Svelte route mounts before the
/// OS window appears (avoids a blank-frame flash); the frontend
/// reveals it after first paint.
pub fn open_modify_window(app: &tauri::AppHandle, files: Vec<String>) -> anyhow::Result<()> {
    let (pw, ph) = (920.0, 760.0);
    let mut b = WebviewWindowBuilder::new(app, "modify", WebviewUrl::App("modify/".into()))
        .title("Offspring — Modify")
        .inner_size(pw, ph)
        .min_inner_size(640.0, 520.0)
        .resizable(true)
        .focused(true)
        .visible(false);
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

pub fn open_trim_window(app: &tauri::AppHandle, files: Vec<String>) -> anyhow::Result<()> {
    let (pw, ph) = (440.0, 420.0);
    let mut b = WebviewWindowBuilder::new(app, "trim", WebviewUrl::App("trim/".into()))
        .title("Offspring — Trim")
        .inner_size(pw, ph)
        .min_inner_size(380.0, 320.0)
        .resizable(true)
        .focused(true)
        .visible(false);
    if let Some((x, y)) = position_near_cursor(app, pw, ph) {
        b = b.position(x, y);
    }
    let w = b.build()?;
    app.manage_pending_files(files);
    let _ = w; // frontend handles show() + focus dance after first paint
    Ok(())
}

/// Compare-grid dialog window. Opened by `merge_pending` when the user
/// invokes Compare on 3+ files (the 2-file case skips the dialog and
/// goes straight to progress — see `open_window_for_pending`). Lives
/// at /compare-grid/ in the SPA; the user picks `cols` + `layout`
/// there, the dialog calls `prepare_compare_grid_encode` to stash the
/// files in app state, then navigates this same webview to /progress/
/// with the params baked into the URL.
pub fn open_compare_grid_window(app: &tauri::AppHandle, files: Vec<String>) -> anyhow::Result<()> {
    let (pw, ph) = (440.0, 400.0);
    let mut b = WebviewWindowBuilder::new(
        app,
        "compare-grid",
        WebviewUrl::App("compare-grid/".into()),
    )
        .title("Offspring — Compare Grid")
        .inner_size(pw, ph)
        .min_inner_size(380.0, 280.0)
        .resizable(true)
        .focused(true)
        .visible(false);
    if let Some((x, y)) = position_near_cursor(app, pw, ph) {
        b = b.position(x, y);
    }
    let w = b.build()?;
    app.manage_pending_files(files);
    let _ = w; // frontend handles show() + focus dance after first paint
    Ok(())
}

pub fn open_main_window(app: &tauri::AppHandle) -> anyhow::Result<()> {
    let (pw, ph) = (880.0, 820.0);
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
    /// True when the progress window should route to `encode_invert`.
    pub invert: std::sync::Mutex<bool>,
    /// True when the progress window should route to `encode_make_square`.
    pub make_square: std::sync::Mutex<bool>,
    /// True when the user invoked `offspring modify <files>` and
    /// the Modify dialog should open. Mirrors `trim_dialog` — the
    /// dialog reads pending files, lets the user pick transforms
    /// (crop / flip / reverse / overwrite), then drives
    /// `prepare_modify_encode` + navigates to /progress/ in-place.
    pub modify_dialog: std::sync::Mutex<bool>,
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
    fn manage_pending_invert(&self, v: bool);
    fn manage_pending_make_square(&self, v: bool);
    fn manage_pending_modify_dialog(&self, v: bool);
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
    fn manage_pending_invert(&self, v: bool) {
        if let Some(state) = self.try_state::<PendingState>() {
            *state.invert.lock().unwrap() = v;
        }
    }
    fn manage_pending_make_square(&self, v: bool) {
        if let Some(state) = self.try_state::<PendingState>() {
            *state.make_square.lock().unwrap() = v;
        }
    }
    fn manage_pending_modify_dialog(&self, v: bool) {
        if let Some(state) = self.try_state::<PendingState>() {
            *state.modify_dialog.lock().unwrap() = v;
        }
    }
}

#[tauri::command]
pub fn get_pending_modify_dialog(state: tauri::State<'_, PendingState>) -> bool {
    *state.modify_dialog.lock().unwrap()
}

#[tauri::command]
pub fn get_pending_invert(state: tauri::State<'_, PendingState>) -> bool {
    *state.invert.lock().unwrap()
}

#[tauri::command]
pub fn get_pending_make_square(state: tauri::State<'_, PendingState>) -> bool {
    *state.make_square.lock().unwrap()
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
