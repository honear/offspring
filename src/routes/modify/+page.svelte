<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { goto } from "$app/navigation";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { LogicalSize } from "@tauri-apps/api/dpi";
  import { convertFileSrc } from "@tauri-apps/api/core";
  import * as api from "$lib/api";

  // ---- File + media state ---------------------------------------
  // The user's selection comes from pending-state. The PREVIEW is
  // sourced from the FIRST selected file; the same crop rect is
  // applied to every file at encode time.
  let files = $state<string[]>([]);
  let firstFile = $derived(files[0] ?? "");
  let previewSrc = $state<string>("");
  // Source dimensions in real pixels — drives every coordinate
  // mapping. Comes from a Rust ffprobe probe.
  let srcW = $state(0);
  let srcH = $state(0);
  let isImage = $state(false);
  // Filename-extension shortcut so the template can decide whether
  // to even bother trying <video>. Image extensions skip straight to
  // <img>.
  let imageExtensions = ["png", "jpg", "jpeg", "webp", "avif", "bmp", "tif", "tiff"];

  // ---- Preview mode ---------------------------------------------
  // "video" → <video> with native scrubbing
  // "image" → <img> for image inputs
  // "frame" → <img> showing a fallback JPEG extracted via ffmpeg
  //           (used when <video> errors out on exotic codecs)
  // "loading" → still resolving
  // "error" → couldn't probe / decode anything
  let previewMode = $state<"loading" | "video" | "image" | "frame" | "error">("loading");
  let previewError = $state<string | null>(null);

  // <video> element ref + scrub state
  let videoEl = $state<HTMLVideoElement | null>(null);
  let videoDuration = $state(0);
  let videoCurrentTime = $state(0);

  // ---- Crop rectangle (always in SOURCE pixels) -----------------
  // Defaults to the full frame so Cancel = no-op; the user reduces
  // it by dragging a corner/edge.
  let cropX = $state(0);
  let cropY = $state(0);
  let cropW = $state(0);
  let cropH = $state(0);
  // Aspect ratio constraint
  let aspectMode = $state<"free" | "original" | "16:9" | "9:16" | "1:1" | "4:3">("free");

  // ---- Modify-tool transform toggles ----------------------------
  // All default off so the dialog opens in a "no-op" state — the
  // user has to tick at least one box (or change the crop rect)
  // before Apply lights up.
  let flipH = $state(false);
  let flipV = $state(false);
  let reverse = $state(false);
  // Strip the audio track from the output. Hidden for image inputs
  // since they have no audio anyway, and counts as a "real" transform
  // for the validity check (so this can be applied alone).
  let removeAudio = $state(false);
  // Overwrite is the destructive option. Defaulting OFF + a visible
  // warning + the button text changing keeps it from being a footgun.
  let overwrite = $state(false);

  // Clamp helper — keeps every state mutation honest
  function clamp(v: number, lo: number, hi: number): number {
    return Math.max(lo, Math.min(hi, v));
  }

  // Compute the target aspect (width/height) for the current mode.
  // Returns null when "free" — caller skips the lock.
  function targetAspect(): number | null {
    switch (aspectMode) {
      case "free": return null;
      case "original": return srcW > 0 && srcH > 0 ? srcW / srcH : null;
      case "16:9": return 16 / 9;
      case "9:16": return 9 / 16;
      case "1:1": return 1;
      case "4:3": return 4 / 3;
    }
  }

  // ---- Stage layout (display ↔ source coordinate mapping) -------
  // The "stage" is the area inside the dialog where the preview is
  // drawn. We letterbox the source into it preserving aspect.
  let stageEl = $state<HTMLDivElement | null>(null);
  let stageW = $state(0);
  let stageH = $state(0);

  // Display rect of the preview content within the stage
  // (letterboxed). All `disp*` fields are CSS pixels.
  const fitScale = $derived.by(() => {
    if (srcW <= 0 || srcH <= 0 || stageW <= 0 || stageH <= 0) return 1;
    return Math.min(stageW / srcW, stageH / srcH);
  });
  const fitW = $derived(srcW * fitScale);
  const fitH = $derived(srcH * fitScale);
  const fitOffsetX = $derived((stageW - fitW) / 2);
  const fitOffsetY = $derived((stageH - fitH) / 2);

  // src px → display px (for the overlay rect we draw)
  function s2d(px: number, axis: "x" | "y"): number {
    if (axis === "x") return fitOffsetX + px * fitScale;
    return fitOffsetY + px * fitScale;
  }
  // Inverse: display-coord (relative to stage) → src px
  function d2s(px: number, axis: "x" | "y"): number {
    if (fitScale <= 0) return 0;
    if (axis === "x") return (px - fitOffsetX) / fitScale;
    return (px - fitOffsetY) / fitScale;
  }

  // ---- Mount: pull files, probe, set preview --------------------
  onMount(async () => {
    // Reveal the window after first paint + focus dance — same
    // pattern as Trim/Custom.
    const w = getCurrentWindow();
    try {
      await w.show();
      await w.unminimize();
      await w.setAlwaysOnTop(true);
      await w.setFocus();
      await w.setAlwaysOnTop(false);
    } catch {
      // Cosmetic only.
    }

    files = await api.getPendingFiles();
    if (files.length === 0) {
      previewMode = "error";
      previewError = "No files supplied to the Crop dialog.";
      return;
    }

    // Probe the first file's dimensions — the crop rect lives in
    // source pixels.
    const dims = await api.probeDimensions(firstFile);
    if (!dims) {
      previewMode = "error";
      previewError = "Could not read this file's dimensions.";
      return;
    }
    [srcW, srcH] = dims;
    // Default crop = full frame (no-op). User reduces it by dragging.
    cropX = 0;
    cropY = 0;
    cropW = srcW;
    cropH = srcH;

    // Pick preview source.
    const lower = firstFile.toLowerCase();
    isImage = imageExtensions.some((ext) => lower.endsWith("." + ext));
    previewSrc = convertFileSrc(firstFile);
    previewMode = isImage ? "image" : "video";
  });

  onDestroy(() => {
    // No global teardown required — temp preview frames are reaped
    // by OS on the next reboot's temp cleanup.
  });

  // ---- Video <error> fallback -----------------------------------
  // WebView2 can't decode some codecs (ProRes, DNxHD, exotic MKVs).
  // When `<video>` reports an error, fall back to a single ffmpeg-
  // extracted JPEG at 33% of the source's runtime — far enough in
  // that we skip the typical first-frame-is-black intro.
  async function handleVideoError() {
    try {
      // We don't know the duration here (the <video> errored before
      // metadata loaded), so guess 33% of a default 60 s. ffmpeg
      // accepts a seek time past the end and just returns the last
      // frame — fine for preview purposes.
      const path = await api.extractPreviewFrame(firstFile, 2.0);
      previewSrc = convertFileSrc(path);
      previewMode = "frame";
    } catch (err) {
      previewMode = "error";
      previewError = `Could not generate preview: ${err}`;
    }
  }

  // ---- Aspect-lock helpers --------------------------------------
  // When the user changes aspect mode mid-edit, snap the rect to
  // the new aspect from its current center, clamped into source
  // bounds.
  $effect(() => {
    const target = targetAspect();
    if (target == null || cropW <= 0 || cropH <= 0) return;
    const current = cropW / cropH;
    if (Math.abs(current - target) < 0.001) return;
    // Re-aspect around the rect's center.
    const cx = cropX + cropW / 2;
    const cy = cropY + cropH / 2;
    let newW: number;
    let newH: number;
    // Pick the orientation that fits inside the existing rect's
    // bounding circle while still hitting the target aspect.
    const fitFromW = { w: cropW, h: cropW / target };
    const fitFromH = { w: cropH * target, h: cropH };
    const pick = fitFromW.h <= cropH ? fitFromW : fitFromH;
    newW = Math.round(pick.w);
    newH = Math.round(pick.h);
    // Clamp into source bounds while maintaining aspect.
    if (newW > srcW) {
      newW = srcW;
      newH = Math.round(newW / target);
    }
    if (newH > srcH) {
      newH = srcH;
      newW = Math.round(newH * target);
    }
    cropW = newW;
    cropH = newH;
    cropX = clamp(Math.round(cx - newW / 2), 0, srcW - newW);
    cropY = clamp(Math.round(cy - newH / 2), 0, srcH - newH);
  });

  // ---- Pointer handlers for the crop overlay --------------------
  // We support: drag a handle to resize, drag the body to move.
  // No drag-to-create-from-empty — the rect always starts at
  // full-frame and the user reduces it.
  type DragMode =
    | { kind: "move"; startX: number; startY: number; origX: number; origY: number }
    | { kind: "resize"; handle: string; origX: number; origY: number; origW: number; origH: number };
  let drag = $state<DragMode | null>(null);

  function pointerToSource(e: PointerEvent): { x: number; y: number } {
    if (!stageEl) return { x: 0, y: 0 };
    const rect = stageEl.getBoundingClientRect();
    return {
      x: d2s(e.clientX - rect.left, "x"),
      y: d2s(e.clientY - rect.top, "y"),
    };
  }

  function startMove(e: PointerEvent) {
    e.preventDefault();
    const p = pointerToSource(e);
    drag = {
      kind: "move",
      startX: p.x,
      startY: p.y,
      origX: cropX,
      origY: cropY,
    };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function startResize(e: PointerEvent, handle: string) {
    e.preventDefault();
    e.stopPropagation();
    drag = {
      kind: "resize",
      handle,
      origX: cropX,
      origY: cropY,
      origW: cropW,
      origH: cropH,
    };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    if (!drag) return;
    const p = pointerToSource(e);
    if (drag.kind === "move") {
      const dx = p.x - drag.startX;
      const dy = p.y - drag.startY;
      cropX = clamp(Math.round(drag.origX + dx), 0, srcW - cropW);
      cropY = clamp(Math.round(drag.origY + dy), 0, srcH - cropH);
    } else {
      // Resize. Track which edges/corners moved and recompute the
      // rect from the original starting state + cursor delta.
      const aspect = targetAspect();
      let nx = drag.origX;
      let ny = drag.origY;
      let nw = drag.origW;
      let nh = drag.origH;
      const right = drag.origX + drag.origW;
      const bottom = drag.origY + drag.origH;

      const movesLeft = drag.handle.includes("w");
      const movesRight = drag.handle.includes("e");
      const movesTop = drag.handle.includes("n");
      const movesBottom = drag.handle.includes("s");

      if (movesRight) nw = clamp(p.x - drag.origX, 16, srcW - drag.origX);
      if (movesLeft) {
        const nlx = clamp(p.x, 0, right - 16);
        nx = nlx;
        nw = right - nlx;
      }
      if (movesBottom) nh = clamp(p.y - drag.origY, 16, srcH - drag.origY);
      if (movesTop) {
        const nty = clamp(p.y, 0, bottom - 16);
        ny = nty;
        nh = bottom - nty;
      }

      // Aspect-lock: the dragged primary axis drives, the
      // perpendicular derives from it. Corner handles use the
      // larger of the two candidate dimensions so the rect grows
      // toward where the cursor actually is.
      if (aspect != null) {
        const isCorner = (movesLeft || movesRight) && (movesTop || movesBottom);
        if (isCorner) {
          const fromW = nw / aspect;
          const fromH = nh * aspect;
          if (fromW <= nh) {
            nh = fromW;
          } else {
            nw = fromH;
          }
        } else if (movesLeft || movesRight) {
          // Edge resize on horizontal axis → derive height
          nh = nw / aspect;
        } else {
          // Edge resize on vertical axis → derive width
          nw = nh * aspect;
        }
        // After the aspect adjustment, clamp again into source bounds.
        if (movesLeft) nx = right - nw;
        if (movesTop) ny = bottom - nh;
        nx = clamp(nx, 0, srcW - nw);
        ny = clamp(ny, 0, srcH - nh);
        nw = Math.min(nw, srcW - nx);
        nh = Math.min(nh, srcH - ny);
      }

      cropX = Math.round(nx);
      cropY = Math.round(ny);
      cropW = Math.max(16, Math.round(nw));
      cropH = Math.max(16, Math.round(nh));
    }
  }

  function onPointerUp() {
    drag = null;
  }

  function resetCrop() {
    cropX = 0;
    cropY = 0;
    cropW = srcW;
    cropH = srcH;
  }

  // ---- Numeric input handlers (synced with overlay) -------------
  function setX(v: number) {
    cropX = clamp(Math.round(v), 0, Math.max(0, srcW - cropW));
  }
  function setY(v: number) {
    cropY = clamp(Math.round(v), 0, Math.max(0, srcH - cropH));
  }
  function setW(v: number) {
    cropW = clamp(Math.round(v), 16, srcW - cropX);
    const aspect = targetAspect();
    if (aspect != null) cropH = clamp(Math.round(cropW / aspect), 16, srcH - cropY);
  }
  function setH(v: number) {
    cropH = clamp(Math.round(v), 16, srcH - cropY);
    const aspect = targetAspect();
    if (aspect != null) cropW = clamp(Math.round(cropH * aspect), 16, srcW - cropX);
  }

  // ---- Submit / cancel ------------------------------------------
  let encoding = $state(false);
  // The crop rect is "active" only when it differs from the full
  // frame. Flips/reverse can carry the modify on their own; cropping
  // a full frame to itself is a no-op we shouldn't bother encoding.
  const cropActive = $derived(
    cropW > 0 && cropH > 0 &&
    (cropX !== 0 || cropY !== 0 || cropW !== srcW || cropH !== srcH),
  );
  const valid = $derived(
    files.length > 0 &&
      srcW > 0 &&
      srcH > 0 &&
      (cropActive || flipH || flipV || reverse || removeAudio),
  );

  async function startModify() {
    if (!valid) return;
    encoding = true;
    try {
      await api.prepareModifyEncode(files);
    } catch (err) {
      encoding = false;
      alert(`Couldn't start modify:\n${err}`);
      return;
    }
    const w = getCurrentWindow();
    try {
      await w.setResizable(false);
      await w.setSize(new LogicalSize(420, 160));
      await w.setTitle("Offspring — Modifying");
      await w.setAlwaysOnTop(true);
    } catch {
      // Cosmetic.
    }
    // URL params carry the full ModifySpec to the progress route.
    // crop_w=0 / crop_h=0 means "no crop, transforms only".
    const cw = cropActive ? cropW : 0;
    const ch = cropActive ? cropH : 0;
    const cx = cropActive ? cropX : 0;
    const cy = cropActive ? cropY : 0;
    const params = new URLSearchParams({
      mode: "modify",
      x: String(cx),
      y: String(cy),
      w: String(cw),
      h: String(ch),
      fh: flipH ? "1" : "0",
      fv: flipV ? "1" : "0",
      rev: reverse ? "1" : "0",
      ra: removeAudio ? "1" : "0",
      ow: overwrite ? "1" : "0",
    });
    await goto(`/progress/?${params.toString()}`);
  }

  function cancel() {
    void getCurrentWindow().close();
  }
</script>

<main class="shell">
  <header>
    <h1>Modify</h1>
    <p class="muted tiny">
      {#if files.length > 1}
        Drag the handles to crop, then pick any flips / reverse below. The
        same transforms apply to all {files.length} selected files.
      {:else}
        Drag the handles to crop, or pick a flip / reverse from the
        toggles below.
      {/if}
    </p>
  </header>

  <div class="stage" bind:this={stageEl} bind:clientWidth={stageW} bind:clientHeight={stageH}
       onpointermove={onPointerMove} onpointerup={onPointerUp} onpointercancel={onPointerUp}>
    {#if previewMode === "loading"}
      <div class="placeholder muted">Loading preview…</div>
    {:else if previewMode === "error"}
      <div class="placeholder error">{previewError ?? "Preview unavailable."}</div>
    {:else if previewMode === "video"}
      <video
        bind:this={videoEl}
        bind:duration={videoDuration}
        bind:currentTime={videoCurrentTime}
        src={previewSrc}
        muted
        preload="auto"
        onerror={handleVideoError}
        style="left: {fitOffsetX}px; top: {fitOffsetY}px; width: {fitW}px; height: {fitH}px;"
      ></video>
    {:else}
      <!-- image or frame fallback -->
      <img
        src={previewSrc}
        alt="preview"
        style="left: {fitOffsetX}px; top: {fitOffsetY}px; width: {fitW}px; height: {fitH}px;"
      />
    {/if}

    {#if srcW > 0 && srcH > 0 && previewMode !== "loading" && previewMode !== "error"}
      <!-- The crop overlay — drawn on top of the preview at the same
           coordinates. Body drag = move; handle drag = resize. -->
      <div
        class="crop-rect"
        style="left: {s2d(cropX, 'x')}px; top: {s2d(cropY, 'y')}px; width: {cropW * fitScale}px; height: {cropH * fitScale}px;"
        onpointerdown={startMove}
      >
        {#each ["nw","n","ne","e","se","s","sw","w"] as h (h)}
          <div class="handle {h}" onpointerdown={(e) => startResize(e, h)}></div>
        {/each}
      </div>

      <!-- Darken the area outside the crop so it reads as "cropped
           away". Four absolutely-positioned strips covering the
           non-cropped regions. -->
      <div class="dim top" style="left: 0; top: 0; width: 100%; height: {s2d(cropY, 'y')}px;"></div>
      <div class="dim bottom" style="left: 0; top: {s2d(cropY + cropH, 'y')}px; width: 100%; bottom: 0;"></div>
      <div class="dim left" style="left: 0; top: {s2d(cropY, 'y')}px; width: {s2d(cropX, 'x')}px; height: {cropH * fitScale}px;"></div>
      <div class="dim right" style="left: {s2d(cropX + cropW, 'x')}px; top: {s2d(cropY, 'y')}px; right: 0; height: {cropH * fitScale}px;"></div>
    {/if}
  </div>

  {#if previewMode === "video" && videoDuration > 0}
    <div class="scrub">
      <input
        type="range"
        min="0"
        max={videoDuration}
        step="0.01"
        bind:value={videoCurrentTime}
      />
      <span class="tiny muted">
        {videoCurrentTime.toFixed(2)} / {videoDuration.toFixed(2)} s
      </span>
    </div>
  {:else if previewMode === "frame"}
    <div class="scrub muted tiny">
      Preview limited for this format — cropping the original works correctly.
    </div>
  {/if}

  <div class="controls">
    <label class="control">
      <span>Aspect</span>
      <select bind:value={aspectMode}>
        <option value="free">Free</option>
        <option value="original">Original</option>
        <option value="16:9">16:9</option>
        <option value="9:16">9:16</option>
        <option value="1:1">1:1</option>
        <option value="4:3">4:3</option>
      </select>
    </label>
    <label class="control">
      <span>X</span>
      <input
        type="number" min="0" step="1"
        value={cropX}
        oninput={(e) => setX(parseInt((e.currentTarget as HTMLInputElement).value, 10) || 0)}
      />
    </label>
    <label class="control">
      <span>Y</span>
      <input
        type="number" min="0" step="1"
        value={cropY}
        oninput={(e) => setY(parseInt((e.currentTarget as HTMLInputElement).value, 10) || 0)}
      />
    </label>
    <label class="control">
      <span>W</span>
      <input
        type="number" min="16" step="1"
        value={cropW}
        oninput={(e) => setW(parseInt((e.currentTarget as HTMLInputElement).value, 10) || 16)}
      />
    </label>
    <label class="control">
      <span>H</span>
      <input
        type="number" min="16" step="1"
        value={cropH}
        oninput={(e) => setH(parseInt((e.currentTarget as HTMLInputElement).value, 10) || 16)}
      />
    </label>
    <button class="ghost reset" onclick={resetCrop} title="Reset to full frame">Reset</button>
  </div>

  <!-- Transform toggles. Reverse is hidden for image inputs since
       there's nothing to reverse on a single frame. -->
  <div class="toggles">
    <label class="toggle">
      <input type="checkbox" bind:checked={flipH}>
      <span>Flip horizontal</span>
    </label>
    <label class="toggle">
      <input type="checkbox" bind:checked={flipV}>
      <span>Flip vertical</span>
    </label>
    {#if !isImage}
      <label class="toggle" title="Reverses video frames (and audio when present). Buffers all frames in memory — slow on long clips.">
        <input type="checkbox" bind:checked={reverse}>
        <span>Reverse</span>
      </label>
      <label class="toggle" title="Strip the audio track from the output. Useful for muting clips before sharing.">
        <input type="checkbox" bind:checked={removeAudio}>
        <span>Remove audio</span>
      </label>
    {/if}
    <label class="toggle danger" title="Replace the source file with the modified version. Cannot be undone.">
      <input type="checkbox" bind:checked={overwrite}>
      <span>⚠ Overwrite original</span>
    </label>
  </div>

  <footer class="bottom">
    <button class="ghost" onclick={cancel}>Cancel</button>
    <button class="primary" class:destructive={overwrite} onclick={startModify} disabled={encoding || !valid}>
      {#if encoding}
        Starting…
      {:else if overwrite}
        Overwrite {files.length} file{files.length === 1 ? "" : "s"}
      {:else}
        Apply to {files.length} file{files.length === 1 ? "" : "s"}
      {/if}
    </button>
  </footer>
</main>

<style>
  .shell {
    padding: 12px 14px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    height: 100vh;
    box-sizing: border-box;
    background: var(--c-surface);
  }
  header h1 {
    font-size: var(--fs-16);
    margin-bottom: 0;
  }
  .stage {
    position: relative;
    flex: 1;
    min-height: 200px;
    background: var(--c-surface-2, #111);
    border-radius: 6px;
    overflow: hidden;
    user-select: none;
    touch-action: none;
  }
  .stage video,
  .stage img {
    position: absolute;
    object-fit: contain;
    pointer-events: none;
  }
  .placeholder {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 16px;
  }
  .placeholder.error {
    color: var(--c-error, #d04a4a);
  }
  .crop-rect {
    position: absolute;
    border: 1.5px solid #fff;
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.45);
    cursor: move;
    box-sizing: border-box;
  }
  .handle {
    position: absolute;
    width: 12px;
    height: 12px;
    background: #fff;
    border: 1px solid #000;
    box-sizing: border-box;
  }
  .handle.nw { left: -6px;  top: -6px;    cursor: nwse-resize; }
  .handle.n  { left: 50%;   top: -6px;    transform: translateX(-50%); cursor: ns-resize; }
  .handle.ne { right: -6px; top: -6px;    cursor: nesw-resize; }
  .handle.e  { right: -6px; top: 50%;     transform: translateY(-50%); cursor: ew-resize; }
  .handle.se { right: -6px; bottom: -6px; cursor: nwse-resize; }
  .handle.s  { left: 50%;   bottom: -6px; transform: translateX(-50%); cursor: ns-resize; }
  .handle.sw { left: -6px;  bottom: -6px; cursor: nesw-resize; }
  .handle.w  { left: -6px;  top: 50%;     transform: translateY(-50%); cursor: ew-resize; }
  .dim {
    position: absolute;
    background: rgba(0, 0, 0, 0.55);
    pointer-events: none;
  }
  .scrub {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 4px;
  }
  .scrub input[type="range"] {
    flex: 1;
  }
  .controls {
    display: flex;
    gap: 8px;
    align-items: flex-end;
    flex-wrap: wrap;
  }
  .control {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--fs-12);
  }
  .control input[type="number"],
  .control select {
    width: 80px;
  }
  .control:nth-child(1) select { width: 110px; }
  .reset {
    align-self: flex-end;
  }
  .toggles {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 16px;
    padding: 4px 4px 0;
  }
  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-13, 13px);
    color: var(--c-text);
    user-select: none;
    cursor: pointer;
  }
  .toggle.danger {
    color: var(--c-error, #c83838);
  }
  .bottom {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: auto;
  }
  /* Make the "overwrite" path visually distinct so accidental clicks
     are less likely. The button stays inside the same primary slot
     so layout doesn't reflow. */
  .primary.destructive {
    background: var(--c-error, #c83838);
    color: white;
  }
</style>
