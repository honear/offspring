<script lang="ts">
  import { onMount } from "svelte";
  import FormatFields from "$lib/components/FormatFields.svelte";
  import * as api from "$lib/api";
  import type { Preset, Settings, FfmpegStatus } from "$lib/types";

  let presets = $state<Preset[]>([]);
  let selectedId = $state<string | null>(null);
  let settings = $state<Settings>({});
  let ffmpeg = $state<FfmpegStatus>({ found: false, path: null });
  let tab = $state<"presets" | "settings">("presets");
  let dirty = $state(false);
  let saving = $state(false);
  let savedTick = $state(0);

  // FFmpeg download state (fed by the `ffmpeg-download` event from Rust)
  let dl = $state<{
    active: boolean;
    phase: string;
    percent: number | null;
    message: string | null;
    error: string | null;
  }>({ active: false, phase: "", percent: null, message: null, error: null });

  const selected = $derived(presets.find((p) => p.id === selectedId) ?? null);

  onMount(async () => {
    await reload();

    // Subscribe to FFmpeg download events so the Settings pane can show
    // progress inline and flip the header badge when the install completes.
    await api.onFfmpegDownload(async (e) => {
      dl.phase = e.phase;
      dl.percent = e.percent;
      dl.message = e.message;
      if (e.phase === "error") {
        dl.active = false;
        dl.error = e.message ?? "Download failed";
      } else if (e.phase === "done") {
        dl.active = false;
        dl.error = null;
        // Re-check status so the header badge flips green immediately.
        ffmpeg = await api.ffmpegStatus();
      } else {
        dl.active = true;
        dl.error = null;
      }
    });
  });

  async function startDownloadFfmpeg() {
    dl = { active: true, phase: "starting", percent: 0, message: "Starting…", error: null };
    try {
      await api.downloadFfmpeg();
    } catch (err) {
      dl.active = false;
      dl.error = String(err);
    }
  }

  async function reload() {
    presets = await api.listPresets();
    settings = await api.getSettings();
    ffmpeg = await api.ffmpegStatus();
    if (!selectedId && presets.length > 0) selectedId = presets[0].id;
    // First-run guidance: if FFmpeg is missing on app open, surface the
    // Settings tab directly so the big "Download FFmpeg" button is the
    // first thing they see instead of a silently-broken app.
    if (!ffmpeg.found) tab = "settings";
  }

  function genId(name: string): string {
    const base = name.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "");
    let id = base || "preset";
    let n = 1;
    while (presets.some((p) => p.id === id)) {
      n++;
      id = `${base}_${n}`;
    }
    return id;
  }

  function addPreset() {
    const fresh: Preset = {
      id: genId("new_preset"),
      name: "New preset",
      enabled: true,
      format: "gif",
      suffix: "_new",
      width: 500,
      height: null,
      fps: 24,
      crop: null,
      palette_colors: 128,
      dither: "bayer",
      bayer_scale: 3,
      crf: 23,
      preset_speed: "medium",
      video_bitrate: null,
      audio_bitrate: "128k",
      use_cuda: false,
      target_max_mb: null,
      icon: null,
      order: presets.length,
    };
    presets = [...presets, fresh];
    selectedId = fresh.id;
    dirty = true;
  }

  function duplicatePreset(p: Preset) {
    const copy: Preset = {
      ...p,
      id: genId(p.name + " copy"),
      name: p.name + " copy",
      order: presets.length,
    };
    presets = [...presets, copy];
    selectedId = copy.id;
    dirty = true;
  }

  function deletePreset(p: Preset) {
    if (!confirm(`Delete preset "${p.name}"? This also removes its SendTo shortcut.`)) return;
    presets = presets.filter((x) => x.id !== p.id);
    if (selectedId === p.id) selectedId = presets[0]?.id ?? null;
    dirty = true;
  }

  function move(p: Preset, delta: number) {
    const i = presets.findIndex((x) => x.id === p.id);
    const j = i + delta;
    if (j < 0 || j >= presets.length) return;
    const copy = [...presets];
    [copy[i], copy[j]] = [copy[j], copy[i]];
    copy.forEach((x, k) => (x.order = k));
    presets = copy;
    dirty = true;
  }

  async function save() {
    saving = true;
    try {
      await api.savePresets(presets);
      dirty = false;
      savedTick++;
    } finally {
      saving = false;
    }
  }

  async function saveSettings() {
    await api.saveSettings(settings);
    ffmpeg = await api.ffmpegStatus();
  }

  async function resetDefaults() {
    if (!confirm("Reset all presets to defaults? Your customizations will be lost.")) return;
    presets = await api.resetPresetsToDefaults();
    selectedId = presets[0]?.id ?? null;
    dirty = false;
  }

  // Track edits to mark dirty
  $effect(() => {
    if (selected) {
      // reading selected fields subscribes effect
      void selected.name;
      void selected.format;
      void selected.suffix;
      void selected.width;
      void selected.height;
      void selected.fps;
      void selected.crop;
      void selected.crf;
      void selected.palette_colors;
      void selected.dither;
      void selected.bayer_scale;
      void selected.preset_speed;
      void selected.video_bitrate;
      void selected.audio_bitrate;
      void selected.use_cuda;
      void selected.target_max_mb;
      void selected.enabled;
      dirty = true;
    }
  });
