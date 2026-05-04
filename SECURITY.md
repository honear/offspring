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
[github.com/honear/offspring/releases](https://github.com/honear/offspring/releases)
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

Offspring makes **no analytics, telemetry, or "phone-home" calls**. The
only outbound network traffic the app ever generates is:

| When | Where | Why |
|---|---|---|
| On launch | `https://api.github.com/repos/honear/offspring/releases/latest` | Update check. Fire-and-forget; failures collapse to "no update available" with no UI. The request carries the running version in the `User-Agent` header for release-page traffic stats; no other identifying data. |
| When the user clicks "Restart and install" on a pending update | GitHub-owned download host (one of `github.com`, `objects.githubusercontent.com`, `release-assets.githubusercontent.com`) | Downloads the installer .exe and its `.minisig` sidecar. Refuses to fetch from any other host. |
| When the user clicks "Download FFmpeg" in Settings (or accepts the prompt on first install) | `https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip` and the matching `.sha256` sidecar | One-time FFmpeg fetch. After that, no further gyan.dev traffic. |

That's the complete list. No background pings, no crash reports, no
usage stats, no third-party SDKs, no remote config, no A/B tests. The
in-app debug log lives only on the user's machine
(`%LOCALAPPDATA%\Offspring\debug.log`) and is never uploaded.

## What we already do

- All update installers are signed with a pinned Ed25519 (minisign)
  key. The in-app updater refuses to launch any installer whose
  signature is missing or doesn't verify against the pinned public
  key in `src-tauri/src/updates.rs`.
- The FFmpeg download is verified against gyan.dev's published
  SHA-256 sidecar before extraction.
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
