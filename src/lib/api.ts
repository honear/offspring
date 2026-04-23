import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Preset, Settings, FfmpegStatus, ProgressEvent, UpdateInfo } from "./types";

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

export const syncIntegrations = () => invoke<void>("sync_integrations");
export const restartExplorer = () => invoke<void>("restart_explorer");
export const openDataFolder = () => invoke<void>("open_data_folder");

export const encode = (files: string[], preset: Preset) =>
  invoke<void>("encode", { files, preset });

export const getPendingFiles = () => invoke<string[]>("get_pending_files");
export const getPendingPresetId = () => invoke<string | null>("get_pending_preset_id");
export const getPendingCustomPreset = () => invoke<Preset | null>("get_pending_custom_preset");

/** Stash files + custom preset in app state ahead of navigating the current
 * webview to the progress route. Unlike the old `start_custom_encode`, this
 * does NOT open a second window — the caller (Custom dialog) is expected to
 * navigate its own webview to /progress/ after this resolves. */
export const prepareCustomEncode = (files: string[], preset: Preset) =>
  invoke<void>("prepare_custom_encode", { files, preset });

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