</script>

<main class="shell">
  <header class="topbar">
    <div class="brand">
      <h1>Offspring</h1>
      <span class="tiny">Right-click convert · powered by FFmpeg</span>
    </div>

    <nav class="tabs">
      <button class={tab === "presets" ? "tab active" : "tab"} onclick={() => (tab = "presets")}>Presets</button>
      <button class={tab === "settings" ? "tab active" : "tab"} onclick={() => (tab = "settings")}>Settings</button>
    </nav>

    <div class="tools">
      <span class="badge {ffmpeg.found ? 'ok' : 'warn'}" title={ffmpeg.path ?? ''}>
        <span class="dot {ffmpeg.found ? 'ok' : 'warn'}"></span>
        FFmpeg {ffmpeg.found ? "ready" : "missing"}
      </span>
      {#if dirty}
        <button class="primary" onclick={save} disabled={saving}>
          {saving ? "Saving…" : "Save & Sync SendTo"}
        </button>
      {:else if savedTick > 0}
        <span class="tiny saved">Saved</span>
      {/if}
    </div>
  </header>

  {#if tab === "presets"}
    <section class="panes">
      <aside class="sidebar">
        <div class="sidebar-head">
          <span class="tiny">PRESETS</span>
          <button class="ghost" onclick={addPreset} title="Add preset">+ Add</button>
        </div>
        <ul class="preset-list">
          {#each presets as p (p.id)}
            <li
              class={selectedId === p.id ? "row-item active" : "row-item"}
              onclick={() => (selectedId = p.id)}
              onkeydown={(e) => e.key === "Enter" && (selectedId = p.id)}
              role="button"
              tabindex="0"
            >
              <input
                type="checkbox"
                checked={p.enabled}
                onclick={(e) => e.stopPropagation()}
                onchange={(e) => {
                  p.enabled = (e.currentTarget as HTMLInputElement).checked;
                  dirty = true;
                }}
                title="Show in SendTo menu"
              />
              <span class="fmt-tag {p.format}">{p.format.toUpperCase()}</span>
              <span class="preset-name">{p.name}</span>
              <div class="actions">
                <button class="ghost tiny-btn" onclick={(e) => { e.stopPropagation(); move(p, -1); }} title="Move up">↑</button>
                <button class="ghost tiny-btn" onclick={(e) => { e.stopPropagation(); move(p, 1); }} title="Move down">↓</button>
              </div>
            </li>
          {/each}
        </ul>
        <div class="sidebar-foot">
          <button class="ghost" onclick={resetDefaults}>Reset to defaults</button>
        </div>
      </aside>

      <section class="editor">
        {#if selected}
          <div class="editor-head">
            <input
              class="title-input"
              type="text"
              bind:value={selected.name}
              placeholder="Preset name"
            />
            <div class="row">
              <button class="ghost" onclick={() => duplicatePreset(selected!)}>Duplicate</button>
              <button class="danger" onclick={() => deletePreset(selected!)}>Delete</button>
            </div>
          </div>
          <p class="muted tiny">Shortcut appears in right-click → Send To as <code>Offspring - {selected.name}.lnk</code></p>

          <div class="fields">
            <FormatFields preset={selected} />
          </div>
        {:else}
          <div class="empty">
            <h2>No preset selected</h2>
            <p class="muted">Pick one from the sidebar or add a new one.</p>
          </div>
        {/if}
      </section>
    </section>
  {:else}
    <section class="settings-pane">
      <div class="card">
        <h3>FFmpeg</h3>
        <p class="muted tiny">Leave path blank to use the bundled/managed FFmpeg, or point to your own install.</p>
        <div class="row" style="margin-top: 12px;">
          <input
            type="text"
            value={settings.ffmpeg_path ?? ""}
            oninput={(e) => {
              const v = (e.currentTarget as HTMLInputElement).value;
              settings.ffmpeg_path = v === "" ? null : v;
            }}
            placeholder="(default location)"
          />
          <button onclick={saveSettings}>Save</button>
        </div>
        <p class="tiny" style="margin-top: 8px;">
          Status: <span class="badge {ffmpeg.found ? 'ok' : 'warn'}">
            {ffmpeg.found ? ffmpeg.path : "not found"}
          </span>
        </p>

        {#if !ffmpeg.found && !dl.active && dl.phase !== "done"}
          <div class="dl-box">
            <p class="tiny muted">
              No FFmpeg found. Download the LGPL essentials build (~80 MB) from
              <a href="https://www.gyan.dev/ffmpeg/builds/" target="_blank" rel="noreferrer">gyan.dev</a>
              into <code>%LOCALAPPDATA%\Offspring\ffmpeg\</code>.
            </p>
            <button class="primary" onclick={startDownloadFfmpeg}>Download FFmpeg</button>
            {#if dl.error}
              <p class="tiny err">✕ {dl.error}</p>
            {/if}
          </div>
        {:else if dl.active}
          <div class="dl-box">
            <div class="row between">
              <span class="tiny muted">
                {dl.phase === "downloading" ? "Downloading FFmpeg…" :
                 dl.phase === "extracting" ? "Extracting archive…" :
                 dl.phase === "starting" ? "Starting…" : dl.phase}
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
            {#if dl.message}
              <p class="tiny muted">{dl.message}</p>
            {/if}
          </div>
        {/if}
      </div>

      <div class="card">
        <h3>FFmpeg verbosity</h3>
        <select
          value={settings.verbosity ?? "warning"}
          onchange={(e) => {
            settings.verbosity = (e.currentTarget as HTMLSelectElement).value;
          }}
        >
          <option value="warning">warning</option>
          <option value="info">info</option>
          <option value="error">error</option>
        </select>
        <div class="row" style="margin-top: 12px;">
          <button onclick={saveSettings}>Save</button>
        </div>
      </div>

      <div class="card">
        <h3>Data folder</h3>
        <p class="muted tiny">Presets and settings live under <code>%APPDATA%\Offspring</code>.</p>
        <div class="row" style="margin-top: 12px;">
          <button onclick={api.openDataFolder}>Open folder</button>
          <button onclick={api.syncSendto}>Re-sync SendTo menu</button>
        </div>
      </div>
    </section>
  {/if}
</main>

<style>
  .shell { display: flex; flex-direction: column; height: 100vh; }

  .topbar {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    padding: 8px 16px;
    border-bottom: 1px solid var(--c-border);
    background: var(--c-surface);
  }
  .brand { display: flex; flex-direction: column; gap: 0; }
  .brand h1 { font-size: var(--fs-20); line-height: 1.1; }
  .tabs {
    display: flex;
    gap: 2px;
    background: var(--c-surface-3);
    padding: 2px;
    border-radius: var(--r-md);
  }
  .tab {
    background: transparent;
    border: none;
    color: var(--c-text-3);
    padding: 4px 12px;
    min-height: 0;
    border-radius: var(--r-sm);
    font-size: var(--fs-14);
  }
  .tab:hover { background: transparent; color: var(--c-text); }
  .tab.active {
    background: var(--c-surface);
    color: var(--c-text);
    box-shadow: var(--shadow-whisper);
  }
  .tools { justify-self: end; display: flex; align-items: center; gap: 8px; }
  .saved { color: var(--c-text-3); }

  .panes { display: grid; grid-template-columns: 260px 1fr; flex: 1; min-height: 0; }

  .sidebar {
    border-right: 1px solid var(--c-border);
    display: flex;
    flex-direction: column;
    background: var(--c-surface-2);
    min-height: 0;
  }
  .sidebar-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 10px 4px;
    letter-spacing: 0.08em;
    color: var(--c-text-3);
  }
  .sidebar-head button {
    min-height: 0; padding: 2px 8px; font-size: var(--fs-12);
  }
  .preset-list {
    list-style: none;
    padding: 2px 6px;
    margin: 0;
    overflow-y: auto;
    flex: 1;
    min-height: 0;
  }
  .row-item {
    display: flex; align-items: center; gap: 6px;
    padding: 4px 6px;
    border-radius: var(--r-sm);
    cursor: pointer;
    transition: background 120ms ease;
    font-size: var(--fs-13, 13px);
  }
  .row-item:hover { background: var(--c-surface); }
  .row-item.active {
    background: var(--c-surface);
    box-shadow: var(--shadow-ring);
  }
  .fmt-tag {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.06em;
    padding: 1px 5px;
    border-radius: 3px;
    background: var(--c-surface-3);
    color: var(--c-text-2);
    flex: 0 0 auto;
  }
  .fmt-tag.gif { background: #FEF3C7; color: #92400E; }
  .fmt-tag.mp4 { background: var(--c-primary-tint); color: #0D47A1; }
  .preset-name {
    flex: 1;
    font-size: var(--fs-14);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .actions { display: flex; gap: 0; opacity: 0.35; }
  .row-item:hover .actions,
  .row-item.active .actions { opacity: 1; }
  .tiny-btn {
    padding: 0 4px;
    min-height: 0;
    font-size: 11px;
    border: none;
    background: transparent;
    color: var(--c-text-3);
  }
  .tiny-btn:hover { color: var(--c-text); background: var(--c-surface-2); }
  .sidebar-foot {
    padding: 6px 10px;
    border-top: 1px solid var(--c-border);
  }
  .sidebar-foot button { font-size: var(--fs-12); padding: 4px 8px; min-height: 0; }

  .editor {
    padding: 12px 18px 16px;
    overflow-y: auto;
    background: var(--c-surface);
    min-height: 0;
  }
  .editor-head {
    display: flex; justify-content: space-between; align-items: center;
    gap: 12px; margin-bottom: 2px;
  }
  .title-input {
    font-family: var(--font-display);
    font-size: var(--fs-20);
    font-weight: 600;
    border: 1px solid transparent;
    background: transparent;
    padding: 2px 6px;
    min-height: 0;
    border-radius: var(--r-md);
    color: var(--c-text);
  }
  .title-input:hover { background: var(--c-surface-2); border-color: var(--c-border); }
  .title-input:focus {
    background: var(--c-surface);
    border-color: var(--c-primary);
    box-shadow: 0 0 0 3px var(--c-primary-ring);
  }
  .editor-head button { font-size: var(--fs-12); padding: 4px 10px; min-height: 0; }
  .fields { margin-top: 10px; }
  .empty { text-align: center; padding: 40px 20px; color: var(--c-text-3); }

  .settings-pane {
    padding: 14px 18px;
    display: grid;
    gap: 10px;
    max-width: 640px;
    width: 100%;
    margin: 0 auto;
    overflow-y: auto;
  }
  .settings-pane .card { padding: var(--sp-3); }
  .settings-pane h3 { margin-bottom: 2px; }

  code {
    font-family: var(--font-mono);
    font-size: 0.88em;
    background: var(--c-surface-3);
    padding: 1px 5px;
    border-radius: 3px;
    color: var(--c-text-2);
  }

  /* FFmpeg download block — fits inside the FFmpeg settings card. */
  .dl-box {
    margin-top: 10px;
    padding: 10px 12px;
    background: var(--c-canvas-muted);
    border: 1px solid var(--c-border);
    border-radius: var(--r-lg);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .dl-box a {
    color: var(--c-primary);
    text-decoration: underline;
    text-decoration-color: var(--c-primary-ring);
    text-underline-offset: 2px;
  }
  .dl-box .row.between { display: flex; justify-content: space-between; gap: 8px; }
  .dl-box .bar {
    height: 6px;
    background: var(--c-surface-3);
    border-radius: var(--r-pill);
    overflow: hidden;
  }
  .dl-box .fill {
    height: 100%;
    background: var(--c-primary);
    transition: width 200ms ease;
  }
  .dl-box .fill.indet {
    width: 40%;
    animation: slide 1.2s ease-in-out infinite;
  }
  @keyframes slide {
    0%   { transform: translateX(-100%); }
    100% { transform: translateX(250%); }
  }
  .dl-box .err { color: var(--c-danger); }
</style>
