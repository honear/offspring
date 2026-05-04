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
export const openDataFolder = () => invoke<void>("open_data_folder");

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
