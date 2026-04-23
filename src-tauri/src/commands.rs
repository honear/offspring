use serde::Serialize;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::bootstrap;
use crate::defaults;
use crate::ffmpeg::{self, ProgressEvent};
use crate::integration;
use crate::paths;
use crate::presets::{self, Preset, Settings};

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
    std::process::Command::new("explorer")
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
    let total = files.len();

    std::thread::spawn(move || {
        for (i, f) in files.iter().enumerate() {
            let input = std::path::PathBuf::from(f);
            let duration = ffmpeg::probe_duration(&ffmpeg_path, &input);
            let app_cl = app.clone();
            let result = ffmpeg::encode_file(
                &ffmpeg_path,
                &input,
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
                        input: f.clone(),
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

pub fn open_progress_window(app: &tauri::AppHandle) -> anyhow::Result<()> {
    // Trailing slash is important: svelte.config uses
    // `trailingSlash: 'always'` so each route is prerendered to
    // `build/<route>/index.html`, and the URL that SvelteKit's client router
    // normalises against its registered routes is `/progress/` — NOT
    // `/progress.html`, which 404s on the router even though the file is
    // present on disk.
    WebviewWindowBuilder::new(app, "progress", WebviewUrl::App("progress/".into()))
        .title("Offspring — Encoding")
        .inner_size(380.0, 140.0)
        .resizable(false)
        .always_on_top(true)
        .decorations(true)
        .build()?;
    Ok(())
}

pub fn open_custom_window(app: &tauri::AppHandle, files: Vec<String>) -> anyhow::Result<()> {
    let w = WebviewWindowBuilder::new(app, "custom", WebviewUrl::App("custom/".into()))
        .title("Offspring — Custom")
        .inner_size(500.0, 520.0)
        .min_inner_size(460.0, 440.0)
        .resizable(true)
        .build()?;
    // Files are read by the frontend via a Tauri command.
    // Stash them in app state.
    app.manage_pending_files(files);
    let _ = w; // suppress unused
    Ok(())
}

pub fn open_main_window(app: &tauri::AppHandle) -> anyhow::Result<()> {
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("".into()))
        .title("Offspring")
        .inner_size(880.0, 760.0)
        .min_inner_size(720.0, 460.0)
        .resizable(true)
        .build()?;
    Ok(())
}

// App state for pending CLI inputs
#[derive(Default)]
pub struct PendingState {
    pub files: std::sync::Mutex<Vec<String>>,
    pub preset_id: std::sync::Mutex<Option<String>>,
    pub custom_preset: std::sync::Mutex<Option<Preset>>,
}

pub trait AppHandleExt {
    fn manage_pending_files(&self, files: Vec<String>);
    fn manage_pending_preset(&self, preset_id: Option<String>);
    fn manage_pending_custom_preset(&self, preset: Option<Preset>);
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
