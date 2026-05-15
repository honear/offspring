# Security Policy

## Reporting a vulnerability

Please report security issues privately to **dev@secondmarch.xyz** rather
than opening a public issue. Encrypted email is welcome but not required.

A useful report includes:

- A description of the issue and its impact
- Steps to reproduce (or a proof-of-concept)
- The affected version (`Settings → About`, or run `Offspring-Setup.exe`
  and check the file properties)
- Whether you've shared the finding with anyone else

I'll acknowledge receipt within 7 days. If the issue is reproducible and
in scope (see below), I aim to ship a fix in the next release. Please
allow 90 days from acknowledgement before public disclosure unless we
agree on a different timeline.

## Supported versions

Only the latest published release on
[github.com/second-march/offspring/releases](https://github.com/second-march/offspring/releases)
receives security fixes. Local-iteration builds (versions ending in
`-bNNNN`) are pre-release artifacts and are not supported.

## Scope

The full threat model lives in [THREAT_MODEL.md](./THREAT_MODEL.md), but
in short:

**In scope** — issues I'll prioritise fixing:

- Network-attacker scenarios against the FFmpeg download or in-app
  updater (DNS hijack, MITM, hostile mirror, signature bypass)
- Tampered or malicious GitHub release assets passing the in-app
  updater's verification
- Drive-by attacks via right-clicking a malicious file (path
  traversal, argument injection, ffmpeg command injection)
- Supply-chain compromise of `cargo` / `npm` dependencies that ends up
  in a published release
- Privilege escalation or sandbox escape from the shell-extension DLL
- Content Security Policy bypasses or XSS through the webview

**Out of scope** — known limits I won't be patching:

- Malware already running as the same Windows user. Such code can
  modify presets, swap the offspring.exe binary, write to its
  registry keys, or read its data folder regardless of any control
  this app could plausibly add.
- Local privilege escalation by an admin or by a user who has already
  written to `HKLM`. Offspring is a per-user tool; we don't defend
  against the machine's administrator.
- Physical access to an unlocked machine.
- Issues that depend on the user installing a tampered installer
  obtained outside the official GitHub Releases page.
- DoS by spamming malformed CLI arguments to the named-pipe IPC.
  The pipe is local-and-current-user-only by Windows' default DACL,
  and the only effect is to make the primary instance briefly busy.

## Privacy / network connections

Offspring makes **no analytics, telemetry, or "phone-home" calls**.
The Standard variant makes a small number of outbound requests for
its core function (FFmpeg fetch, update check); the Studio variant
makes **zero** outbound requests of any kind. The complete list for
Standard:

| When | Where | Why |
|---|---|---|
| When the user clicks **Settings → Check for updates** | `https://api.github.com/repos/second-march/offspring/releases/latest` | Update check. Failures surface as "couldn't reach the update server" only on the manual path. The request carries the running version in the `User-Agent` header for release-page traffic stats; no other identifying data. |
| When the user clicks **Download** in the update banner that follows a successful check | GitHub-owned download host (one of `github.com`, `objects.githubusercontent.com`, `release-assets.githubusercontent.com`) | Downloads the installer .exe and its `.minisig` sidecar. Refuses to fetch from any other host. |
| At **first launch** on Standard, if no FFmpeg is resolvable (no path set in Settings, no managed copy installed, no `ffmpeg` on `PATH`) | Windows: `https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip` and the matching `.sha256` sidecar. macOS: `https://evermeet.cx/ffmpeg/info/{ffmpeg,ffprobe}/release` followed by the versioned `.zip` URL each info document points to. | One-time FFmpeg + FFprobe fetch. Runs once when nothing else is available. After the binaries are installed under the per-user data folder, no further evermeet.cx / gyan.dev traffic on subsequent launches. Users who already have FFmpeg on `PATH` (e.g. Homebrew on macOS, Chocolatey on Windows) never hit this path — Offspring uses the existing binary and does not auto-download. Users who want to bypass the auto-fetch entirely can install Studio instead, or set a custom FFmpeg path in Settings before first launch. |

That's the complete list. **No traffic at launch beyond the
conditional first-run FFmpeg fetch, no background pings, no scheduled
checks**, no crash reports, no usage stats, no third-party SDKs, no
remote config, no A/B tests. The in-app debug log lives only on the
user's machine (Windows: `%LOCALAPPDATA%\Offspring\debug.log`; macOS:
`~/Library/Application Support/Offspring/debug.log`) and is never
uploaded.

## Build variants: Offspring and Offspring Studio

Each release ships two installers built from the same source tree:

| | **Offspring** | **Offspring Studio** |
|---|---|---|
| Filename | `Offspring-Setup-X.Y.Z.exe` | `Offspring-Studio-Setup-X.Y.Z.exe` |
| Binary name | `offspring.exe` | `offspring-studio.exe` |
| Install path | `%LocalAppData%\Programs\Offspring` | `%LocalAppData%\Programs\Offspring Studio` |
| Data folder | `%AppData%\Offspring` | `%AppData%\Offspring Studio` |
| Classic right-click menu | Yes | Yes |
| Win11 modern (top-level) right-click menu | Yes (default on) | **No** |
| Self-signed cert in `CurrentUser\TrustedPeople` | Yes | **No, never** |
| Shipped shell-extension DLL + MSIX packages | Yes | **No** |
| Compile-time-included FFmpeg downloader | Yes (auto on first launch if not present, or manual via Settings) | **No (code compiled out)** |
| Compile-time-included in-app updater | Yes (minisign-verified) | **No (code compiled out)** |

