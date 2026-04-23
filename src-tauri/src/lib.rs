mod bootstrap;
mod cli;
mod commands;
mod defaults;
mod ffmpeg;
mod integration;
mod paths;
mod presets;
mod updates;

use clap::Parser;
use cli::{Cli, Command};
use commands::{AppHandleExt, PendingState};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args = Cli::parse();

    // Headless fast-paths — do not start the Tauri event loop.
    match &args.command {
        Some(Command::FirstRun) => {
            let ps = presets::load_presets().unwrap_or_else(|_| defaults::default_presets());
            let _ = presets::save_presets(&ps);
            let settings = presets::load_settings().unwrap_or_default();
            let _ = integration::sync_all(&ps, &settings);
            return;
        }
        Some(Command::Cleanup) => {
            let _ = integration::cleanup_all();
            return;
        }
        _ => {}
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(PendingState::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_presets,
            commands::save_presets,
            commands::reset_presets_to_defaults,
            commands::get_settings,
            commands::save_settings,
            commands::ffmpeg_status,
            commands::download_ffmpeg,
            commands::get_custom_last,
            commands::save_custom_last,
            commands::sync_integrations,
            commands::open_data_folder,
            commands::encode,
            commands::get_pending_files,
            commands::get_pending_preset_id,
            commands::get_pending_custom_preset,
            commands::prepare_custom_encode,
            updates::check_for_updates,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            // First-run-if-needed: seed defaults + sync SendTo when
            // the presets file does not exist yet.
            if let Ok(path) = paths::presets_path() {
                if !path.exists() {
                    let ps = defaults::default_presets();
                    let _ = presets::save_presets(&ps);
                    let settings = presets::load_settings().unwrap_or_default();
                    let _ = integration::sync_all(&ps, &settings);
                }
            }

            match args.command {
                Some(Command::Preset { id, files }) => {
                    let strs: Vec<String> =
                        files.iter().map(|p| p.display().to_string()).collect();
                    handle.manage_pending_files(strs);
                    handle.manage_pending_preset(Some(id));
                    commands::open_progress_window(&handle)?;
                }
                Some(Command::Custom { files }) => {
                    let strs: Vec<String> =
                        files.iter().map(|p| p.display().to_string()).collect();
                    commands::open_custom_window(&handle, strs)?;
                }
                _ => {
                    commands::open_main_window(&handle)?;
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
