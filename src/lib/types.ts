export type Format = "gif" | "mp4" | "image";
export type Crop = "16:9" | "9:16" | "1:1" | "4:3";
export type Dither = "bayer" | "floydsteinberg" | "sierra2" | "sierra24a" | "none";

/** Encoder for `format=image` presets. Each codec defines its own
 *  scale for `image_quality` — see the Rust side `ImageCodec` enum
 *  for ranges. The UI's quality field re-labels and re-bounds itself
 *  based on this value. */
export type ImageCodec = "png" | "jpeg" | "webp" | "avif";

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
  /** Image-format only. PNG / JPEG / WebP / AVIF. */
  image_codec?: ImageCodec | null;
  /** Image-format only. Quality / compression level in the codec's
   *  native scale (PNG: 0-9, JPEG: 1-100, WebP: 0-100, AVIF: 0-63). */
  image_quality?: number | null;
  /** Image-format only. Strip EXIF / GPS / camera-serial metadata
   *  via ffmpeg's `-map_metadata -1`. On for shipped image presets. */
  strip_metadata?: boolean | null;
  /** Desaturate the output to greyscale. Independent of format — works
   *  on both GIF and MP4. Also reachable as a standalone Tool. */
  grayscale?: boolean | null;
  /** Burn in the current frame number in the top-left corner using
   *  Consolas. Independent of format. Also available as an Overlay
   *  feature when that tool lands. */
  timecode?: boolean | null;
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
  /** Extension tools: auto-sequence detection, merge, etc. Always present
   *  — Rust fills in defaults for missing fields when loading old settings. */
  tools?: ToolsSettings;
}

export interface ToolsSettings {
  sequence: SequenceTool;
  merge: MergeTool;
  grayscale: GrayscaleTool;
  compare: CompareTool;
  overlay: OverlayTool;
  trim: TrimTool;
}

/** Trim tool: per-file frame-accurate trim. UI surfaces a "Trim..."
 *  entry in the right-click menu that opens a mini dialog asking for
 *  start/end frame counts. Output is `<stem>_trimmed.<ext>`. */
export interface TrimTool {
  enabled: boolean;
}

/** Persisted last-used Trim dialog values so the dialog reopens with
 *  the user's previous numbers instead of zeros. `remove_from`/
 *  `remove_to` are the optional middle-range cut — both must be set and
 *  in non-inverted order for the encoder to honour them. */
export interface TrimLast {
  start_frames: number;
  end_frames: number;
  remove_from?: number | null;
  remove_to?: number | null;
}

export interface SequenceTool {
  /** On by default. When enabled, right-clicking a numbered image frame
   *  auto-expands to the full sequence before encoding. */
  enabled: boolean;
  /** Minimum zero-padded digit count that counts as a sequence. Default 4
   *  — matches VFX convention (render_0001.png) and filters out version
   *  tags like r01 / v02. */
  min_digits: number;
  /** Fallback framerate used when the preset doesn't specify one. VFX /
   *  broadcast rates (23.976, 29.97) are allowed. Preset.fps wins over
   *  this when set — only MP4 presets that leave fps unset fall back. */
  default_fps: number;
}

export interface MergeTool {
  /** On by default. Shows a "Merge" leaf entry in the Windows 11
   *  modern right-click menu for multi-file selections. */
  enabled: boolean;
}

export interface GrayscaleTool {
  /** On by default. Shows a "Greyscale" leaf entry that converts each
   *  selected file to a greyscale copy, preserving its format, size
   *  and fps. */
  enabled: boolean;
}

export type OverlaySlot = "none" | "filename" | "timecode" | "custom" | "custom2";

export interface CompareTool {
  /** On by default. Shows a "Compare" leaf entry that hstacks all
   *  selected files into a single side-by-side output for A/B review.
   *  Only appears when ≥2 files are selected. */
  enabled: boolean;
}

export interface OverlayTool {
  /** Off by default — niche workflow. Per-file encode that burns
   *  corner text + optional border + optional aspect-ratio guides. */
  enabled: boolean;
  top_left: OverlaySlot;
  top_right: OverlaySlot;
  bottom_left: OverlaySlot;
  bottom_right: OverlaySlot;
  /** Shared text used by any corner whose slot is "custom". */
  custom_text: string;
  /** Second independent text slot, paired with the "custom2" dropdown
   *  option so one overlay can carry two arbitrary labels at once. */
  custom_text_2: string;
  /** 0–100 UI opacity. Mapped to 0.0–1.0 inside the ffmpeg filter. */
  opacity: number;
  /** ffmpeg-parseable color (e.g. "white", "0xffcc00"). The UI sends
   *  hex-ish strings with a leading "0x". */
  color: string;
  /** Pad the clip with equal black bars on all four sides so corner
   *  text sits in the border instead of on top of the image. */
  border: boolean;
  /** "Add metadata" toggle: when false, corner text + border + the
   *  color/opacity controls they depend on do nothing — only the
   *  Guides half of the pane is emitted. */
  metadata: boolean;
  /** "Add guides" toggle. When true, draw the aspect-ratio guide
   *  boxes on top of the clip (the per-ratio show_* booleans pick
   *  which boxes). When false, no guide boxes are drawn regardless. */
  guides: boolean;
  show_16_9: boolean;
  show_9_16: boolean;
  show_4_5: boolean;
  color_16_9: string;
  color_9_16: string;
  color_4_5: string;
  /** 0–100 UI opacity applied to the guide boxes (separate from
   *  the corner-text opacity above). Defaults to 90. */
  guides_opacity: number;
  /** Font size as a percentage (50–200, default 100). Scales the
   *  corner text, its margin from the frame edge, and the box-border
   *  width together so layout stays balanced. */
  metadata_font_scale: number;
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
