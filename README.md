<div align="center">

<img src="src-tauri/icons/128x128@2x.png" alt="Offspring" width="128" height="128" />

# Offspring

**Right-click convert videos & images with FFmpeg — from anywhere in Windows or macOS.**

</div>

---

Offspring is a tiny cross-platform app (Windows 11 + macOS) that adds
video/image conversion shortcuts to your right-click menu. Drop a `.mov`
on a shortcut, get a trimmed-down `.mp4` or a Discord-ready `.gif` in
the same folder. No windows to open, no command lines, no upload to a
web service.

<p align="center">
  <img src="docs/screenshots/contextmenu.png" alt="Right-click menu with Offspring preset entries" width="420" />
</p>

## What it does

- Ships a **classic right-click submenu** on every video and image
  (always on) — one click per preset.
- Optionally populates your **Send to** menu.
- Optionally integrates with the **Windows 11 modern right-click menu**
  (top-level, no "Show more options") via a signed sparse MSIX package.
- Manages a curated list of presets — GIFs for Discord, compressed MP4s,
  cropped 9:16 verticals, 1080p downscales — that you can freely edit,
  reorder, and add to from the Presets tab. Every preset can set its
  own crop, max file size, palette, and dither for GIFs.
- Streams per-file progress in a small always-on-top window while
  FFmpeg does the work.

<p align="center">
  <img src="docs/screenshots/main.png" alt="Offspring main window — preset list and Discord GIF settings" width="760" />
</p>

<p align="center">
  <img src="docs/screenshots/encoding.png" alt="Encoding progress window" width="420" />
</p>

## Install

Grab the latest installer from the
[Releases page](https://github.com/second-march/offspring/releases/latest):

- **Windows**: `Offspring-Setup-<version>.exe` (Standard) or
  `Offspring-Studio-Setup-<version>.exe` (no-network variant — see
  [SECURITY.md](./SECURITY.md)). Standard is a per-user install; no
  admin prompt unless you're upgrading from a pre-0.4.4 per-machine
  build.
- **macOS**: `Offspring_<version>_universal.dmg`. Drag to Applications.
  Signed + notarized; Gatekeeper allows it on first launch. To enable
  the Finder right-click integration, after first launch open
  **System Settings → Keyboard → Keyboard Shortcuts → Services →
  Files and Folders** and tick **Offspring…**.

On first launch, Offspring downloads a small FFmpeg build into the
per-user data folder if one isn't already on `PATH` — gyan.dev's LGPL
essentials build on Windows (~80 MB to
`%LOCALAPPDATA%\Offspring\ffmpeg\`), evermeet.cx's universal static
build on macOS (to `~/Library/Application Support/Offspring/ffmpeg/`).
You can also point the app at a pre-existing FFmpeg install from the
Settings tab (e.g. `/opt/homebrew/bin/ffmpeg` for a Homebrew install).

Updates: Offspring **never pings GitHub on its own**. It only checks
for updates when you click **Settings → Check for updates**, and even
then it just shows a banner — the new installer doesn't download until
you click "Download" in that banner. Every installer is signed offline
with an Ed25519 [minisign](https://jedisct1.github.io/minisign/) key
whose public counterpart is pinned in the binary; the in-app updater
verifies the signature against that key and refuses to launch any
installer whose signature is missing or doesn't match. See
[SECURITY.md](./SECURITY.md) and
[THREAT_MODEL.md](./THREAT_MODEL.md) for the full picture.

## FFmpeg licensing

Offspring does **not** bundle FFmpeg. It invokes whatever FFmpeg lives in
its per-user data folder (downloaded on demand —
`%LOCALAPPDATA%\Offspring\ffmpeg\` on Windows,
`~/Library/Application Support/Offspring/ffmpeg/` on macOS) or the path
you configured in Settings. FFmpeg is © the FFmpeg developers, licensed
under the [LGPL v2.1+](https://www.ffmpeg.org/legal.html), and its source
is available at <https://ffmpeg.org/download.html>.

Because Offspring calls FFmpeg as a separate executable (no linking), the
LGPL does not propagate to Offspring's own code. Offspring itself is MIT.

Full third-party attributions live in [NOTICE.md](./NOTICE.md).


## Privacy

Offspring makes no analytics, telemetry, or phone-home calls. The
app makes **zero outbound requests on its own** — every network call
is explicitly user-initiated:

- **GitHub Releases update check** when you click *Settings → Check
  for updates*.
- **Installer download** when you click *Download* in the update
  banner that follows.
- **gyan.dev FFmpeg download** when you click *Download FFmpeg* on
  first run (or any time later if you uninstall FFmpeg).

Full inventory in
[SECURITY.md → Privacy / network connections](./SECURITY.md#privacy--network-connections).

## License

[MIT](./LICENSE) © 2026 Second March.
