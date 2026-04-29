<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
  import * as api from "$lib/api";
  import type { ProgressEvent, Preset } from "$lib/types";

  let ev = $state<ProgressEvent | null>(null);
  let finished = $state(false);
  let errored = $state(false);
  let errorMsg = $state<string | null>(null);
  let totalDone = $state(0);

  // Step the mount lifecycle goes through. Surfaced in the UI while the
  // window is still in its pre-progress state so a stall is visibly
  // localized instead of just showing "Preparing…" forever.
  let phase = $state<string>("mount");

  // When the failure is "ffmpeg.exe not found…" we offer the user an
  // inline one-click download path so they don't have to hunt for the
  // Settings tab after a failed right-click conversion.
  let dl = $state<{
    active: boolean;
    phase: string;
    percent: number | null;
    message: string | null;
    done: boolean;
  }>({ active: false, phase: "", percent: null, message: null, done: false });
  const ffmpegMissing = $derived(
    errored && !!errorMsg && errorMsg.toLowerCase().includes("ffmpeg.exe not found"),
  );

  // Grow the toast a little when we reveal the FFmpeg-missing flow so the
  // download button + progress bar aren't cramped under a 140px window.
  let didGrow = false;
  $effect(() => {
    if ((ffmpegMissing || dl.active) && !didGrow) {
      didGrow = true;
      void getCurrentWindow().setSize(new LogicalSize(420, 200));
    }
  });

  // Auto-close delay after the finish event. Short enough to feel snappy,
  // long enough to glimpse the "Done" tick.
  const CLOSE_DELAY_MS = 1200;
  const ERROR_CLOSE_DELAY_MS = 3000;
  // Hard safety net: if nothing has happened for this long, close anyway.
  // Prevents the window from sitting there forever if the backend stalls
  // (e.g. FFmpeg command hung) or if encode-finished was missed somehow.
  const STALL_CLOSE_MS = 10 * 60 * 1000;

  let closeTimer: ReturnType<typeof setTimeout> | null = null;
  let stallTimer: ReturnType<typeof setTimeout> | null = null;
  let didStart = false;

  async function closeWindow() {
    // Try a graceful close first. If that fails — Tauri ACL rejection,
    // a close-requested handler swallowing the request, whatever — fall
    // through to destroy() which is non-interceptable. Surface any error
    // so we don't silently hang on a "Done" screen forever.
    const w = getCurrentWindow();
    try {
      await w.close();
    } catch (err) {
      console.error("[progress] close() failed, trying destroy():", err);
      try {
        await w.destroy();
      } catch (err2) {
        console.error("[progress] destroy() also failed:", err2);
      }
    }
  }

  function closeSoon(delay: number) {
    if (closeTimer) return; // already scheduled
    closeTimer = setTimeout(() => {
      void closeWindow();
    }, delay);
  }

  function armStallTimer() {
    if (stallTimer) clearTimeout(stallTimer);
    stallTimer = setTimeout(() => {
      // Nothing happened for a long time — bail.
      errored = true;
      errorMsg = "Encoding appears to have stalled.";
      finished = true;
      closeSoon(ERROR_CLOSE_DELAY_MS);
    }, STALL_CLOSE_MS);
  }

  function finish(total: number) {
    if (finished) return;
    finished = true;
    totalDone = total;
    if (stallTimer) {
      clearTimeout(stallTimer);
      stallTimer = null;
    }
    // If we can offer an inline fix (download FFmpeg), don't auto-close —
    // let the user click the button. Otherwise auto-close as before.
    if (!ffmpegMissing) {
      closeSoon(errored ? ERROR_CLOSE_DELAY_MS : CLOSE_DELAY_MS);
    }
  }

  async function downloadFfmpeg() {
    dl = { active: true, phase: "starting", percent: 0, message: "Starting…", done: false };
    // Prevent the error-close from firing behind the download.
    if (closeTimer) { clearTimeout(closeTimer); closeTimer = null; }
    await api.onFfmpegDownload((e) => {
      dl.phase = e.phase;
      dl.percent = e.percent;
      dl.message = e.message;
      if (e.phase === "done") {
        dl.active = false;
        dl.done = true;
        // FFmpeg is now installed; close the progress window so the user
        // can retry the SendTo invocation. (We don't re-run automatically
        // because that would surprise the user.)
        setTimeout(() => { void closeWindow(); }, 900);
      } else if (e.phase === "error") {
        dl.active = false;
        errorMsg = e.message ?? "FFmpeg download failed";
      } else {
        dl.active = true;
      }
    });
    try {
      await api.downloadFfmpeg();
    } catch (err) {
      dl.active = false;
      errorMsg = String(err);
    }
  }

  onMount(async () => {
    // Outer try/catch so an unexpected throw in the mount chain surfaces
    // to the UI instead of leaving the window frozen at "Preparing…".
    try {
      phase = "subscribing";
      // Subscribe first so we don't miss early events.
      await api.onProgress((e) => {
        ev = e;
        armStallTimer();
        if (e.stage === "error") {
          errored = true;
          errorMsg = e.message ?? errorMsg;
        }
        // Fallback: if the very last file reports a per-file "done", schedule
        // a graceful close even if `encode-finished` never arrives. We give
        // the backend a short grace window to emit the real finished event.
        if (e.stage === "done" && e.file_index === e.total_files) {
          setTimeout(() => {
            if (!finished) finish(e.total_files);
          }, 400);
        }
      });
      await api.onFinished((total) => finish(total));

      // Resolve pending preset + files and kick off.
      // Modes:
      //   - Preset (SendTo): pending preset_id is set; look it up in list.
      //   - Custom:           pending custom_preset holds the full preset.
      //   - Trim:             URL has ?mode=trim&start=N&end=M (the Trim
      //                       dialog navigates here in-place after writing
      //                       trim_last.json + stashing files).
      phase = "resolving preset";
      let files: string[] = [];
      let preset: Preset | undefined;
      const search = new URLSearchParams(window.location.search);
      const isTrim = search.get("mode") === "trim";
      const trimStart = Math.max(0, parseInt(search.get("start") ?? "0", 10) || 0);
      const trimEnd = Math.max(0, parseInt(search.get("end") ?? "0", 10) || 0);
      // Optional middle-range cut. Both `from` and `to` must be present
      // for the cut to take effect; missing or partial → no cut.
      const fromParam = search.get("from");
      const toParam = search.get("to");
      const trimRemoveFrom = fromParam != null ? Math.max(0, parseInt(fromParam, 10) || 0) : null;
      const trimRemoveTo = toParam != null ? Math.max(0, parseInt(toParam, 10) || 0) : null;
      const [f, presetId, customPreset, allPresets, mergeFlag, grayscaleFlag, compareFlag, overlayFlag] = await Promise.all([
        api.getPendingFiles(),
        api.getPendingPresetId(),
        api.getPendingCustomPreset(),
        api.listPresets(),
        api.getPendingMerge(),
        api.getPendingGrayscale(),
        api.getPendingCompare(),
        api.getPendingOverlay(),
      ]);
      files = f;
      const isMerge = mergeFlag === true;
      const isGrayscale = grayscaleFlag === true;
      const isCompare = compareFlag === true;
      const isOverlay = overlayFlag === true;
      if (presetId) {
        preset = allPresets.find((p) => p.id === presetId);
      } else if (customPreset) {
        preset = customPreset;
      }

      if (files.length === 0) {
        // Nothing to do — close immediately so we don't leave a blank dialog.
        phase = "no files";
        closeSoon(0);
        return;
      }
      // Tool paths derive their own settings from the inputs, so they
      // don't need a resolved preset.
      const isTool = isMerge || isGrayscale || isCompare || isOverlay || isTrim;
      if (!isTool && !preset) {
        errored = true;
        errorMsg = `No preset was resolved (presetId=${presetId ?? "null"}, customPreset=${customPreset ? "present" : "null"}). The shortcut may be stale.`;
        finish(0);
        return;
      }
      if (isMerge && files.length < 2) {
        errored = true;
        errorMsg = "Merge needs at least two files. Select two or more and try again.";
        finish(0);
        return;
      }
      if (isCompare && files.length < 2) {
        errored = true;
        errorMsg = "Compare needs at least two files. Select two or more and try again.";
        finish(0);
        return;
      }

      phase = isTrim
        ? "starting trim"
        : isMerge
          ? "starting merge"
          : isGrayscale
            ? "starting greyscale"
            : isCompare
              ? "starting compare"
              : isOverlay
                ? "starting overlay"
                : "starting encode";
      armStallTimer();
      didStart = true;
      if (isTrim) {
        // A trim job is valid if it strips ANY frames OR cuts a middle
        // range. Both being absent means the user invoked the route
        // directly without filling out the dialog.
        const hasMiddle = trimRemoveFrom != null && trimRemoveTo != null && trimRemoveTo >= trimRemoveFrom;
        if (trimStart === 0 && trimEnd === 0 && !hasMiddle) {
          errored = true;
          errorMsg = "Trim was started without any frames to remove. Open the Trim dialog and enter values.";
          finish(0);
          return;
        }
        await api.encodeTrim(files, trimStart, trimEnd, trimRemoveFrom, trimRemoveTo);
      } else if (isMerge) {
        // Merge derives its own settings from the first file — no
        // preset is sent through the wire.
        await api.encodeMerge(files);
      } else if (isGrayscale) {
        await api.encodeGrayscale(files);
      } else if (isCompare) {
        await api.encodeCompare(files);
      } else if (isOverlay) {
        await api.encodeOverlay(files);
      } else {
        await api.encode(files, preset!);
      }
      phase = isTrim
        ? "trimming"
        : isMerge
          ? "merging"
          : isGrayscale
            ? "greyscaling"
            : isCompare
              ? "stacking"
              : isOverlay
                ? "overlaying"
                : "encoding";
    } catch (err) {
      // Anything that throws — listen(), invoke() failure, serde rejection,
      // FFmpeg-not-found — lands here and surfaces to the user.
      errored = true;
      errorMsg = `${phase} failed: ${String(err)}`;
      finish(0);
    }
  });

  function close() {
    void closeWindow();
  }
