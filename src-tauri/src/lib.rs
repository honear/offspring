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
#[cfg(windows)]
mod single_instance;
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
    // Redact file paths in the log dump — argv from a right-click drops
    // the absolute paths of every selected file, which is more
    // information than the debug log should retain. `redact_argv` keeps
    // the verb names and flags but collapses paths to `…/<filename>`.
    dlog!("run() enter; argv={:?}", debug_log::redact_argv(&raw_argv));

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

    // Singleton + argv forwarding (Windows only; Offspring is Windows-only
    // but the cfg gates this so non-Windows dev builds still compile).
    //
    // Crucial: this runs BEFORE `tauri::Builder::default().run(...)` boots
    // tao. Tauri's runtime creates a hidden message-pump window on
    // Windows during init, which briefly flashed on screen for every
    // secondary instance under the old `tauri-plugin-single-instance`
    // arrangement (the plugin only short-circuited AFTER the runtime
    // was up). Doing the IPC ourselves up here makes secondaries exit
    // cleanly with no UI side effects whatsoever.
    //
    // We ALSO start the IPC listener thread right here — before Tauri
    // Builder runs. A multi-file right-click via the classic context
    // menu fires N offspring.exe processes in rapid succession; the
    // secondaries that race ahead of the primary's setup() would
    // otherwise find no pipe to forward to and silently drop their
    // argv after the connect-retry budget expired. The listener
    // queues incoming argv into an mpsc channel; the primary's setup()
    // spins up a processor thread that drains the channel and feeds
    // each forwarded invocation through `merge_pending` + the debounce
    // clock. Bridging via the channel decouples "I can accept argv
    // now" (early, no AppHandle needed) from "I can dispatch argv
    // through Tauri state" (only after setup() runs).
    //
    // `_primary_guard` must stay in scope for the rest of run() — it
    // owns the mutex handle that proves we're the primary.
    #[cfg(windows)]
    let (argv_tx, argv_rx) = std::sync::mpsc::channel::<Vec<String>>();
    #[cfg(windows)]
    let argv_rx = std::sync::Mutex::new(Some(argv_rx));
    #[cfg(windows)]
    let _primary_guard = match single_instance::try_become_primary() {
        Ok(Some(guard)) => {
            dlog!("singleton: acquired primary mutex");
            // Start the listener IMMEDIATELY. Anything that arrives
            // before setup() runs is buffered in the channel and
            // drained when the processor thread comes up.
            let argv_tx_for_listener = argv_tx.clone();
            single_instance::start_listener(move |argv| {
                let _ = argv_tx_for_listener.send(argv);
            });
            dlog!("singleton: pipe listener spawned");
            Some(guard)
        }
        Ok(None) => {
            dlog!("singleton: another primary exists; forwarding argv and exiting");
            // Best effort — if forwarding fails (primary crashed mid-flight,
            // pipe denied) we exit anyway. The user re-clicking will start
            // a fresh primary because the dead primary's mutex auto-clears
            // at process exit.
            if let Err(e) = single_instance::forward_argv_to_primary(&raw_argv) {
                dlog!("singleton: forward failed: {:#}", e);
            }
            std::process::exit(0);
        }
        Err(e) => {
            dlog!("singleton: mutex check error: {:#}; degrading to primary", e);
            None
        }
    };
    // Drop the original Sender so the receiver's iterator terminates
    // when no listener clones remain. (We don't actually rely on
    // termination — the listener thread runs forever — but cleaning up
    // the unused clone is good hygiene.)
    #[cfg(windows)]
    drop(argv_tx);

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
            commands::get_trim_last,
            commands::save_trim_last,
            commands::sync_integrations,
            commands::restart_explorer,
            commands::open_data_folder,
            commands::encode,
            commands::encode_merge,
            commands::encode_grayscale,
            commands::encode_compare,
            commands::encode_overlay,
            commands::encode_trim,
            commands::encode_invert,
            commands::encode_make_square,
            commands::get_pending_files,
            commands::get_pending_preset_id,
            commands::get_pending_custom_preset,
            commands::get_pending_merge,
            commands::get_pending_grayscale,
            commands::get_pending_compare,
            commands::get_pending_overlay,
            commands::get_pending_trim_dialog,
            commands::get_pending_invert,
            commands::get_pending_make_square,
            commands::prepare_custom_encode,
            commands::prepare_trim_encode,
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
            // more args will land via the single_instance listener
            // (registered below) before the debounce fires.
            merge_pending(&handle, initial_command.clone());
            *last_arrival.lock().unwrap() = Instant::now();

            // Drain the IPC channel started before Tauri Builder ran.
            // The listener has been buffering argv from secondaries
            // since the moment we acquired the mutex; this thread
            // reads them in arrival order and applies the same logic
            // that used to live inline in the listener callback:
            // new-batch detection, CLI parse, merge_pending, arrival
            // clock update.
            #[cfg(windows)]
            {
                let handle_for_listener = handle.clone();
                let last_arrival_for_listener = last_arrival.clone();
                let last_dispatch_for_listener = last_dispatch.clone();
                // Take ownership of the receiver — we hold it in an
                // Option<Mutex> at the run() level so the move closure
                // can extract it once. (mpsc::Receiver is !Sync so we
                // can't share it with other threads anyway.)
                let argv_rx = argv_rx
                    .lock()
                    .unwrap()
                    .take()
                    .expect("argv receiver should still be available");
                std::thread::spawn(move || {
                    dlog!("ipc processor thread spawned");
                    for argv in argv_rx {
                        dlog!("ipc processor: secondary argv={:?}", debug_log::redact_argv(&argv));

                        // New-batch detection: if the last window
                        // dispatch was more than BATCH_DEBOUNCE * 2
                        // ago, this arrival begins a fresh user action
                        // (a new right-click, not a still-trickling-in
                        // multi-file batch). Clear stale pending state
                        // so the previous batch's files don't ride
                        // along.
                        let is_new_batch = {
                            let guard = last_dispatch_for_listener.lock().unwrap();
                            match *guard {
                                Some(d) => d.elapsed() > BATCH_DEBOUNCE * 2,
                                None => false,
                            }
                        };
                        if is_new_batch {
                            dlog!("  new batch detected; clearing stale pending state");
                            if let Some(state) = handle_for_listener.try_state::<PendingState>() {
                                state.files.lock().unwrap().clear();
                                *state.preset_id.lock().unwrap() = None;
                                *state.merge.lock().unwrap() = false;
                                *state.compare.lock().unwrap() = false;
                                *state.grayscale.lock().unwrap() = false;
                                *state.overlay.lock().unwrap() = false;
                                *state.trim_dialog.lock().unwrap() = false;
                                *state.invert.lock().unwrap() = false;
                                *state.make_square.lock().unwrap() = false;
                            }
                            // Critical: zero `last_dispatch` so the very next
                            // arrival in this same batch (typically tens of
                            // milliseconds later, well within
                            // BATCH_DEBOUNCE * 2) doesn't ALSO see the stale
                            // dispatch timestamp and re-trigger the clear-
                            // state path. Without this, every multi-file
                            // batch after the first one collapses to its
                            // LAST file because each subsequent arrival
                            // wipes the files added by its predecessors.
                            // The next dispatch (the debounce thread) will
                            // re-set this to `Some(now)` after the window
                            // opens.
                            *last_dispatch_for_listener.lock().unwrap() = None;
                        }

                        match Cli::try_parse_from(&argv) {
                            Ok(cli) => {
                                dlog!("  secondary parsed; command={:?}", cli.command);
                                merge_pending(&handle_for_listener, cli.command);
                                *last_arrival_for_listener.lock().unwrap() = Instant::now();
                            }
                            Err(e) => {
                                dlog!("  secondary PARSE FAILED: {}", e);
                            }
                        }
                    }
                });
            }

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
        Command::Invert { files } => {
            dlog!("merge_pending: Invert +files={}", files.len());
            append_files(&state, files);
            *state.invert.lock().unwrap() = true;
        }
        Command::MakeSquare { files } => {
            dlog!("merge_pending: MakeSquare +files={}", files.len());
            append_files(&state, files);
            *state.make_square.lock().unwrap() = true;
        }
        Command::Trim { files } => {
            dlog!("merge_pending: Trim +files={}", files.len());
            append_files(&state, files);
            *state.trim_dialog.lock().unwrap() = true;
            // Trim routes to its own mini window. The window reads the
            // pending files + the saved trim_last numbers, asks the user
            // to confirm/edit them, then navigates in-place to /progress/
            // and calls encode_trim. No second window opens — same
            // pattern Custom uses to dodge the WebView2 bug.
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
    let trim_dialog = *state.trim_dialog.lock().unwrap();
    let invert = *state.invert.lock().unwrap();
    let make_square = *state.make_square.lock().unwrap();
    let preset_id = state.preset_id.lock().unwrap().clone();

    dlog!(
        "open_window_for_pending: files={}, preset={:?}, merge={}, compare={}, grayscale={}, overlay={}, trim_dialog={}",
        files.len(),
        preset_id,
        merge,
        compare,
        grayscale,
        overlay,
        trim_dialog
    );

    if !has_files {
        dlog!("  → open_main_window (no files)");
        match commands::open_main_window(handle) {
            Ok(_) => dlog!("  open_main_window OK"),
            Err(e) => dlog!("  open_main_window FAILED: {:#}", e),
        }
        return;
    }

    // Trim: files present + trim_dialog flag → open the Trim mini
    // dialog. Checked before Custom because trim_dialog is the more
    // specific signal — Custom is the fallback for "files but no
    // tool flag at all".
    if trim_dialog {
        dlog!("  → open_trim_window");
        match commands::open_trim_window(handle, files) {
            Ok(_) => dlog!("  open_trim_window OK"),
            Err(e) => dlog!("  open_trim_window FAILED: {:#}", e),
        }
        return;
    }

    // Custom: files present but no preset_id and no tool flag → user
    // invoked `offspring custom <files>` and wants the tweak dialog.
    if preset_id.is_none() && !merge && !compare && !grayscale && !overlay && !invert && !make_square {
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
