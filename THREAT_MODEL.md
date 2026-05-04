# Threat model

A plain-English description of what Offspring does and doesn't try to
defend against. Written so a security reporter can quickly tell whether
an issue they're looking at is in scope, and so a contributor can tell
which design decisions are load-bearing.

## What Offspring is

A right-click utility that hands user-selected media files to a local
FFmpeg binary and writes the output back to the same folder. It runs as
a per-user Windows app that:

- Spawns FFmpeg as a subprocess with file paths the user picked in
  Explorer.
- Downloads FFmpeg from gyan.dev on first run if the user opted in.
- Checks GitHub Releases for new versions and offers to download +
  install them via Inno Setup.
- Registers itself with the Windows shell so right-clicking a file
  shows an "Offspring" submenu.
- Optionally registers a sparse-MSIX shell extension for Windows 11's
  modern menu (signed with a self-signed dev cert that the installer
  trusts at machine scope, admin-only).

It does not maintain network state, listen on any port, accept files
over the network, or run as a service.

## Trust boundaries

```
┌────────────────────────────────────────────────────────────────┐
│                      Local user session                         │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │  Explorer +  │  │  offspring   │  │ Other user-level     │  │
│  │  shell-ext   │──│   .exe       │  │ processes (browser,  │  │
│  │   DLL        │  │  (primary)   │  │ malware, …)          │  │
│  └──────────────┘  └──────┬───────┘  └──────────────────────┘  │
│                           │                                      │
│                           │ named pipe                          │
│                    ┌──────┴────────┐                             │
│                    │  offspring    │                             │
│                    │  .exe         │                             │
│                    │  (secondary,  │                             │
│                    │   exits fast) │                             │
│                    └───────────────┘                             │
└─────────────┬──────────────────────────┬───────────────────────┘
              │ HTTPS                    │ HTTPS
        ┌─────▼─────────┐         ┌─────▼─────────┐
        │  gyan.dev     │         │  GitHub       │
        │  (FFmpeg ZIP, │         │  (release     │
        │   sidecar     │         │   metadata,   │
        │   sha256)     │         │   installer,  │
        │               │         │   .minisig)   │
        └───────────────┘         └───────────────┘
```

## Attackers we defend against ("in scope")

### A. Network attacker against gyan.dev

A passive or active MITM, a DNS hijacker, a poisoned cache, or a
compromised gyan.dev mirror.

**Defense.** The downloaded FFmpeg ZIP is hashed during streaming and
compared to gyan.dev's `.sha256` sidecar URL. Mismatch is a hard error;
extraction does not run. Both URLs are HTTPS-only and ureq's default
TLS chain validation applies.

**Residual risk.** A full compromise of gyan.dev's TLS (their cert
private key) lets the attacker swap both files consistently. We accept
this — pinning gyan.dev's cert across rotations would brittle the FFmpeg
flow without a meaningful security gain (gyan.dev is the trust anchor
either way).

### B. Network attacker against GitHub Releases

The same set of attacks against the in-app updater's release-metadata
fetch and installer download.

**Defense.** Three layers:

1. The release JSON is fetched from `api.github.com`. Tags are filtered
   to plausible-semver shapes only.
2. The installer's redirect target is checked against an allowlist of
   GitHub-owned hosts (`github.com`, `objects.githubusercontent.com`,
   `release-assets.githubusercontent.com`). Anything else is refused
   before a single byte is downloaded.
3. Every installer must have a `.minisig` sidecar at the same URL
   pattern, and the signature must verify against the pinned Ed25519
   public key compiled into the app at build time. Missing or invalid
   signature → download is deleted, no install runs, the user sees an
   error.

**Residual risk.** A compromise of the maintainer's offline minisign
private key is game over. Mitigation: the key lives in
`installer/.minisign/` (gitignored) and on a separate offline backup,
never on a CI machine, and is password-protected. A key rotation
requires shipping a new pinned pubkey to existing installs, which can
only happen via an already-trusted update — chicken-and-egg, by design.

### C. Drive-by malicious file

A user right-clicks a file with an unusual name or contents. The shell
extension hands the path(s) to offspring.exe, which hands them to
FFmpeg.

