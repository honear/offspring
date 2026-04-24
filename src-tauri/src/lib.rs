mod bootstrap;
mod cli;
mod commands;
mod debug_log;
mod defaults;
mod ffmpeg;
mod integration;
mod paths;
mod presets;
mod sequence;
mod updates;

use clap::Parser;
use cli::{Cli, Command};
use commands::{AppHandleExt, PendingState};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::Manager;

/// Rolling debounce window. Each incoming CLI arrival resets the deadline;
/// the window is only opened after this much quiet time. Tuned by ear —
/// long enough that Explorer's per-file process spawn (seen at ~30–80 ms
/// per file on SSDs) doesn't miss the end of a batch, short enough not to
/// feel laggy on a single-file click.
const BATCH_DEBOUNCE: Duration = Duration::from_millis(500);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let raw_argv: Vec<String> = std::env::args().collect();
    dlog!("run() enter; argv={:?}", raw_argv);

    let args = Cli::parse();
    dlog!("CLI parsed; command={:?}", args.command);

    // Headless fast-paths — do not start the Tauri event loop. These
    // complete synchronously and exit before single-instance / debounce
    // machinery would matter.
    match &args.command {
        Some(Command::FirstRun) => {
            dlog!("FirstRun fast-path");
            let ps = presets::load_presets().unwrap_or_else(|_| defaults::default_presets());
            let _ = presets::save_presets(&ps);
            let settings = presets::load_settings().unwrap_or_default();
            let _ = integration::sync_all(&ps, &settings);
            return;
        }
        Some(Command::Cleanup) => {
            dlog!("Cleanup fast-path");
            let _ = integration::cleanup_all();
            return;
        }
        _ => {}
    }

    // Activity clock: updated whenever a new CLI-arg bundle arrives
    // (primary start OR secondary instance forwarded by the plugin).
    // The debounce watcher in setup() waits until this has been quiet
    // for BATCH_DEBOUNCE before opening a window.
    let last_arrival: Arc<Mutex<Instant>> = Arc::new(Mutex::new(Instant::now()));
    // Marks when the debounce watcher last opened a window. The
    // single-instance callback uses this to detect a NEW batch (vs.
    // additional files trickling into the current batch) and clear
    // stale pending state before the new files land — otherwise a
    // second right-click would accumulate files from the prior batch
    // and re-process them.
    let last_dispatch: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let initial_command = args.command.clone();

    tauri::Builder::default()
        // Single-instance must be registered first — the plugin hooks the
        // secondary's entry point and short-circuits it before anything
        // else runs. Without this, every right-clicked file would spawn
        // its own offspring.exe + its own progress window (the root
        // cause of "multi-select only processes one file" before v0.3.30).
        .plugin(tauri_plugin_single_instance::init({
            let last_arrival = last_arrival.clone();
            let last_dispatch = last_dispatch.clone();
            move |app, argv, _cwd| {
                dlog!("single-instance callback: secondary argv={:?}", argv);

                // New-batch detection: if the last window dispatch was
                // more than BATCH_DEBOUNCE * 2 ago, assume this arrival
                // begins a fresh user action (a new right-click, not a
                // still-trickling-in multi-file batch). Clear stale
                // pending state so the previous batch's files don't
                // ride along.
                let is_new_batch = {
                    let guard = last_dispatch.lock().unwrap();
                    match *guard {
                        Some(d) => d.elapsed() > BATCH_DEBOUNCE * 2,
                        None => false, // primary's first arrival — state already seeded by setup()
                    }
                };
                if is_new_batch {
                    dlog!("  new batch detected; clearing stale pending state");
                    if let Some(state) = app.try_state::<PendingState>() {
                        state.files.lock().unwrap().clear();
                        *state.preset_id.lock().unwrap() = None;
                        *state.merge.lock().unwrap() = false;
                        *state.compare.lock().unwrap() = false;
                        *state.grayscale.lock().unwrap() = false;
                        *state.overlay.lock().unwrap() = false;
                    }
                }

                // Parse the secondary's argv using the same clap spec
                // the primary used. `try_parse_from` tolerates malformed
                // secondary calls (e.g. missing --id) without panicking.
                match Cli::try_parse_from(&argv) {
                    Ok(cli) => {
                        dlog!("  secondary parsed; command={:?}", cli.command);
                        merge_pending(app, cli.command);
                        *last_arrival.lock().unwrap() = Instant::now();
                    }
                    Err(e) => {
                        dlog!("  secondary PARSE FAILED: {}", e);
                    }
                }
            }
        }))
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
            commands::restart_explorer,
            commands::open_data_folder,
            commands::encode,
            commands::encode_merge,
            commands::encode_grayscale,
            commands::encode_compare,
            commands::encode_overlay,
            commands::get_pending_files,
            commands::get_pending_preset_id,
            commands::get_pending_custom_preset,
            commands::get_pending_merge,
            commands::get_pending_grayscale,
            commands::get_pending_compare,
            commands::get_pending_overlay,
            commands::prepare_custom_encode,
            updates::check_for_updates,
            updates::download_update,
            updates::install_update,
        ])
        .setup(move |app| {
            dlog!("setup() enter (primary)");
            let handle = app.handle().clone();

            // First-run-if-needed: seed defaults + sync right-click menus
            // when the presets file does not exist yet.
            if let Ok(path) = paths::presets_path() {
                if !path.exists() {
                    dlog!("first-run: seeding defaults + sync_all");
                    let ps = defaults::default_presets();
                    let _ = presets::save_presets(&ps);
                    let settings = presets::load_settings().unwrap_or_default();
                    let _ = integration::sync_all(&ps, &settings);
                }
            }

            // Seed pending state with the primary's own command and start
            // the debounce clock. If this is a multi-file right-click,
            // more args will land in merge_pending() via the
            // single-instance callback before the window opens.
            merge_pending(&handle, initial_command.clone());
            *last_arrival.lock().unwrap() = Instant::now();

            // Long-lived debounce watcher: waits (cheap sleeps) for any
            // new CLI arrival newer than our last dispatch, then debounces
            // BATCH_DEBOUNCE of quiet before hopping to the main event-loop
            // thread to open a window. Stays alive for the life of the
            // process so that future secondary arrivals (e.g. a later
            // right-click when the primary is already running but idle)
            // still trigger a window open.
            //
            // Tauri 2's `WebviewWindowBuilder::build()` silently no-ops
            // when called from an arbitrary worker thread — that's how
            // 0.3.30 shipped "no window opens, not even for single
            // files." `run_on_main_thread` fixes that.
            let handle_for_thread = handle.clone();
            let last_arrival_for_thread = last_arrival.clone();
            let last_dispatch_for_thread = last_dispatch.clone();
            std::thread::spawn(move || {
                dlog!("debounce thread spawned (long-lived)");
                loop {
                    // Wait for an arrival newer than our last dispatch.
                    loop {
                        let arrival = *last_arrival_for_thread.lock().unwrap();
                        let last = *last_dispatch_for_thread.lock().unwrap();
                        let needs_dispatch = match last {
                            None => true,
                            Some(d) => arrival > d,
                        };
                        if needs_dispatch {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    dlog!("debounce: new arrival detected, starting quiet wait");

                    // Wait until BATCH_DEBOUNCE of quiet has elapsed.
                    loop {
                        let elapsed = {
                            let guard = last_arrival_for_thread.lock().unwrap();
                            guard.elapsed()
                        };
                        if elapsed >= BATCH_DEBOUNCE {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    dlog!("debounce elapsed; dispatching to main thread");

                    let handle_for_main = handle_for_thread.clone();
                    match handle_for_thread.run_on_main_thread(move || {
                        dlog!("on main thread; calling open_window_for_pending");
                        open_window_for_pending(&handle_for_main);
                    }) {
                        Ok(_) => dlog!("run_on_main_thread dispatch OK"),
                        Err(e) => dlog!("run_on_main_thread dispatch FAILED: {}", e),
                    }
                    *last_dispatch_for_thread.lock().unwrap() = Some(Instant::now());
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            dlog!("tauri run() FAILED: {:#}", e);
            panic!("error while running tauri application: {e}");
        });
    dlog!("tauri run() returned (event loop exited cleanly)");
}

/// Fold an incoming `Command` into the shared `PendingState`. Called from
/// both the primary's initial setup() and every secondary that the
/// single-instance plugin forwards. File lists APPEND rather than
/// replace so a multi-file right-click ends up with the full selection
/// by the time the debounce fires.
fn merge_pending(handle: &tauri::AppHandle, cmd: Option<Command>) {
    let Some(cmd) = cmd else {
        dlog!("merge_pending: no command (noop)");
        return;
    };
    let Some(state) = handle.try_state::<PendingState>() else {
        dlog!("merge_pending: PendingState not yet managed (early secondary?)");
        return;
    };

    match cmd {
        Command::Preset { id, files } => {
            dlog!("merge_pending: Preset id={} +files={}", id, files.len());
            append_files(&state, files);
            *state.preset_id.lock().unwrap() = Some(id);
        }
        Command::Merge { files } => {
            dlog!("merge_pending: Merge +files={}", files.len());
            append_files(&state, files);
            *state.merge.lock().unwrap() = true;
        }
        Command::Grayscale { files } => {
            dlog!("merge_pending: Grayscale +files={}", files.len());
            append_files(&state, files);
            *state.grayscale.lock().unwrap() = true;
        }
        Command::Compare { files } => {
            dlog!("merge_pending: Compare +files={}", files.len());
            append_files(&state, files);
            *state.compare.lock().unwrap() = true;
        }
        Command::Overlay { files } => {
            dlog!("merge_pending: Overlay +files={}", files.len());
            append_files(&state, files);
            *state.overlay.lock().unwrap() = true;
        }
        Command::Custom { files } => {
            dlog!("merge_pending: Custom +files={}", files.len());
            append_files(&state, files);
            // Custom routes to its own window — `open_window_for_pending`
            // detects this via `custom_preset` being None + files +
            // no other tool flag. The Custom window reads `pending_files`
            // and the user picks the preset interactively.
        }
        Command::Settings | Command::FirstRun | Command::Cleanup => {
            // Settings opens the main window — falls out of
            // `open_window_for_pending` when no files are pending.
            // FirstRun/Cleanup are the headless fast-paths handled in
            // `run()` above and never reach here.
        }
    }
}

fn append_files(state: &tauri::State<'_, PendingState>, files: Vec<std::path::PathBuf>) {
    let strs: Vec<String> = files.iter().map(|p| p.display().to_string()).collect();
    state.files.lock().unwrap().extend(strs);
}

/// Decide which window to open after the debounce settles and do it.
/// Called exactly once per primary-process lifetime. Errors are swallowed
/// (we're on a helper thread with nowhere to propagate to) — a failed
/// open would surface on first user interaction regardless.
fn open_window_for_pending(handle: &tauri::AppHandle) {
    let state = match handle.try_state::<PendingState>() {
        Some(s) => s,
        None => {
            dlog!("open_window_for_pending: no PendingState → open_main_window");
            match commands::open_main_window(handle) {
                Ok(_) => dlog!("  open_main_window OK"),
                Err(e) => dlog!("  open_main_window FAILED: {:#}", e),
            }
            return;
        }
    };

    let files = state.files.lock().unwrap().clone();
    let has_files = !files.is_empty();
    let merge = *state.merge.lock().unwrap();
    let compare = *state.compare.lock().unwrap();
    let grayscale = *state.grayscale.lock().unwrap();
    let overlay = *state.overlay.lock().unwrap();
    let preset_id = state.preset_id.lock().unwrap().clone();

    dlog!(
        "open_window_for_pending: files={}, preset={:?}, merge={}, compare={}, grayscale={}, overlay={}",
        files.len(),
        preset_id,
        merge,
        compare,
        grayscale,
        overlay
    );

    if !has_files {
        dlog!("  → open_main_window (no files)");
        match commands::open_main_window(handle) {
            Ok(_) => dlog!("  open_main_window OK"),
            Err(e) => dlog!("  open_main_window FAILED: {:#}", e),
        }
        return;
    }

    // Custom: files present but no preset_id and no tool flag → user
    // invoked `offspring custom <files>` and wants the tweak dialog.
    if preset_id.is_none() && !merge && !compare && !grayscale && !overlay {
        dlog!("  → open_custom_window");
        match commands::open_custom_window(handle, files) {
            Ok(_) => dlog!("  open_custom_window OK"),
            Err(e) => dlog!("  open_custom_window FAILED: {:#}", e),
        }
        return;
    }

    // Everything else (preset batch, merge, compare, grayscale, overlay)
    // routes through the shared progress window. The Svelte side reads
    // the pending flags via get_pending_* and picks the encode backend.
    dlog!("  → open_progress_window");
    match commands::open_progress_window(handle) {
        Ok(_) => dlog!("  open_progress_window OK"),
        Err(e) => dlog!("  open_progress_window FAILED: {:#}", e),
    }
}
