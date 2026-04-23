export type Format = "gif" | "mp4";
export type Crop = "16:9" | "9:16" | "1:1" | "4:3";
export type Dither = "bayer" | "floydsteinberg" | "sierra2" | "sierra24a" | "none";

export interface Preset {
  id: string;
  name: string;
  enabled: boolean;
  format: Format;
  suffix: string;
  width?: number | null;
  height?: number | null;
  fps?: number | null;
  crop?: Crop | null;
  palette_colors?: number | null;
  dither?: Dither | null;
  bayer_scale?: number | null;
  crf?: number | null;
  preset_speed?: string | null;
  video_bitrate?: string | null;
  audio_bitrate?: string | null;
  use_cuda?: boolean | null;
  target_max_mb?: number | null;
  icon?: string | null;
  order: number;
}

export interface Settings {
  ffmpeg_path?: string | null;
  verbosity?: string | null;
  pause_after?: boolean | null;
  descriptive_names?: boolean | null;
  /** Mirror presets into the user's Windows SendTo folder. Off by default
   *  — the registry right-click menu covers the same use-case and is more
   *  discoverable on Windows 11. */
  sendto_enabled?: boolean | null;
  /** Surface Offspring in the Windows 11 _top-level_ right-click menu via
   *  an MSIX sparse package. Off by default; flipping on prompts the user
   *  to trust our self-signed cert. */
  modern_menu_enabled?: boolean | null;
}

export interface FfmpegStatus {
  found: boolean;
  path: string | null;
}

export interface ProgressEvent {
  file_index: number;
  total_files: number;
  input: string;
  stage: "palette" | "encode" | "done" | "error";
  percent: number | null;
  message: string | null;
}

export interface UpdateInfo {
  /** Running version, e.g. "0.2.0". */
  current: string;
  /** Latest published version, or "" if the check couldn't resolve one. */
  latest: string;
  /** True iff `latest` is strictly greater than `current`. */
  update_available: boolean;
  /** GitHub release page. Opened in the default browser on click. */
  html_url: string;
  /** Direct .exe download URL, or "" if no matching asset was found. */
  installer_url: string;
}