**Defense.**

- Argument boundaries: `Command::spawn` calls `CreateProcessW`
  directly with separate argv elements. No shell. No format-string
  interpolation. A filename like `; rm -rf /` is a literal seven-byte
  string that FFmpeg sees as one argument.
- Output paths: filenames are derived from the input via
  `file_stem()`, which strips the directory and extension. Windows
  rejects path separators in filenames at the filesystem layer, so a
  malicious filename can't traverse out of the input's directory.
- FFmpeg itself: trusted to the same degree FFmpeg is trusted in
  general. If a malformed media file can RCE inside FFmpeg, that
  would be a vulnerability in FFmpeg, reported upstream.

**Residual risk.** None we control beyond FFmpeg's own hardening.

### D. Supply-chain compromise of build dependencies

A malicious crate makes it into a published release.

**Defense.** Cargo lockfile is committed; build pinning is
deterministic. Direct dependencies are minimal and well-known
(`tauri`, `serde`, `clap`, `dirs`, `anyhow`, `mslnk`, `winreg`,
`ureq`, `zip`, `sha2`, `minisign-verify`). Transitive deps are
inspected on each `cargo update`.

**Residual risk.** The standard cargo ecosystem caveats. We're not
running `cargo-audit` in CI yet — that's a future improvement.

### E. XSS or webview content injection

A code path that ends up rendering attacker-controlled text into the
DOM with a sink that allows script execution.

**Defense.** Strict CSP (`script-src 'self'`, `object-src 'none'`,
`frame-src 'none'`, `base-uri 'self'`, `form-action 'none'`). Svelte 5
escapes interpolations by default. The webview only loads bundled
assets — there is no remote content path. Tauri 2's IPC ACL gates
which JS can call which Rust commands.

## Attackers we do NOT defend against ("out of scope")

### F. Malware already running as the same Windows user

A process running under the same user account as Offspring already
has full read/write access to:

- `%APPDATA%\Offspring\*.json` (presets, settings, last-used trim values)
- `%LOCALAPPDATA%\Offspring\*` (FFmpeg binary, debug log)
- `HKCU\Software\Offspring\*` (the registry hive that tells the shell
  extension where offspring.exe lives)
- The installed offspring.exe and offspring_shell_ext.dll (Program
  Files is admin-write but we're an admin install — once installed,
  same-user-with-admin-once owns the binaries until uninstall)

There is no defense Offspring could plausibly add against this — at
that point the attacker already has the user's identity. We document
the limitation and accept it.

### G. Local administrator

If your machine's administrator account is hostile to you, every
piece of software running as your user is compromisable, including
this one. Out of scope.

### H. Physical access to an unlocked machine

Out of scope. Lock your screen.

### I. DoS by spamming the IPC pipe

A malicious local same-user process can connect to the named pipe
and forward fake argv. The worst this can do is cause Offspring's
primary instance to briefly process a bogus encode batch — no
filesystem effects beyond what FFmpeg would do for the supplied
arguments, which is bounded by the user's own permissions. We
deliberately don't authenticate pipe connections beyond the default
DACL because (F) makes it pointless.

### J. Tampered installer obtained outside GitHub Releases

If someone hands you `Offspring-Setup-via-shady-link.exe`, we have no
way to vet it. The official source is
`https://github.com/honear/offspring/releases/latest`. A tampered
installer running through Inno Setup is signed by us only if the
attacker also has our minisign private key — which they shouldn't,
per (B).

## Key trust assumptions, summarised

- **gyan.dev** is the trust anchor for the FFmpeg binary. Its TLS
  cert + the SHA-256 sidecar together are sufficient.
- **GitHub** is the trust anchor for release metadata, installer
  binaries, and signature sidecars. Its TLS chain is sufficient.
- **The maintainer's offline minisign private key** is the trust
  anchor for "is this installer authentic". Loss or compromise of
  that key requires a coordinated key-rotation event that can only
  reach existing installs via an already-trusted update.
- **The Windows user account boundary** is the trust anchor for
  everything else. We don't try to be a sandbox against ourselves.
