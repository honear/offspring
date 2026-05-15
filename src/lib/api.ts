import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Preset, Settings, FfmpegStatus, ProgressEvent, UpdateInfo, TrimLast } from "./types";

export const listPresets = () => invoke<Preset[]>("list_presets");
export const savePresets = (presets: Preset[]) => invoke<void>("save_presets", { presetsIn: presets });
export const resetPresetsToDefaults = () => invoke<Preset[]>("reset_presets_to_defaults");

export const getSettings = () => invoke<Settings>("get_settings");
export const saveSettings = (settings: Settings) => invoke<void>("save_settings", { settings });

export const ffmpegStatus = () => invoke<FfmpegStatus>("ffmpeg_status");

/** Start the in-app FFmpeg download. Progress arrives on `ffmpeg-download`.
 * The Rust side returns as soon as the worker thread is spawned. */
export const downloadFfmpeg = () => invoke<void>("download_ffmpeg");

export type FfmpegDownloadEvent = {
  /** "downloading" | "extracting" | "done" | "error" */
  phase: string;
  percent: number | null;
  message: string | null;
};

export function onFfmpegDownload(
  fn: (ev: FfmpegDownloadEvent) => void,
): Promise<UnlistenFn> {
  return listen<FfmpegDownloadEvent>("ffmpeg-download", (e) => fn(e.payload));
}

export const getCustomLast = () => invoke<Preset>("get_custom_last");
export const saveCustomLast = (preset: Preset) => invoke<void>("save_custom_last", { preset });

export const getTrimLast = () => invoke<TrimLast>("get_trim_last");
export const saveTrimLast = (trim: TrimLast) => invoke<void>("save_trim_last", { trim });

export const syncIntegrations = () => invoke<void>("sync_integrations");
export const restartExplorer = () => invoke<void>("restart_explorer");
/** Manual setup path for the Win11 modern right-click menu. Imports
 *  the shipped shell-extension cert into the current user's
 *  TrustedPeople store, then registers the MSIX package(s) for the
 *  active split-layout setting. Surfaced via a Settings button so
 *  users who opted out at install time, or who are a second user on
 *  a shared PC, can opt in without re-running the installer. */
export const setupModernMenu = () => invoke<void>("setup_modern_menu");

/** Returns the build variant the Rust side was compiled as.
 *  "standard" → full app with FFmpeg downloader, in-app updater, and
 *  the Win11 modern-menu cert+MSIX integration.
 *  "studio" → cert-free, no-outbound-network variant. The frontend
 *  hides the buttons that would call into compiled-out code paths
 *  (Download FFmpeg, Check for updates, Reinstall modern menu). */
export const getBuildVariant = () => invoke<"standard" | "studio">("get_build_variant");

/** Returns the OS the binary is running on. Lets the frontend
 *  conditionally hide platform-specific UI — e.g. NVIDIA NVENC
 *  toggle hidden on macOS, since Mac falls back to libx264 regardless. */
export const getPlatform = () => invoke<"windows" | "macos" | "linux">("get_platform");

/** macOS Services picker: run a chosen preset on the pasted files. */
export const pickRunPreset = (files: string[], preset_id: string) =>
  invoke<void>("pick_run_preset", { files, presetId: preset_id });

/** macOS Services picker: open the chosen tool's dialog window on the pasted files.
 *  `tool` is one of: "modify" | "trim" | "compare". */
export const pickRunTool = (files: string[], tool: "modify" | "trim" | "compare") =>
  invoke<void>("pick_run_tool", { files, tool });
export const openDataFolder = () => invoke<void>("open_data_folder");
/** Open `%LOCALAPPDATA%\Offspring` in Explorer with `debug.log`
 *  selected (or just the directory when the log doesn't exist yet). */
export const openLogFolder = () => invoke<void>("open_log_folder");

/** Hand off an http(s) URL to the user's default browser (via the
 *  Windows shell). Replaces `@tauri-apps/plugin-opener`'s `openUrl`
 *  for our brand-link click — the JS path was failing silently in
 *  WebView2. The Rust side rejects non-http(s) schemes. */
export const openExternalUrl = (url: string) =>
  invoke<void>("open_external_url", { url });

/** Signal any in-flight ffmpeg encode to abort. The next ~1s poll
 *  tick in `run_with_progress` kills the ffmpeg child + deletes the
 *  partial output file. Safe to call when no encode is running —
 *  next encode entry-point resets the flag on the way in. */
export const cancelEncode = () => invoke<void>("cancel_encode");

/** Open a native file picker filtered to alpha-capable image formats
 *  and return the chosen path. `null` means the user cancelled.
 *  Used by the Overlay tool's "Add watermark" toggle. */
export const pickWatermarkFile = () => invoke<string | null>("pick_watermark_file");

export const encode = (files: string[], preset: Preset) =>
  invoke<void>("encode", { files, preset });

