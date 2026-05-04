<script lang="ts">
  import type { Preset, ImageCodec } from "$lib/types";
  let { preset }: { preset: Preset } = $props();

  // Per-codec quality field metadata: label, range, default, and
  // whether the field is a quality slider or a compression-level dial
  // (PNG is lossless — its "quality" is really speed-vs-size). Pulled
  // out so the template stays declarative and adding a new codec
  // means adding one row here, not surgery on three Svelte blocks.
  const IMG_QUALITY: Record<
    ImageCodec,
    { label: string; min: number; max: number; default: number; hint: string }
  > = {
    png:  { label: "Compression level (0–9)", min: 0,  max: 9,   default: 6,  hint: "0 = fastest / largest, 9 = slowest / smallest. PNG is lossless either way." },
    jpeg: { label: "Quality (1–100)",         min: 1,  max: 100, default: 85, hint: "Higher = better quality, larger file. 85 is the standard 'web-quality' default." },
    webp: { label: "Quality (0–100)",         min: 0,  max: 100, default: 80, hint: "Higher = better. Lossy. 80 is a sensible default for web-shareable images." },
    avif: { label: "CRF (0–63, lower=better)", min: 0,  max: 63,  default: 24, hint: "Lower = better quality, larger file. 24 is a good middle ground." },
  };

  // Default the codec to PNG when format flips to image and no codec
  // is set yet; saves the user one click on a fresh preset.
  $effect(() => {
    if (preset.format === "image" && !preset.image_codec) {
      preset.image_codec = "png";
      if (preset.image_quality == null) {
        preset.image_quality = IMG_QUALITY.png.default;
      }
    }
  });
</script>

