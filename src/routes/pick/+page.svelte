<script lang="ts">
  // macOS Services picker. The flow:
  //   1. User selects files in Finder + right-clicks → Services → Offspring…
  //   2. Rust's NSServices provider opens this window with files queued
  //      via app.manage_pending_files
  //   3. We fetch the pending files + the enabled presets via Tauri,
  //      render the list, and on click route to either:
  //        - pick_run_preset (instant convert)
  //        - pick_run_tool   (opens a tool dialog window)
  //   4. Picker window closes itself after dispatch.
  //
  // Visual style is intentionally spartan — this isn't the main app,
  // it's a quick action picker. Looks closest to Spotlight / macOS
  // Services menu typography rather than the full Offspring UI.

  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import * as api from "$lib/api";
  import type { Preset } from "$lib/types";

  let files = $state<string[]>([]);
  let presets = $state<Preset[]>([]);
  let loading = $state(true);
  let busy = $state(false);
  let err = $state<string | null>(null);

  // Tools the picker offers. Mirrors the full Windows context-menu
  // surface (context_menu.rs) so feature parity is one-for-one. Each
  // entry maps to a `pick_run_tool` invocation; Rust either opens a
  // dialog (Modify, Trim) or runs the encoder directly + opens the
  // progress window (the remaining five).
  type ToolId =
    | "grayscale" | "overlay" | "merge" | "compare"
    | "trim" | "invert" | "make_square" | "modify";
  const TOOLS: { id: ToolId; label: string; hint: string }[] = [
    { id: "grayscale",   label: "Greyscale",    hint: "Convert to black and white" },
    { id: "overlay",     label: "Overlay",      hint: "Burn filename / timecode / custom text into the file" },
    { id: "merge",       label: "Merge",        hint: "Concatenate selected files into one (2+ videos)" },
    { id: "compare",     label: "Compare…",     hint: "Side-by-side (2 files) or grid (3+ files)" },
    { id: "trim",        label: "Trim…",        hint: "Set start / end seconds" },
    { id: "invert",      label: "Invert",       hint: "Flip RGB to create a negative (images only)" },
    { id: "make_square", label: "Make Square",  hint: "Pad shorter edge to a square output (images only)" },
    { id: "modify",      label: "Modify…",      hint: "Crop, rotate, flip, reverse, remove audio" },
  ];

  onMount(async () => {
    try {
      const [pending, allPresets] = await Promise.all([
        api.getPendingFiles(),
        api.listPresets(),
      ]);
      files = pending;
      presets = allPresets.filter((p) => p.enabled);
      loading = false;
      // Reveal the window after first paint to avoid a blank-frame
      // flash (matches the pattern used by other Offspring dialogs).
      const w = getCurrentWindow();
      await w.show();
      await w.setFocus();
    } catch (e) {
      err = String(e);
      loading = false;
    }
  });

  async function runPreset(p: Preset) {
    if (busy) return;
    busy = true;
    try {
      await api.pickRunPreset(files, p.id);
      await getCurrentWindow().close();
    } catch (e) {
      err = String(e);
      busy = false;
    }
  }

  async function runTool(tool: ToolId) {
    if (busy) return;
    busy = true;
    try {
      await api.pickRunTool(files, tool);
      await getCurrentWindow().close();
    } catch (e) {
      err = String(e);
      busy = false;
    }
  }
</script>

<main class="pick">
  <header>
    <h1>Offspring</h1>
    <p class="sub">
      {#if files.length === 0}
        No files selected
      {:else if files.length === 1}
        {fileName(files[0])}
      {:else}
        {files.length} files selected
      {/if}
    </p>
  </header>

  {#if loading}
    <p class="muted center">Loading…</p>
  {:else if err}
    <p class="err">{err}</p>
  {:else}
    <div class="columns">
      <section class="col col-presets">
        <h2>Presets</h2>
        {#if presets.length === 0}
          <p class="muted tiny">No presets enabled. Add one in Settings.</p>
        {:else}
          <ul>
            {#each presets as p (p.id)}
              <li>
                <button
                  class="row"
                  disabled={busy}
                  onclick={() => runPreset(p)}
                >
                  <span class="fmt-tag {p.format}">{p.format.toUpperCase()}</span>
                  <span class="name">{p.name}</span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      <section class="col col-tools">
        <h2>Tools</h2>
        <ul>
          {#each TOOLS as t (t.id)}
            <li>
              <button
                class="row tool"
                disabled={busy}
                onclick={() => runTool(t.id)}
              >
                <span class="name">{t.label}</span>
                <span class="hint">{t.hint}</span>
              </button>
            </li>
          {/each}
        </ul>
      </section>
    </div>
  {/if}
</main>

<script module lang="ts">
  function fileName(path: string): string {
    // Strip directories — path can be / or \ separated depending on
    // where Finder hands it over. Both work.
    const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    return i >= 0 ? path.slice(i + 1) : path;
  }
</script>

<style>
  :global(html), :global(body) {
    margin: 0;
    background: #1c1c1e;
    color: #f5f5f7;
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif;
    -webkit-font-smoothing: antialiased;
  }
  .pick {
    padding: 16px 18px 18px;
    display: flex;
    flex-direction: column;
    height: 100vh;
    box-sizing: border-box;
    gap: 12px;
  }
  .columns {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 18px;
    flex: 1;
    min-height: 0; /* let children scroll */
  }
  .col {
    min-height: 0;
    overflow-y: auto;
  }
  .col-presets {
    border-right: 1px solid #2c2c2e;
    padding-right: 14px;
  }
  .tiny {
    font-size: 11px;
    margin: 4px 0 0;
  }
  header h1 {
    font-size: 16px;
    margin: 0 0 2px;
    font-weight: 600;
  }
  .sub {
    margin: 0;
    font-size: 12px;
    color: #98989d;
  }
  h2 {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: #8e8e93;
    margin: 0 0 6px;
    font-weight: 600;
  }
  section {
    display: flex;
    flex-direction: column;
  }
  ul {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row {
    width: 100%;
    text-align: left;
    background: transparent;
    color: inherit;
    border: 0;
    padding: 8px 10px;
    border-radius: 6px;
    font: inherit;
    font-size: 13px;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .row:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.08);
  }
  .row:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .row.tool {
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
  }
  .fmt-tag {
    font-size: 10px;
    padding: 2px 5px;
    border-radius: 3px;
    background: #3a3a3c;
    color: #f5f5f7;
    font-weight: 600;
    letter-spacing: 0.03em;
  }
  .fmt-tag.mp4 { background: #0a84ff; }
  .fmt-tag.gif { background: #ff9f0a; color: #1c1c1e; }
  .fmt-tag.image { background: #30d158; color: #1c1c1e; }
  .name {
    flex: 1;
  }
  .hint {
    font-size: 11px;
    color: #8e8e93;
  }
  .muted {
    color: #8e8e93;
    font-size: 12px;
  }
  .center { text-align: center; padding: 20px; }
  .err {
    color: #ff453a;
    font-size: 12px;
  }
</style>
