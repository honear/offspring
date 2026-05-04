<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { LogicalSize } from "@tauri-apps/api/dpi";
  import FormatFields from "$lib/components/FormatFields.svelte";
  import * as api from "$lib/api";
  import type { Preset } from "$lib/types";

  let preset = $state<Preset | null>(null);
  let files = $state<string[]>([]);
  let encoding = $state(false);

  onMount(async () => {
    // Reveal the window once WebView2 has actually painted the first
    // frame. `onMount` runs as soon as the DOM is built, which is one
    // or two frames before pixels reach the screen — calling `show()`
    // earlier than that is what produced the brief blank-window flash.
    // See `afterFirstPaint` in `$lib/api`.
    void api.afterFirstPaint().then(() =>
      getCurrentWindow().show().catch(() => {}),
    );
    preset = await api.getCustomLast();
    files = await api.getPendingFiles();
  });

  async function startEncode() {
    if (!preset) return;
    encoding = true;
    // Svelte 5 wraps `preset` in a reactive $state proxy. Passing proxies
    // across the Tauri IPC boundary is fragile, so take a plain snapshot.
    const snap = $state.snapshot(preset) as Preset;
    try {
      await api.saveCustomLast(snap);
      // Stash files + preset in app state so the progress route can pick
      // them up on mount.
      await api.prepareCustomEncode(files, snap);
    } catch (err) {
      encoding = false;
      alert(`Couldn't start encode:\n${err}`);
      return;
    }

    // Reuse THIS window instead of opening a new "progress" window. On
    // Windows WebView2, opening a second webview in the same Tauri
    // instance while the first is still alive has been unreliable —
    // the second window shows up blank white. Navigating the existing
    // webview via SvelteKit's client-side router is instant, keeps one
    // webview alive for the whole session, and sidesteps the bug.
    const w = getCurrentWindow();
    try {
      await w.setResizable(false);
      await w.setSize(new LogicalSize(420, 160));
      await w.setTitle("Offspring — Encoding");
      await w.setAlwaysOnTop(true);
    } catch {
      // Resizing/re-titling is cosmetic; if it fails (e.g. capability
      // denied in a future config), we still want to navigate.
    }
    await goto("/progress/");
  }

  function cancel() {
    const w = getCurrentWindow();
    w.close();
  }
</script>

<main class="shell">
  <header>
    <h1>Custom conversion</h1>
    <p class="muted tiny">Tweak settings — starts from your last-used values.</p>
  </header>

  {#if files.length > 0}
    <div class="card files">
      <div class="tiny muted">{files.length} file{files.length === 1 ? "" : "s"}</div>
      {#each files as f}
        <div class="file">{f.split(/[\\/]/).pop()}</div>
      {/each}
    </div>
  {/if}

  {#if preset}
    <div class="card fields">
      <FormatFields {preset} />
    </div>

    <footer class="bottom">
      <button class="ghost" onclick={cancel}>Cancel</button>
      <button class="primary" onclick={startEncode} disabled={encoding || files.length === 0}>
        {encoding ? "Starting…" : `Encode ${files.length} file${files.length === 1 ? "" : "s"}`}
      </button>
    </footer>
  {:else}
    <p class="muted">Loading…</p>
  {/if}
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
  .fields { padding: 10px 12px; overflow-y: auto; min-height: 0; flex: 1; }
  .bottom {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: auto;
  }
</style>