<div class="grid">
  <div>
    <label>Format</label>
    <select bind:value={preset.format}>
      <option value="gif">GIF</option>
      <option value="mp4">MP4</option>
      <option value="image">Image</option>
    </select>
  </div>
  <div>
    <label>Suffix</label>
    <input type="text" bind:value={preset.suffix} placeholder="_720p" />
  </div>
  <div>
    <label>Width (px)</label>
    <input
      type="number"
      value={preset.width ?? ""}
      oninput={(e) => {
        const v = (e.currentTarget as HTMLInputElement).value;
        preset.width = v === "" ? null : parseInt(v, 10);
      }}
      placeholder="auto"
    />
  </div>
  <div>
    <label>Height (px)</label>
    <input
      type="number"
      value={preset.height ?? ""}
      oninput={(e) => {
        const v = (e.currentTarget as HTMLInputElement).value;
        preset.height = v === "" ? null : parseInt(v, 10);
      }}
      placeholder="auto"
    />
  </div>
  {#if preset.format !== "image"}
    <!-- FPS is meaningless on still-image output; hide it instead of
         leaving a confusing always-empty field on image presets. -->
    <div>
      <label>FPS</label>
      <input
        type="number"
        value={preset.fps ?? ""}
        oninput={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          preset.fps = v === "" ? null : parseInt(v, 10);
        }}
        placeholder="keep source"
      />
    </div>
  {/if}
  <div>
    <label>Crop</label>
    <select
      value={preset.crop ?? ""}
      onchange={(e) => {
        const v = (e.currentTarget as HTMLSelectElement).value;
        preset.crop = (v === "" ? null : v) as any;
      }}
    >
      <option value="">None</option>
      <option value="16:9">16:9 (horizontal)</option>
      <option value="9:16">9:16 (vertical)</option>
      <option value="1:1">1:1 (square)</option>
      <option value="4:3">4:3</option>
    </select>
  </div>
  {#if preset.format !== "image"}
    <!-- Target-size logic is video-specific (computes bitrate or
         re-encodes at smaller widths). For images, file size is
         driven by codec quality which is set explicitly below. -->
    <div class="full">
      <label title="Leave blank for quality-based encoding. When set, MP4 bitrate is computed from clip duration; GIF width is iteratively scaled down until output fits.">
        Target max size (MB) — auto-adjusts quality / width
      </label>
      <input
        type="number"
        min="1"
        step="1"
        value={preset.target_max_mb ?? ""}
        oninput={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          preset.target_max_mb = v === "" ? null : parseInt(v, 10);
        }}
        placeholder="no limit"
      />
    </div>
  {/if}
  <div class="full">
    <label class="inline">
      <input
        type="checkbox"
        checked={preset.grayscale ?? false}
        onchange={(e) => {
          preset.grayscale = (e.currentTarget as HTMLInputElement).checked;
        }}
      />
      Greyscale (desaturate output)
    </label>
  </div>
  {#if preset.format !== "image"}
    <!-- Frame-number burn-in is meaningless on a still — there's only
         one frame, and "1" stamped on every output is noise. -->
    <div class="full">
      <label class="inline" title="Burns the frame number in the top-left corner using Consolas.">
        <input
          type="checkbox"
          checked={preset.timecode ?? false}
          onchange={(e) => {
            preset.timecode = (e.currentTarget as HTMLInputElement).checked;
          }}
        />
        Burn-in frame number (timecode)
      </label>
    </div>
  {/if}
</div>

{#if preset.format === "image"}
  <h4 class="subhead">Image options</h4>
  <div class="grid">
    <div>
      <label>Codec</label>
      <select
        value={preset.image_codec ?? "png"}
        onchange={(e) => {
          const v = (e.currentTarget as HTMLSelectElement).value as ImageCodec;
          // Clamp the existing quality value into the new codec's
          // range so changing codec doesn't leave an out-of-range
          // number sitting in state. If the previous value would
          // have been the previous codec's default, also reset to
          // the new codec's default (heuristic: probably the user
          // hadn't customised yet).
          const prev = preset.image_codec ?? "png";
          const wasDefault = preset.image_quality === IMG_QUALITY[prev].default;
          preset.image_codec = v;
          if (wasDefault || preset.image_quality == null) {
            preset.image_quality = IMG_QUALITY[v].default;
          } else {
            const q = preset.image_quality;
            preset.image_quality = Math.max(IMG_QUALITY[v].min, Math.min(IMG_QUALITY[v].max, q));
          }
        }}
      >
        <option value="png">PNG (lossless)</option>
        <option value="jpeg">JPEG</option>
        <option value="webp">WebP</option>
        <option value="avif">AVIF</option>
      </select>
    </div>
    {#if preset.image_codec}
      {@const q = IMG_QUALITY[preset.image_codec]}
      <div>
        <label title={q.hint}>{q.label}</label>
        <input
          type="number"
          min={q.min}
          max={q.max}
          step="1"
          value={preset.image_quality ?? q.default}
          oninput={(e) => {
            const v = (e.currentTarget as HTMLInputElement).value;
            preset.image_quality = v === "" ? null : parseInt(v, 10);
          }}
        />
      </div>
    {/if}
    <div class="full">
      <label class="inline" title="Removes EXIF, GPS coordinates, camera serial number, and other embedded metadata. Recommended for any image you plan to share publicly.">
        <input
          type="checkbox"
          checked={preset.strip_metadata ?? false}
          onchange={(e) => {
            preset.strip_metadata = (e.currentTarget as HTMLInputElement).checked;
          }}
        />
        Strip EXIF / GPS / metadata
      </label>
    </div>
  </div>
{:else if preset.format === "gif"}
  <h4 class="subhead">GIF options</h4>
  <div class="grid">
    <div>
      <label>Palette colors (max 256)</label>
      <input
        type="number"
        min="8"
        max="256"
        value={preset.palette_colors ?? 128}
        oninput={(e) => {
          preset.palette_colors = parseInt((e.currentTarget as HTMLInputElement).value, 10);
        }}
      />
    </div>
    <div>
      <label>Dither</label>
      <select bind:value={preset.dither}>
        <option value="bayer">Bayer (ordered, small)</option>
        <option value="sierra24a">Sierra 2-4A (quality)</option>
        <option value="floydsteinberg">Floyd–Steinberg</option>
        <option value="sierra2">Sierra 2</option>
        <option value="none">None</option>
      </select>
    </div>
    {#if preset.dither === "bayer"}
      <div>
        <label>Bayer scale (1–5)</label>
        <input
          type="number"
          min="1"
          max="5"
          value={preset.bayer_scale ?? 3}
          oninput={(e) => {
            preset.bayer_scale = parseInt((e.currentTarget as HTMLInputElement).value, 10);
          }}
        />
      </div>
    {/if}
  </div>
{:else}
  <h4 class="subhead">MP4 options</h4>
  <div class="grid">
    <div>
      <label>CRF (quality, lower = better)</label>
      <input
        type="number"
        min="0"
        max="51"
        value={preset.crf ?? 23}
        oninput={(e) => {
          preset.crf = parseInt((e.currentTarget as HTMLInputElement).value, 10);
        }}
      />
    </div>
    <div>
      <label>Encoder preset</label>
      <select bind:value={preset.preset_speed}>
        <option value="ultrafast">ultrafast</option>
        <option value="superfast">superfast</option>
        <option value="veryfast">veryfast</option>
        <option value="faster">faster</option>
        <option value="fast">fast</option>
        <option value="medium">medium</option>
        <option value="slow">slow</option>
        <option value="slower">slower</option>
        <option value="veryslow">veryslow</option>
      </select>
    </div>
    <div>
      <label>Video bitrate</label>
      <input
        type="text"
        value={preset.video_bitrate ?? ""}
        oninput={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          preset.video_bitrate = v === "" ? null : v;
        }}
        placeholder="e.g. 2M (overrides CRF)"
      />
    </div>
    <div>
      <label>Audio bitrate</label>
      <input
        type="text"
        value={preset.audio_bitrate ?? ""}
        oninput={(e) => {
          const v = (e.currentTarget as HTMLInputElement).value;
          preset.audio_bitrate = v === "" ? null : v;
        }}
        placeholder="128k"
      />
    </div>
    <div class="full">
      <label class="inline">
        <input
          type="checkbox"
          checked={preset.use_cuda ?? false}
          onchange={(e) => {
            preset.use_cuda = (e.currentTarget as HTMLInputElement).checked;
          }}
        />
        Use NVIDIA NVENC (h264_nvenc) if available
      </label>
    </div>
  </div>
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px 12px;
    margin-bottom: 8px;
  }
  .grid .full { grid-column: 1 / -1; }
  .subhead {
    font-family: var(--font-display);
    font-size: var(--fs-14);
    font-weight: 600;
    margin: 10px 0 4px;
    color: var(--c-text);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  label.inline {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-13, 13px);
    color: var(--c-text);
    margin: 0;
  }
</style>
