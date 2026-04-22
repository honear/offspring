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