</script>

<main class="toast">
  {#if finished}
    <div class="state done">
      <div class="mark" class:err={errored}>{errored ? "!" : "✓"}</div>
      <div class="state-text">
        <h2>
          {#if dl.done}
            FFmpeg installed
          {:else if ffmpegMissing}
            FFmpeg is missing
          {:else if errored}
            Finished with errors
          {:else}
            Done
          {/if}
        </h2>
        <p class="muted">
          {#if dl.done}
            Re-send your files to finish the conversion.
          {:else if ffmpegMissing}
            Offspring needs FFmpeg (~80 MB) to convert. Download it now?
          {:else if errored && errorMsg}
            {errorMsg}
          {:else}
            {totalDone} file{totalDone === 1 ? "" : "s"} processed
          {/if}
        </p>

        {#if ffmpegMissing && !dl.active && !dl.done}
          <div class="actions-row">
            <button class="primary" onclick={downloadFfmpeg}>Download FFmpeg</button>
            <button class="ghost" onclick={close}>Later</button>
          </div>
        {:else if dl.active}
          <div class="dl-progress">
            <div class="row between">
              <span class="tiny muted">
                {dl.phase === "downloading" ? "Downloading…" :
                 dl.phase === "extracting" ? "Extracting…" : dl.phase}
              </span>
              <span class="tiny muted">
                {dl.percent != null ? Math.round(dl.percent) + "%" : ""}
              </span>
            </div>
            <div class="bar">
              <div
                class="fill"
                class:indet={dl.percent == null}
                style={dl.percent != null ? `width: ${Math.round(dl.percent)}%;` : ""}
              ></div>
            </div>
            {#if dl.message}<p class="tiny muted">{dl.message}</p>{/if}
          </div>
        {/if}
      </div>
    </div>
  {:else if ev}
    <header>
      <span class="tiny muted">FILE {ev.file_index} OF {ev.total_files}</span>
      <button class="ghost tiny-btn" onclick={close} aria-label="Close">✕</button>
    </header>
    <div class="filename" title={ev.input}>
      {ev.input.split(/[\\/]/).pop()}
    </div>
    <div class="bar">
      <div
        class="fill"
        style="width: {ev.percent != null ? Math.round(ev.percent * 100) : 0}%;"
      ></div>
    </div>
    <div class="row between">
      <span class="tiny muted">{ev.stage === "palette" ? "Generating palette…" : ev.message ?? "Encoding"}</span>
      <span class="tiny muted">{ev.percent != null ? Math.round(ev.percent * 100) + "%" : ""}</span>
    </div>
  {:else}
    <p class="muted">{didStart ? "Starting…" : `Preparing… (${phase})`}</p>
  {/if}
</main>

<style>
  .toast {
    padding: 10px 14px;
    height: 100vh;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: var(--c-surface);
    border: 1px solid var(--c-border);
  }
  header { display: flex; justify-content: space-between; align-items: center; }
  .filename {
    font-family: var(--font-display);
    font-size: var(--fs-14);
    font-weight: 600;
    color: var(--c-text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .bar {
    height: 6px;
    background: var(--c-surface-3);
    border-radius: 999px;
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--c-primary);
    transition: width 200ms ease;
  }
  .row.between { display: flex; justify-content: space-between; }
  .tiny-btn {
    padding: 0 6px;
    min-height: 0;
    font-size: var(--fs-12);
    background: transparent;
    border: none;
    color: var(--c-text-3);
  }
  .tiny-btn:hover { color: var(--c-text); background: var(--c-surface-2); }
  .state {
    display: flex; align-items: flex-start; gap: 10px;
    padding: 10px 2px;
  }
  .state-text { min-width: 0; flex: 1; }
  .state-text p {
    white-space: normal;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    font-size: var(--fs-12);
  }
  .mark {
    width: 28px; height: 28px;
    border-radius: 50%;
    background: var(--c-success);
    color: #fff;
    display: flex; align-items: center; justify-content: center;
    font-size: 16px;
    font-weight: 700;
    flex-shrink: 0;
  }
  .mark.err { background: var(--c-danger); }
  .state h2 { font-size: var(--fs-16); margin-bottom: 1px; }

  .actions-row {
    display: flex;
    gap: 6px;
    margin-top: 8px;
  }
  .actions-row button { font-size: var(--fs-12); padding: 4px 12px; min-height: 0; }

  .dl-progress {
    margin-top: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .dl-progress .bar {
    height: 5px;
    background: var(--c-surface-3);
    border-radius: var(--r-pill);
    overflow: hidden;
  }
  .dl-progress .fill {
    height: 100%;
    background: var(--c-primary);
    transition: width 200ms ease;
  }
  .dl-progress .fill.indet {
    width: 40%;
    animation: slide 1.2s ease-in-out infinite;
  }
  @keyframes slide {
    0%   { transform: translateX(-100%); }
    100% { transform: translateX(250%); }
  }
</style>
