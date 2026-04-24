<script lang="ts">
  import { onMount } from "svelte";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import FormatFields from "$lib/components/FormatFields.svelte";
  import * as api from "$lib/api";
  import type { Preset, Settings, FfmpegStatus, UpdateInfo } from "$lib/types";

  let presets = $state<Preset[]>([]);
  let selectedId = $state<string | null>(null);
  let settings = $state<Settings>({});
  let ffmpeg = $state<FfmpegStatus>({ found: false, path: null });
  let tab = $state<"presets" | "settings">("presets");
  let dirty = $state(false);
  let saving = $state(false);
  let savedTick = $state(0);
  // Right-click menu for preset rows. Non-null when visible.
  let ctxMenu = $state<{ x: number; y: number; preset: Preset } | null>(null);

  // Drag-and-drop reorder state. `dragId` is the preset being dragged;
  // `dragOver` is the row the cursor is currently over with a position
  // indicator telling us whether to drop above or below it. The drop-line
  // is rendered between rows based on this.
  let dragId = $state<string | null>(null);
  let dragOver = $state<{ id: string; pos: "above" | "below" } | null>(null);

  // FFmpeg download state (fed by the `ffmpeg-download` event from Rust)
  let dl = $state<{
    active: boolean;
    phase: string;
    percent: number | null;
    message: string | null;
    error: string | null;
  }>({ active: false, phase: "", percent: null, message: null, error: null });

  // Update-check state. We cache the most recent result in sessionStorage so
  // switching tabs in the webview doesn't re-hit GitHub on every mount.
  let update = $state<UpdateInfo | null>(null);
  const UPDATE_CACHE_KEY = "offspring.updateInfo";
  const UPDATE_DISMISS_KEY = "offspring.updateDismissedFor";

  // In-app update download state. `phase` drives the banner button:
  //   idle        — update detected, download not started
  //   downloading — streaming the installer in the background
  //   ready       — installer on disk, ready to run
  //   error       — download failed; fall back to browser download
  let upd = $state<{
    phase: "idle" | "downloading" | "ready" | "error";
    percent: number | null;
    message: string | null;
  }>({ phase: "idle", percent: null, message: null });

  // Manual "Check for updates" button state. `checking` drives the
  // spinner, `lastChecked` is the wall-clock time of the most recent
  // successful check, and `manualResult` is a one-shot status line
  // ("You're on the latest version.") shown after a manual check even
  // when no update is available. `currentVersion` is filled by the
  // first `check_for_updates` call — even a network-failed check
  // populates it from `CARGO_PKG_VERSION`, so we always have something
  // to display.
  let updateCheck = $state<{
    checking: boolean;
    lastChecked: number | null;
    manualResult: string | null;
  }>({ checking: false, lastChecked: null, manualResult: null });
  let currentVersion = $state<string>("");

  const selected = $derived(presets.find((p) => p.id === selectedId) ?? null);

  onMount(async () => {
    await reload();

    // Intercept the window close so the user can't quit on a dirty state
    // without being warned. We have to register this via Tauri's
    // onCloseRequested API rather than `beforeunload` — WebView2 on
    // Windows doesn't fire beforeunload for native window-close actions.
    await getCurrentWindow().onCloseRequested(async (event) => {
      if (!dirty) return;
      const ok = confirm(
        "You have unsaved changes.\n\n" +
          "Click OK to close without saving, or Cancel to go back and click 'Save and Sync'.",
      );
      if (!ok) event.preventDefault();
    });

    // Fire-and-forget update check. We don't block reload on this, and any
    // failure (no network / private repo / no releases yet) collapses to
    // "no banner" rather than a visible error.
    void checkUpdate();

    // Subscribe to update download events before kicking off the check so
    // we never miss a "done" emitted from an auto-started download.
    await api.onUpdateDownload((e) => {
      if (e.phase === "downloading") {
        upd.phase = "downloading";
        upd.percent = e.percent;
        upd.message = e.message;
      } else if (e.phase === "done") {
        upd.phase = "ready";
        upd.percent = 100;
        upd.message = null;
      } else if (e.phase === "error") {
        upd.phase = "error";
        upd.percent = null;
        upd.message = e.message;
      }
    });

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

  async function checkUpdate(opts: { manual?: boolean } = {}) {
    // Respect an in-session dismiss for this specific version so closing
    // the banner stays closed until the app is relaunched (or a newer
    // version lands). A manual re-check from Settings bypasses this —
    // if the user explicitly asks, honour it.
    const dismissedFor = opts.manual
      ? null
      : sessionStorage.getItem(UPDATE_DISMISS_KEY);

    // Warm-start from cache so the banner renders without a round-trip
    // when navigating between routes (progress → main, etc). Skipped on
    // manual checks — the user wants a fresh answer.
    if (!opts.manual) {
      const cached = sessionStorage.getItem(UPDATE_CACHE_KEY);
      if (cached) {
        try {
          const parsed = JSON.parse(cached) as UpdateInfo;
          if (parsed.current) currentVersion = parsed.current;
          if (parsed.update_available && dismissedFor !== parsed.latest) {
            update = parsed;
            maybeStartDownload(parsed);
          }
        } catch {}
      }
    }

    if (opts.manual) {
      updateCheck.checking = true;
      updateCheck.manualResult = null;
    }
    try {
      const info = await api.checkForUpdates();
      sessionStorage.setItem(UPDATE_CACHE_KEY, JSON.stringify(info));
      if (info.current) currentVersion = info.current;
      updateCheck.lastChecked = Date.now();
      if (info.update_available && dismissedFor !== info.latest) {
        update = info;
        maybeStartDownload(info);
        if (opts.manual) {
          updateCheck.manualResult = `Version ${info.latest} is available.`;
        }
      } else {
        update = null;
        if (opts.manual) {
          updateCheck.manualResult = info.latest
            ? `You're on the latest version (${info.current}).`
            : `Couldn't reach the update server. Try again later.`;
        }
      }
    } catch {
      // Network fail = stay quiet on the automatic path. On a manual
      // check the user asked, so surface it.
      if (opts.manual) {
        updateCheck.manualResult = "Couldn't reach the update server. Try again later.";
      }
    } finally {
      if (opts.manual) updateCheck.checking = false;
    }
  }

  // Level 2 behaviour: as soon as we know a newer version is out there,
  // eagerly stream the installer in the background. No user interaction
  // needed until they're ready to restart. Skipped if there's no direct
  // installer URL on the release (we fall back to opening the release page).
  function maybeStartDownload(info: UpdateInfo) {
    if (!info.installer_url) return;
    if (upd.phase === "downloading" || upd.phase === "ready") return;
    upd = { phase: "downloading", percent: 0, message: "Starting…" };
    api.downloadUpdate(info.latest, info.installer_url).catch((err) => {
      upd.phase = "error";
      upd.percent = null;
      upd.message = String(err);
    });
  }

  async function onUpdateClick() {
    if (!update) return;
    if (upd.phase === "ready") {
      // Installer is on disk — run it silently and exit. Inno Setup's
      // /RESTARTAPPLICATIONS will re-launch Offspring after the swap.
      try {
        await api.installUpdate(update.latest);
      } catch (err) {
        upd.phase = "error";
        upd.message = String(err);
      }
      return;
    }
    if (upd.phase === "error" || !update.installer_url) {
      // Download failed or there's no .exe asset — open the release page
      // so the user can grab it manually.
      try {
        await openUrl(update.installer_url || update.html_url);
      } catch {}
      return;
    }
    // "idle" or still "downloading" — if we haven't kicked off yet, do so
    // now; otherwise the click is a no-op while progress ticks.
    if (upd.phase === "idle") maybeStartDownload(update);
  }

  function dismissUpdate() {
    if (!update) return;
    sessionStorage.setItem(UPDATE_DISMISS_KEY, update.latest);
    update = null;
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

  function onDragStart(e: DragEvent, p: Preset) {
    dragId = p.id;
    // Firefox won't start a drag without data on the transfer. The payload
    // itself is unused — we key off `dragId` in component state, which
    // survives the serialization restrictions dataTransfer imposes during
    // the drag (only type strings are readable until drop).
    e.dataTransfer?.setData("text/plain", p.id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
  }

  // WebView2 requires preventDefault on BOTH dragenter AND dragover for the
  // element to register as a valid drop target. Skipping dragenter leaves
  // the cursor stuck in the "forbidden" state even while over a child row.
  function onDragEnter(e: DragEvent) {
    if (!dragId) return;
    e.preventDefault();
  }

  function onDragOver(e: DragEvent, p: Preset) {
    if (!dragId) return;
    // Always preventDefault while a drag is active — including over the
    // source row — so the browser shows the "move" cursor instead of
    // "forbidden". Dropping on the source is a no-op (handled in onDrop)
    // but the user shouldn't be punished with a scary cursor for moving
    // over their own row on the way somewhere else.
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    if (dragId === p.id) {
      // Clear any prior indicator so we don't draw a drop line on the
      // source row itself.
      if (dragOver) dragOver = null;
      return;
    }
    // Above / below split at the row's vertical midpoint so the insertion
    // point feels natural as the cursor moves past an item.
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const pos: "above" | "below" = e.clientY < rect.top + rect.height / 2 ? "above" : "below";
    if (!dragOver || dragOver.id !== p.id || dragOver.pos !== pos) {
      dragOver = { id: p.id, pos };
    }
  }

  function onDrop(e: DragEvent, target: Preset) {
    e.preventDefault();
    const src = dragId;
    const over = dragOver;
    dragId = null;
    dragOver = null;
    if (!src || !over || src === target.id) return;
    const from = presets.findIndex((x) => x.id === src);
    if (from < 0) return;
    const copy = [...presets];
    const [moved] = copy.splice(from, 1);
    // Re-derive the insertion index against the spliced array, since
    // removing an earlier element shifts everything after it.
    let insertBefore = copy.findIndex((x) => x.id === target.id);
    if (over.pos === "below") insertBefore += 1;
    copy.splice(insertBefore, 0, moved);
    copy.forEach((x, k) => (x.order = k));
    presets = copy;
    dirty = true;
  }

  function onDragEnd() {
    dragId = null;
    dragOver = null;
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

<svelte:window onclick={() => (ctxMenu = null)} />

{#if ctxMenu}
  <div
    class="ctx-menu"
    style="left: {ctxMenu.x}px; top: {ctxMenu.y}px;"
    role="menu"
    onclick={(e) => e.stopPropagation()}
  >
    <button
      type="button"
      role="menuitem"
      onclick={() => { duplicatePreset(ctxMenu!.preset); ctxMenu = null; }}
    >Duplicate</button>
    <button
      type="button"
      role="menuitem"
      class="danger"
      onclick={() => { deletePreset(ctxMenu!.preset); ctxMenu = null; }}
    >Delete</button>
  </div>
{/if}

{#if update && update.update_available}
  <aside class="update-banner" role="status">
    <span class="update-icon" aria-hidden="true">⬆</span>
    <span class="update-text">
      {#if upd.phase === "downloading"}
        Downloading <strong>{update.latest}</strong>{upd.percent != null ? ` — ${Math.round(upd.percent)}%` : "…"}
      {:else if upd.phase === "ready"}
        Version <strong>{update.latest}</strong> is ready to install.
      {:else if upd.phase === "error"}
        Update <strong>{update.latest}</strong> couldn't download automatically.
      {:else}
        Version <strong>{update.latest}</strong> is available (you have {update.current}).
      {/if}
    </span>
    {#if upd.phase === "downloading"}
      <div class="update-bar" aria-hidden="true">
        <div
          class="update-bar-fill"
          class:indet={upd.percent == null}
          style={upd.percent != null ? `width: ${Math.round(upd.percent)}%;` : ""}
        ></div>
      </div>
    {:else}
      <button
        type="button"
        class="update-btn"
        onclick={onUpdateClick}
        disabled={upd.phase === "downloading"}
      >
        {#if upd.phase === "ready"}
          Restart and install
        {:else if upd.phase === "error"}
          Open download page
        {:else}
          Download
        {/if}
      </button>
    {/if}
    <button
      type="button"
      class="update-close"
      aria-label="Dismiss update notice"
      onclick={dismissUpdate}
    >×</button>
  </aside>
{/if}

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
        <button class="primary save-pulse" onclick={save} disabled={saving}>
          {saving ? "Saving…" : "Save and Sync"}
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
              class="row-item"
              class:active={selectedId === p.id}
              class:dragging={dragId === p.id}
              class:drop-above={dragOver?.id === p.id && dragOver?.pos === "above"}
              class:drop-below={dragOver?.id === p.id && dragOver?.pos === "below"}
              draggable="true"
              ondragstart={(e) => onDragStart(e, p)}
              ondragenter={onDragEnter}
              ondragover={(e) => onDragOver(e, p)}
              ondrop={(e) => onDrop(e, p)}
              ondragend={onDragEnd}
              onclick={() => (selectedId = p.id)}
              oncontextmenu={(e) => {
                e.preventDefault();
                selectedId = p.id;
                ctxMenu = { x: e.clientX, y: e.clientY, preset: p };
              }}
              onkeydown={(e) => e.key === "Enter" && (selectedId = p.id)}
              role="button"
              tabindex="0"
            >
              <span class="grip" aria-hidden="true" title="Drag to reorder">≡</span>
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
        <h3>Right-click menu</h3>
        <p class="muted tiny">
          By default, Offspring lives under Windows 11's "Show more options" (the classic right-click menu).
          Enabling the modern menu below moves it to the top-level right-click menu — it won't also appear
          under "Show more options", so you don't end up with two entries.
        </p>
        <div style="margin-top: 12px; display: flex; flex-direction: column; gap: 10px;">
          <label class="inline">
            <input
              type="checkbox"
              checked={settings.sendto_enabled ?? false}
              onchange={(e) => {
                settings.sendto_enabled = (e.currentTarget as HTMLInputElement).checked;
                saveSettings();
              }}
            />
            <span>Also add entries to the <strong>Send to</strong> menu</span>
          </label>
          <label class="inline">
            <input
              type="checkbox"
              checked={settings.modern_menu_enabled ?? false}
              onchange={async (e) => {
                const checked = (e.currentTarget as HTMLInputElement).checked;
                const wasOff = !(settings.modern_menu_enabled ?? false);
                settings.modern_menu_enabled = checked;
                await saveSettings();
                // Explorer caches the modern-menu handler list — the
                // new entry only appears once it re-launches. Offer to
                // do it for the user (loses open Explorer windows) but
                // never force it.
                if (checked && wasOff) {
                  const ok = confirm(
                    "Modern right-click menu enabled.\n\n" +
                      "Restart Windows Explorer now so it picks up the new menu? " +
                      "Any open File Explorer windows will close.\n\n" +
                      "Cancel to restart later — it will also take effect after sign-out / reboot.",
                  );
                  if (ok) {
                    try { await api.restartExplorer(); } catch (err) { alert(String(err)); }
                  }
                }
              }}
            />
            <span>Integrate with the <strong>Windows 11 modern right-click menu</strong> (top-level, no extra click)</span>
          </label>
          <p class="tiny muted" style="margin: 0; padding-left: 22px;">
            Enabling the modern menu registers a sparse MSIX package. The installer already trusted the
            signing cert, so no prompts appear here. Offspring will offer to restart Windows Explorer so
            the menu shows up right away — skip it if you have File Explorer windows open and it will
            take effect after the next sign-out.
          </p>
        </div>
      </div>

      <div class="card">
        <h3>Updates</h3>
        <p class="muted tiny">
          Current version: <strong>{currentVersion || "…"}</strong>
        </p>
        <div class="row" style="margin-top: 12px;">
          <button onclick={() => checkUpdate({ manual: true })} disabled={updateCheck.checking}>
            {updateCheck.checking ? "Checking…" : "Check for updates"}
          </button>
        </div>
        {#if updateCheck.manualResult}
          <p class="tiny muted" style="margin-top: 8px;">{updateCheck.manualResult}</p>
        {/if}
      </div>

      <div class="card">
        <h3>Data folder</h3>
        <p class="muted tiny">Presets and settings live under <code>%APPDATA%\Offspring</code>.</p>
        <div class="row" style="margin-top: 12px;">
          <button onclick={api.openDataFolder}>Open folder</button>
          <button onclick={api.syncIntegrations}>Re-sync right-click menus</button>
        </div>
      </div>
    </section>
  {/if}
</main>

<style>
  .shell { display: flex; flex-direction: column; height: 100vh; }

  label.inline {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    font-size: var(--fs-13, 13px);
    color: var(--c-text);
    margin: 0;
    cursor: pointer;
  }
  label.inline input[type="checkbox"] {
    margin-top: 2px;
    flex-shrink: 0;
  }

  .update-banner {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 16px;
    background: var(--c-accent, #3b82f6);
    color: #fff;
    font-size: var(--fs-14, 14px);
    border-bottom: 1px solid rgba(0, 0, 0, 0.15);
  }
  .update-icon {
    font-weight: bold;
    opacity: 0.9;
  }
  .update-text {
    flex: 1;
  }
  .update-text strong {
    font-weight: 600;
  }
  .update-btn {
    background: rgba(255, 255, 255, 0.22);
    color: #fff;
    border: 1px solid rgba(255, 255, 255, 0.35);
    padding: 4px 12px;
    border-radius: var(--r-sm, 6px);
    font-weight: 500;
    cursor: pointer;
  }
  .update-btn:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.32);
  }
  .update-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .update-bar {
    width: 140px;
    height: 6px;
    background: rgba(255, 255, 255, 0.25);
    border-radius: var(--r-pill, 999px);
    overflow: hidden;
  }
  .update-bar-fill {
    height: 100%;
    background: #fff;
    transition: width 200ms ease;
  }
  .update-bar-fill.indet {
    width: 40%;
    animation: update-slide 1.2s ease-in-out infinite;
  }
  @keyframes update-slide {
    0%   { transform: translateX(-120%); }
    100% { transform: translateX(260%); }
  }
  .update-close {
    background: transparent;
    color: #fff;
    border: none;
    font-size: 18px;
    line-height: 1;
    padding: 2px 6px;
    cursor: pointer;
    opacity: 0.8;
  }
  .update-close:hover {
    opacity: 1;
  }

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
  .ctx-menu {
    position: fixed;
    z-index: 1000;
    min-width: 140px;
    padding: 4px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    background: var(--c-surface);
    border: 1px solid var(--c-border);
    border-radius: var(--r-sm);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.2);
  }
  .ctx-menu button {
    all: unset;
    padding: 6px 10px;
    border-radius: var(--r-sm);
    font-size: var(--fs-13, 13px);
    cursor: pointer;
    color: var(--c-text);
  }
  .ctx-menu button:hover { background: var(--c-surface-2); }
  .ctx-menu button.danger { color: var(--c-danger, #b91c1c); }
  .ctx-menu button.danger:hover { background: var(--c-danger-tint, rgba(185, 28, 28, 0.12)); }
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
  .grip {
    flex: 0 0 auto;
    color: var(--c-text-3);
    font-size: 14px;
    line-height: 1;
    padding: 0 2px;
    cursor: grab;
    user-select: none;
    opacity: 0.4;
    transition: opacity 120ms ease;
  }
  .row-item:hover .grip,
  .row-item.active .grip { opacity: 1; }
  .row-item.dragging {
    opacity: 0.4;
  }
  .row-item.drop-above {
    box-shadow: inset 0 2px 0 0 var(--c-primary);
  }
  .row-item.drop-below {
    box-shadow: inset 0 -2px 0 0 var(--c-primary);
  }
  .row-item[draggable="true"] { cursor: pointer; }
  .row-item[draggable="true"]:active .grip { cursor: grabbing; }

  @keyframes save-pulse-ring {
    0%   { box-shadow: 0 0 0 0 var(--c-primary-ring); }
    70%  { box-shadow: 0 0 0 8px rgba(0, 0, 0, 0); }
    100% { box-shadow: 0 0 0 0 rgba(0, 0, 0, 0); }
  }
  .save-pulse {
    animation: save-pulse-ring 1.6s ease-out infinite;
  }
  .save-pulse:hover,
  .save-pulse:focus-visible {
    animation: none;
  }
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
