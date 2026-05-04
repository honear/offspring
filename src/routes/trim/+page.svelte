<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { LogicalSize } from "@tauri-apps/api/dpi";
  import * as api from "$lib/api";

  // Every dialog open starts at zero — no carrying yesterday's numbers
  // forward. Trim tends to be a one-shot decision per file, and a
  // stale "47" left over from another clip is more dangerous (silently
  // crops the wrong amount) than mildly inconvenient (have to retype).
  let startFrames = $state(0);
  let endFrames = $state(0);
  // Middle-range cut. Optional — the checkbox below toggles whether
  // these fields are emitted in prepare/encode calls.
  let removeEnabled = $state(false);
  let removeFrom = $state(0);
  let removeTo = $state(0);
  let files = $state<string[]>([]);
  let encoding = $state(false);

  onMount(async () => {
    // Reveal the window after WebView2 has committed the first paint
    // (see `afterFirstPaint` in `$lib/api` for the WHY). Right after
    // the show() we run the focus dance that used to live in Rust:
    // Explorer keeps the foreground when it spawns offspring.exe, so
    // a fresh window ends up behind it. Briefly toggling
    // always-on-top + set_focus moves the foreground to us; we drop
    // always-on-top right away so the user can later put another
    // window over the dialog if they want.
    const w = getCurrentWindow();
    try {
      await api.afterFirstPaint();
      await w.show();
      await w.unminimize();
      await w.setAlwaysOnTop(true);
      await w.setFocus();
      await w.setAlwaysOnTop(false);
    } catch {
      // Cosmetic — fall through so the dialog still works if any of
      // these capabilities is denied in a future config.
    }
    files = await api.getPendingFiles();
  });

  // The Trim button stays disabled when the inputs would be a no-op or
  // invalid. `valid` requires either ends-trim > 0 OR a properly
  // ordered middle cut to be active.
  const middleValid = $derived(
    removeEnabled &&
      Number.isFinite(removeFrom) &&
      Number.isFinite(removeTo) &&
      removeFrom >= 0 &&
      removeTo >= removeFrom,
  );
  const endsValid = $derived(
    Number.isFinite(startFrames) &&
      Number.isFinite(endFrames) &&
      startFrames >= 0 &&
      endFrames >= 0,
  );
  const valid = $derived(
    files.length > 0 &&
      endsValid &&
      (startFrames + endFrames > 0 || middleValid),
  );

  async function startTrim() {
    if (!valid) return;
    encoding = true;
    const start = Math.max(0, Math.floor(startFrames));
    const end = Math.max(0, Math.floor(endFrames));
    const rmFrom = middleValid ? Math.max(0, Math.floor(removeFrom)) : null;
    const rmTo = middleValid ? Math.max(0, Math.floor(removeTo)) : null;
    try {
      await api.prepareTrimEncode(files, start, end, rmFrom, rmTo);
    } catch (err) {
      encoding = false;
      alert(`Couldn't start trim:\n${err}`);
      return;
    }

    // Navigate THIS window to /progress/. Same trick the Custom dialog
    // uses — opening a second webview in the same Tauri instance is
    // unreliable on Windows WebView2 (blank second window).
    const w = getCurrentWindow();
    try {
      await w.setResizable(false);
      await w.setSize(new LogicalSize(420, 160));
      await w.setTitle("Offspring — Trimming");
      await w.setAlwaysOnTop(true);
    } catch {
      // Cosmetic only — fall through to the navigation regardless.
    }
    // URL params drive the progress route's encode dispatch. Middle-cut
    // fields are only added when active so a missing query param means
    // "no middle cut" without an empty-string ambiguity.
    let url = `/progress/?mode=trim&start=${start}&end=${end}`;
    if (rmFrom != null && rmTo != null) {
      url += `&from=${rmFrom}&to=${rmTo}`;
    }
    await goto(url);
  }

  function cancel() {
    void getCurrentWindow().close();
  }
</script>

<main class="shell">
  <header>
    <h1>Trim</h1>
    <p class="muted tiny">
      Strip frames from the start/end of each selected file, and/or cut a
      specific frame range out of the middle. Output keeps the source
      format with a <code>_trimmed</code> suffix.
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
    <!-- Side-by-side ends-trim row. The labels carry the full meaning
         ("Remove frames from start/end") so no helper text is needed
         underneath. -->
    <div class="field-row">
      <label class="field">
        <span>Remove frames from start</span>
        <input
          type="number"
          min="0"
          step="1"
          bind:value={startFrames}
          onkeydown={(e) => { if (e.key === "Enter") void startTrim(); }}
        />
      </label>

      <label class="field">
        <span>Remove frames from end</span>
        <input
          type="number"
          min="0"
          step="1"
          bind:value={endFrames}
          onkeydown={(e) => { if (e.key === "Enter") void startTrim(); }}
        />
      </label>
    </div>

    <!-- Optional middle-cut. Off by default; checking the box reveals
         the from/to inputs. -->
    <label class="inline">
      <input type="checkbox" bind:checked={removeEnabled} />
      <span>Remove a specific frame range</span>
    </label>

    {#if removeEnabled}
      <div class="field-row indent">
        <label class="field">
          <span>From frame (inclusive)</span>
          <input
            type="number"
            min="0"
            step="1"
            bind:value={removeFrom}
            onkeydown={(e) => { if (e.key === "Enter") void startTrim(); }}
          />
        </label>

        <label class="field">
          <span>To frame (inclusive)</span>
          <input
            type="number"
            min="0"
            step="1"
            bind:value={removeTo}
            onkeydown={(e) => { if (e.key === "Enter") void startTrim(); }}
          />
        </label>
      </div>
    {/if}
  </div>

  <footer class="bottom">
    <button class="ghost" onclick={cancel}>Cancel</button>
    <button class="primary" onclick={startTrim} disabled={encoding || !valid}>
      {encoding ? "Starting…" : `Trim ${files.length} file${files.length === 1 ? "" : "s"}`}
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
    flex: 1;
    min-width: 0;
  }
  .field input[type="number"] {
    width: 100%;
    box-sizing: border-box;
  }
  .field-row {
    display: flex;
    gap: 12px;
    align-items: flex-start;
  }
  .inline {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .indent {
    padding-left: 24px;
  }
  .bottom {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: auto;
  }
</style>