Studio's "No" rows are not feature toggles. The Rust code is gated behind a Cargo feature flag (`studio`); building with `--features studio` *literally removes* the HTTP modules (`bootstrap.rs`, `updates.rs`) and the cert/MSIX integration paths (`modern_menu.rs`) from the compiled binary. The studio binary contains no code path that calls `gyan.dev`, `github.com`, or `certutil.exe`. The two variants can coexist on the same Windows account — separate AppIds, separate install dirs, separate data folders, separate registry namespaces.

**Use Studio when:** you're in an enterprise environment that disallows arbitrary MSIX package registrations or third-party certificate trusts; you want auditable proof (read the source under `#[cfg(feature = "studio")]`) that the binary cannot reach the network for its own purposes; you'd rather check for updates manually on GitHub than have the app do it.

**Use Standard when:** you want the Win11 top-level right-click menu, automatic FFmpeg setup, and minisign-verified in-app updates.

### Runtime dependencies

Both variants statically link the MSVC C runtime (`+crt-static` in `.cargo/config.toml`), so neither installer needs the VC++ Redistributable to be pre-installed on the user's machine. The .exe files contain no `vcruntime140.dll` / `vcruntime140_1.dll` / `msvcp140.dll` imports.

Both variants render their UI through Microsoft Edge WebView2, which ships pre-installed on Windows 11 and on mid-2021+ Windows 10 builds. Variants differ in how they handle a missing runtime:

- **Standard** bundles Microsoft's [Evergreen Bootstrapper](https://developer.microsoft.com/en-us/microsoft-edge/webview2/) (a ~1.7 MB Microsoft-signed binary) in the installer. The bootstrapper is only launched when `IsWebView2Installed` returns false (checked via the documented `HKLM\…\EdgeUpdate\Clients\{F3017226…}` / `HKCU\…` registry keys). On installs where WebView2 is already present — which is most of them — the bootstrapper never runs. When it does run, it's a Microsoft-signed binary contacting Microsoft's own update endpoints.
- **Studio** detects the missing runtime in `InitializeSetup` and shows an info dialog pointing the user at Microsoft's WebView2 download page, then aborts the install. No network call is ever made by the Studio installer itself. The user installs WebView2 from Microsoft directly (one-time, ~15 seconds) and re-runs the Studio installer. This is the deliberate cost of Studio's "no automatic outbound network" promise being literally true.

## Install scope and certificate trust

Offspring installs **per-user** under `%LocalAppData%\Programs\Offspring`,
the same scope used by VS Code, Discord, and Slack. The installer runs
without admin rights — no UAC prompt, no machine-wide changes, no
writes to `Program Files`, `HKLM`, or the `LocalMachine` certificate
stores.

The Windows 11 modern right-click menu is delivered as an MSIX sparse
package. `Add-AppxPackage` requires the package's signing certificate
to be present in `TrustedPeople`, so the installer imports our
self-signed cert into the invoking user's `Cert:\CurrentUser\TrustedPeople`
store. This grants signature trust **for that user only**, **for MSIX
manifests / signed documents only** — it is *not* added to
`Root`, `CA`, or any TLS path. It cannot be used by another machine to
authenticate to yours, and it cannot vouch for HTTPS servers, code
authenticode, or driver loading.

The modern-menu component (cert + MSIX) is an opt-in checkbox during
install, defaulting to ON. Unchecking it leaves the classic
right-click menu in place, with no cert installed. Users who skipped
it can opt in later from **Settings → Right-click menu → Set up
Windows 11 modern menu…** with no admin prompt.

Uninstalling removes the cert by FriendlyName + Subject match, so an
unrelated cert sharing the same CN cannot be removed by accident.

Releases prior to 0.4.4 installed per-machine and imported the cert
into `LocalMachine\TrustedPeople`. The 0.4.4 installer detects that
state and offers to uninstall the old version (one UAC prompt) before
proceeding with the per-user install.

## What we already do

- All update installers are signed with a pinned Ed25519 (minisign)
  key. The in-app updater refuses to launch any installer whose
  signature is missing or doesn't verify against the pinned public
  key in `src-tauri/src/updates.rs`.
- The FFmpeg download is integrity-checked before extraction. On
  Windows, the gyan.dev archive is verified against its published
  SHA-256 sidecar (constant-time comparison). On macOS, the SHA-256
  is pulled from evermeet.cx's info JSON endpoint and verified the
  same way when present; if the field is absent in a given response
  Offspring falls back to TLS-only and surfaces the gap to the user
  through the progress event log rather than failing silently.
- Update redirect targets are checked against an allowlist of
  GitHub-owned download hosts before any byte is fetched.
- Subprocesses (FFmpeg, offspring.exe, PowerShell scripts) are
  spawned via Win32 `CreateProcessW` directly with separate argv —
  no shell, no string interpolation.
- The shell-extension DLL has no inbound IPC; it only spawns
  offspring.exe via the path stored in `HKCU\Software\Offspring\ExePath`.
- Tauri 2 capabilities are scoped per-window with an explicit
  permission allowlist.
- Single-instance IPC uses a Win32 named pipe with
  `PIPE_REJECT_REMOTE_CLIENTS` (no network reachability).

See [THREAT_MODEL.md](./THREAT_MODEL.md) for the full picture and
[RELEASING.md](./RELEASING.md) for the signing workflow.