/** Merge-mode encode: concatenate `files` (already in desired order)
 *  into a single output. Format + encode settings are derived from the
 *  first file by the Rust side — no preset arg. Progress events flow
 *  through the same `encode-progress` channel; file_index and
 *  total_files are both 1 since a merge is one logical encode. */
export const encodeMerge = (files: string[]) =>
  invoke<void>("encode_merge", { files });

/** Greyscale-mode encode: desaturates each file to its own
 *  format-matched greyscale copy. No preset arg — settings are derived
 *  per-file by the Rust side. */
export const encodeGrayscale = (files: string[]) =>
  invoke<void>("encode_grayscale", { files });

/** A/B Compare: stack N inputs horizontally into one output video for
 *  visual comparison. Output is named `<first-stem>_compare.<ext>`. */
export const encodeCompare = (files: string[]) =>
  invoke<void>("encode_compare", { files });

/** Compare-grid encode: arrange 3+ clips into a `cols`-wide grid.
 *  Layout = "grid" preserves each clip's aspect (pads with black);
 *  layout = "mosaic" scales + crops to fill cells completely. */
export const encodeCompareGrid = (files: string[], cols: number, layout: "grid" | "mosaic") =>
  invoke<void>("encode_compare_grid", { files, cols, layout });

/** Stash files in app state ahead of the Compare-grid dialog
 *  navigating to /progress/. Mirrors `prepareModifyEncode` /
 *  `prepareTrimEncode`. */
export const prepareCompareGridEncode = (files: string[]) =>
  invoke<void>("prepare_compare_grid_encode", { files });

/** Overlay encode: per-file burn-in of corner text + optional border
 *  + optional aspect guides. All config comes from settings.tools.overlay. */
export const encodeOverlay = (files: string[]) =>
  invoke<void>("encode_overlay", { files });

/** Invert tool: per-image color invert. Image-only — the backend
 *  refuses video inputs with a clear error. The clamp setting comes
 *  from `settings.tools.invert.clamp` server-side, no arg needed. */
export const encodeInvert = (files: string[]) =>
  invoke<void>("encode_invert", { files });

/** Make Square tool: pad shorter edge of each image to match the
 *  longer one. Image-only. Fill mode comes from
 *  `settings.tools.make_square.fill_mode` server-side. */
export const encodeMakeSquare = (files: string[]) =>
  invoke<void>("encode_make_square", { files });

/** Modify tool: per-file crop + optional flip / reverse / overwrite.
 *  Same set of transforms applied to every file. `cropW=0` and
 *  `cropH=0` means "no crop, only the other transforms". */
export const encodeModify = (
  files: string[],
  cropX: number,
  cropY: number,
  cropW: number,
  cropH: number,
  flipH: boolean,
  flipV: boolean,
  reverse: boolean,
  removeAudio: boolean,
  rotate: number,
  trimStartSec: number,
  trimEndSec: number,
  overwrite: boolean,
) =>
  invoke<void>("encode_modify", {
    files,
    cropX,
    cropY,
    cropW,
    cropH,
    flipH,
    flipV,
    reverse,
    removeAudio,
    rotate,
    trimStartSec,
    trimEndSec,
    overwrite,
  });

/** Stash files in app state ahead of the Modify dialog navigating
 *  to /progress/. Mirrors `prepareTrimEncode` — no second window
 *  opens. */
export const prepareModifyEncode = (files: string[]) =>
  invoke<void>("prepare_modify_encode", { files });

/** Probe (width, height) of a file. Returns null when ffprobe can't
 *  read it. The Crop dialog uses this to set up coordinate mapping. */
export const probeDimensions = (path: string) =>
  invoke<[number, number] | null>("probe_dimensions", { path });

/** Extract one preview frame to a JPEG and return its absolute path.
 *  The Crop dialog calls this when WebView2 can't decode a video
 *  natively (ProRes / DNxHD / weird MKVs); the result is displayed
 *  via `<img>` instead of `<video>`. */
export const extractPreviewFrame = (path: string, timeSeconds: number) =>
  invoke<string>("extract_preview_frame", { path, timeSeconds });

/** Trim encode: strip `startFrames` from the front and `endFrames`
 *  from the back of each input, and optionally cut the inclusive
 *  range `[removeFrom, removeTo]` from the middle. Per-file
 *  independent — the same set of values is applied to each input's
 *  own timeline. Output is `<stem>_trimmed.<ext>` next to each source.
 *  Pass `null` (or omit) for `removeFrom`/`removeTo` to skip the
 *  middle cut; both must be set and in non-inverted order for the
 *  cut to actually run. */
export const encodeTrim = (
  files: string[],
  startFrames: number,
  endFrames: number,
  removeFrom?: number | null,
  removeTo?: number | null,
) =>
  invoke<void>("encode_trim", {
    files,
    startFrames,
    endFrames,
    removeFrom: removeFrom ?? null,
    removeTo: removeTo ?? null,
  });

