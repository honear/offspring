<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { LogicalSize } from "@tauri-apps/api/dpi";
  import * as api from "$lib/api";

  let files = $state<string[]>([]);
  // Square-ish default: ceil(sqrt(N)). Filled in after files arrive
  // so the suggestion actually reflects this selection.
  let cols = $state(2);
  let layout = $state<"grid" | "mosaic">("grid");
  let encoding = $state(false);

  onMount(async () => {
    // Same window-reveal + foreground dance as the other dialogs (Trim,
    // Modify). Explorer-spawned offspring.exe windows otherwise come
    // up behind the calling Explorer window — toggling always-on-top +
    // set_focus moves the foreground to us; we drop always-on-top
    // immediately so the user can stack other windows over it later.
    const w = getCurrentWindow();
    try {
      await api.afterFirstPaint();
      await w.show();
      await w.unminimize();
      await w.setAlwaysOnTop(true);
      await w.setFocus();
      await w.setAlwaysOnTop(false);
    } catch {
      // Cosmetic only — dialog still works if any of these capabilities
      // is denied in a future config.
    }
    files = await api.getPendingFiles();
    if (files.length >= 3) {
      cols = Math.max(2, Math.ceil(Math.sqrt(files.length)));
    }
  });

  const rows = $derived(files.length > 0 ? Math.ceil(files.length / cols) : 0);
  const emptySlots = $derived(cols * rows - files.length);
  const valid = $derived(files.length >= 2 && cols >= 1 && cols <= files.length);

  async function startCompareGrid() {
    if (!valid) return;
    encoding = true;
    const c = Math.max(1, Math.min(files.length, Math.floor(cols)));
    try {
      await api.prepareCompareGridEncode(files);
    } catch (err) {
      encoding = false;
      alert(`Couldn't start compare grid:\n${err}`);
      return;
    }

    // Reshape THIS window into the progress window — same pattern
    // every dialog uses (opening a second webview in the same Tauri
    // instance is unreliable on Windows WebView2; blank-second-window
    // bug). The progress route reads `mode` / `cols` / `layout` from
    // the URL and dispatches to `encodeCompareGrid`.
    const w = getCurrentWindow();
    try {
      await w.setResizable(false);
      await w.setSize(new LogicalSize(420, 160));
      await w.setTitle("Offspring — Compare Grid");
      await w.setAlwaysOnTop(true);
    } catch {
      // Cosmetic only — fall through to navigation regardless.
    }
    const url = `/progress/?mode=compare-grid&cols=${c}&layout=${layout}`;
    await goto(url);
  }

  function cancel() {
    void getCurrentWindow().close();
  }
</script>

<main class="shell">
  <header>
    <h1>Compare grid</h1>
    <p class="muted tiny">
      Arrange {files.length} clip{files.length === 1 ? "" : "s"} into a
      grid for side-by-side review. Output keeps the first clip's aspect
      per cell and is named <code>&lt;first-stem&gt;_grid.mp4</code>.
    </p>
  </header>

  {#if files.length > 0}
    <div class="card files">
      <div class="tiny muted">{files.length} file{files.length === 1 ? "" : "s"}</div>
      {#each files as f}
        <div class="file">{f.split(/[\\/]/).pop()}</div>
      {/each}
    </div>
  {/if}

  <div class="card fields">
    <label class="field">
      <span>Columns ({rows} row{rows === 1 ? "" : "s"}{emptySlots > 0 ? `, ${emptySlots} empty slot${emptySlots === 1 ? "" : "s"} filled with black` : ""})</span>
      <input
        type="number"
        min="1"
        max={files.length}
        step="1"
        bind:value={cols}
        onkeydown={(e) => { if (e.key === "Enter") void startCompareGrid(); }}
      />
    </label>

    <fieldset class="layout">
      <legend>Layout</legend>
      <label class="inline">
        <input type="radio" name="layout" value="grid" bind:group={layout} />
        <span><strong>Grid</strong> — uniform cells, each clip aspect-fit with black bars</span>
      </label>
      <label class="inline">
        <input type="radio" name="layout" value="mosaic" bind:group={layout} />
        <span><strong>Mosaic</strong> — masonry pack: each clip keeps its aspect, columns fill shortest-first to minimise gaps</span>
      </label>
    </fieldset>
  </div>

  <footer class="bottom">
    <button class="ghost" onclick={cancel}>Cancel</button>
    <button class="primary" onclick={startCompareGrid} disabled={encoding || !valid}>
      {encoding ? "Starting…" : `Compare ${cols}×${rows}`}
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
  header h1 { font-size: var(--fs-16); margin-bottom: 0; }
  .files {
    max-height: 72px;
    overflow-y: auto;
    padding: 6px 10px;
  }
  .file {
    font-family: var(--font-mono);
    font-size: var(--fs-12);
    padding: 1px 0;
    color: var(--c-text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fields {
    padding: 10px 12px;
    overflow-y: auto;
    min-height: 0;
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field input[type="number"] {
    width: 100%;
    box-sizing: border-box;
  }
  .layout {
    border: 1px solid var(--c-border);
    border-radius: var(--r-md);
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .layout legend {
    padding: 0 4px;
    font-size: var(--fs-12);
    color: var(--c-text-2);
  }
  .inline {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .bottom {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: auto;
  }
</style>