export const getPendingFiles = () => invoke<string[]>("get_pending_files");
export const getPendingPresetId = () => invoke<string | null>("get_pending_preset_id");
export const getPendingCustomPreset = () => invoke<Preset | null>("get_pending_custom_preset");
export const getPendingMerge = () => invoke<boolean>("get_pending_merge");
export const getPendingGrayscale = () => invoke<boolean>("get_pending_grayscale");
export const getPendingCompare = () => invoke<boolean>("get_pending_compare");
export const getPendingOverlay = () => invoke<boolean>("get_pending_overlay");
export const getPendingTrimDialog = () => invoke<boolean>("get_pending_trim_dialog");
export const getPendingInvert = () => invoke<boolean>("get_pending_invert");
export const getPendingMakeSquare = () => invoke<boolean>("get_pending_make_square");
export const getPendingModifyDialog = () => invoke<boolean>("get_pending_modify_dialog");

/** Stash files + custom preset in app state ahead of navigating the current
 * webview to the progress route. Unlike the old `start_custom_encode`, this
 * does NOT open a second window — the caller (Custom dialog) is expected to
 * navigate its own webview to /progress/ after this resolves. */
export const prepareCustomEncode = (files: string[], preset: Preset) =>
  invoke<void>("prepare_custom_encode", { files, preset });

/** Stash files in app state ahead of the Trim dialog navigating in-place
 *  to /progress/. Persists the chosen frame counts (and optional middle
 *  cut) to disk so the dialog reopens with them next time. The progress
 *  page picks up the files via `getPendingFiles()` and calls
 *  `encodeTrim` directly. */
export const prepareTrimEncode = (
  files: string[],
  startFrames: number,
  endFrames: number,
  removeFrom?: number | null,
  removeTo?: number | null,
) =>
  invoke<void>("prepare_trim_encode", {
    files,
    startFrames,
    endFrames,
    removeFrom: removeFrom ?? null,
    removeTo: removeTo ?? null,
  });

export function onProgress(fn: (ev: ProgressEvent) => void): Promise<UnlistenFn> {
  return listen<ProgressEvent>("encode-progress", (e) => fn(e.payload));
}

export function onFinished(fn: (total: number) => void): Promise<UnlistenFn> {
  return listen<number>("encode-finished", (e) => fn(e.payload));
}

/** Hit the GitHub Releases API and report whether a newer version exists.
 * Never throws — network errors collapse to `update_available: false`. */
export const checkForUpdates = () => invoke<UpdateInfo>("check_for_updates");

/** Return the installed Offspring version (`CARGO_PKG_VERSION`) WITHOUT
 *  making any network call. The Settings page uses this to fill the
 *  "Current version: …" line on launch — we deliberately don't auto-
 *  hit GitHub anymore, so the UI needs a local way to learn its own
 *  version. */
export const getAppVersion = () => invoke<string>("get_app_version");

/** Kick off a background download of the installer for `version` from
 * `installerUrl`. Progress arrives on `update-download`. Resolves as soon
 * as the worker thread is spawned — observe `onUpdateDownload` for
 * completion. */
export const downloadUpdate = (version: string, installerUrl: string) =>
  invoke<void>("download_update", { version, installerUrl });

/** Launch the previously-downloaded installer silently and exit the app
 * so Inno Setup can overwrite offspring.exe. The installer re-launches
 * Offspring automatically after the swap. */
export const installUpdate = (version: string) =>
  invoke<void>("install_update", { version });

export type UpdateDownloadEvent = {
  /** "downloading" | "done" | "error" */
  phase: string;
  percent: number | null;
  /** On "done": absolute path to the downloaded .exe. On "error": the
   * error message. On "downloading": a human-readable byte count. */
  message: string | null;
};

export function onUpdateDownload(
  fn: (ev: UpdateDownloadEvent) => void,
): Promise<UnlistenFn> {
  return listen<UpdateDownloadEvent>("update-download", (e) => fn(e.payload));
}

/** Resolve after WebView2 has had a chance to commit the next paint.
 *
 *  We open the encode-related windows with `.visible(false)` on the
 *  Rust side and reveal them from the frontend so the user never sees
 *  a blank-frame flash. But Svelte's `onMount` fires the moment the
 *  DOM is built — Chromium-based WebView2 lags one or two frames
 *  behind that before the pixels are actually on screen. Calling
 *  `show()` straight from `onMount` therefore briefly reveals an
 *  empty white window.
 *
 *  Two `requestAnimationFrame`s are the standard browser idiom: the
 *  first lets style/layout settle, the second runs after the paint
 *  has committed. Equivalent to "wait until the next frame is on
 *  screen". 16-ish ms on a 60Hz display.
 */
export function afterFirstPaint(): Promise<void> {
  return new Promise((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  });
}
