# Releasing Offspring

End-to-end release flow. This document is the single source of truth
for "how do I cut a new version?"; if it's wrong, fix this first and
then follow the corrected steps.

## TL;DR

```powershell
# Iterate locally as many times as you like — each build gets a -bNNNN suffix.
pwsh tools\build-release.ps1 -SkipInstall

# When the iteration is good, cut the actual release:
pwsh tools\build-release.ps1 -Version 0.4.4      # explicit version (preferred)
# OR:   pwsh tools\build-release.ps1 -Release    # auto-bump patch

pwsh tools\sign-release.ps1                      # interactive passphrase

# Duplicate the .minisig for the byte-identical "latest" alias:
Copy-Item installer\dist\Offspring-Setup-0.4.4.exe.minisig `
          installer\dist\Offspring-Setup.exe.minisig -Force

# Commit, tag, push, and publish:
git add -A
git commit -m "v0.4.4: <one-liner>"
git tag -a v0.4.4 -m "v0.4.4`n`n<release notes here>"
git push origin main
git push origin v0.4.4
gh release create v0.4.4 `
  installer\dist\Offspring-Setup-0.4.4.exe `
  installer\dist\Offspring-Setup-0.4.4.exe.minisig `
  installer\dist\Offspring-Setup.exe `
  installer\dist\Offspring-Setup.exe.minisig `
  --repo Second-March/offspring `
  --title "Offspring 0.4.4" `
  --notes "<release notes>"
```

**Four assets every release.** Versioned `.exe` + versioned `.minisig`
+ unversioned `Offspring-Setup.exe` (forever-link) + unversioned
`.minisig` (so the marketing-site one-click download is also
signature-verifiable). The two `.exe`s are byte-identical, so the same
`.minisig` is valid for both — copying the file is correct, not a
workaround.

## Versioning scheme

- `0.4.3` — the last clean checkpoint, normally the previous published release.
- `0.4.3-b0001`, `0.4.3-b0002`, … — local iteration builds. The
  installer filename carries the suffix, and `Offspring-Setup.exe`
  (the public "latest" symlink) is **not** touched. These exist only
  so we can reproducibly bisect "the build I tried Tuesday" without
  burning patch versions on every tweak.
- `0.4.4` — the next published release, produced by
  `build-release.ps1 -Version 0.4.4` (or `-Release` for an auto-patch
  bump). The bumper strips the `-bNNNN` suffix and writes the new
  version; the public `Offspring-Setup.exe` is refreshed to point at
  this build.

The `b` prefix on the counter exists because strict SemVer 2.0.0
forbids leading zeroes on numeric pre-release identifiers, which
breaks Tauri's config validator. `b0001` is alphanumeric and
parses cleanly.

`installer\offspring.iss` carries a separate `AppVersionMsix`
define in the four-numeric `MAJOR.MINOR.PATCH.BUILD` form Inno's
`VersionInfoVersion=` requires. The bumper writes both.

Files the bumper updates (8 of them):

- `package.json`
- `package-lock.json` (synced via `npm install --package-lock-only`)
- `src-tauri/Cargo.toml` and `src-tauri/Cargo.lock`
- `src-tauri/tauri.conf.json`
- `shell-ext/Cargo.toml` and `shell-ext/Cargo.lock`
- `installer/offspring.iss` (`AppVersion` and `AppVersionMsix`)

## Step-by-step release

### 1. Iterate locally

```powershell
pwsh tools\build-release.ps1 -SkipInstall
```

Each invocation:

1. Bumps the build counter (`0.4.3` → `0.4.3-b0001` → `0.4.3-b0002`).
2. Runs `npm run tauri build`.
3. Builds the shell-extension DLL.
4. Builds and signs **three** sparse MSIX packages (`OffspringShellExt.msix`,
   `…Presets.msix`, `…Tools.msix`) with the dev cert at
   `installer/msix/.cert/offspring-shellext.pfx`. The cert is separate
   from the minisign signing key; this is for Windows shell-extension
   trust, used by the modern-menu integration.
5. Compiles the Inno Setup installer (bundles all three MSIX + the
   shared `.cer` + the shell-ext DLL + `offspring.exe`).

Output: `installer\dist\Offspring-Setup-0.4.3-bNNNN.exe`. Install it
and test. Repeat as needed. `-SkipInstall` skips `npm ci` to make
repeat builds fast — only re-run without it when `package.json`
dependencies actually change.

### 2. Cut the release

```powershell
pwsh tools\build-release.ps1 -Version 0.4.4
```

Same five steps, but with the version finalised. Use `-Version X.Y.Z`
explicitly for clarity; `-Release` (auto patch-bump) also works for
trivial follow-ups.

The unversioned `Offspring-Setup.exe` symlink is refreshed because
this is the build we'll actually publish.

### 3. Sign the installer

```powershell
pwsh tools\sign-release.ps1
```

Produces `installer\dist\Offspring-Setup-0.4.4.exe.minisig` next to
the installer. The script:

- Looks for the private key at `installer\.minisign\offspring.key`
  (path overridable via `-KeyPath` or `$env:OFFSPRING_MINISIGN_KEY`).
- Prompts for the key's password — set
  `MINISIGN_KEY_PASSWORD` in the environment if you want to skip the
  prompt for batch use, but this is rare in practice.
- Refuses to overwrite an existing `.minisig` (which would silently
  re-sign and confuse later verification). If you need to re-sign,
  delete the old `.minisig` first.

**Then duplicate the .minisig for the latest alias** — the
unversioned `Offspring-Setup.exe` is a byte-identical copy of the
versioned installer, so the same signature verifies both:

```powershell
Copy-Item installer\dist\Offspring-Setup-0.4.4.exe.minisig `
          installer\dist\Offspring-Setup.exe.minisig -Force
```

Sanity-check both before publishing:

```powershell
minisign -Vm installer\dist\Offspring-Setup-0.4.4.exe `
         -p installer\.minisign\offspring.pub
minisign -Vm installer\dist\Offspring-Setup.exe `
         -p installer\.minisign\offspring.pub
```

Both should say "Signature and comment signature verified". If
either fails, **stop** — something is wrong with the build or key.

### 4. Commit, tag, push

```powershell
git add -A
git commit -m "v0.4.4: <one-liner describing user-visible change>"
git tag -a v0.4.4 -m "v0.4.4`n`n<short release notes inside the annotation>"
git push origin main
git push origin v0.4.4
```

The commit will include all the version-file bumps the bumper made.
The tag format is `vX.Y.Z` — the in-app updater's tag filter
(`is_plausible_tag` in [updates.rs](src-tauri/src/updates.rs))
expects that exact shape. Use **annotated** tags (`-a`), never
lightweight tags; the annotation is the release-notes seed if you
don't override with `gh release create --notes`.

### 5. Publish on GitHub

```powershell
gh release create v0.4.4 `
  installer\dist\Offspring-Setup-0.4.4.exe `
  installer\dist\Offspring-Setup-0.4.4.exe.minisig `
  installer\dist\Offspring-Setup.exe `
  installer\dist\Offspring-Setup.exe.minisig `
  --repo Second-March/offspring `
  --title "Offspring 0.4.4" `
  --notes "$(cat <<'EOF'
## Highlights

- ...

## Install

Download [Offspring-Setup-0.4.4.exe](https://github.com/Second-March/offspring/releases/download/v0.4.4/Offspring-Setup-0.4.4.exe).
The installer is signed offline with minisign:

```
minisign -Vm Offspring-Setup-0.4.4.exe -P RWSozxN0N0fWyF2cXP0fC+q5Hg2kb2zW/ML+e+zItvm7A8BCXNLZunjr
```
EOF
)"
```

**Four assets attached:**

- `Offspring-Setup-0.4.4.exe` — the versioned installer.
- `Offspring-Setup-0.4.4.exe.minisig` — the signature the in-app
  updater fetches and verifies. **Without this, every existing
  install will refuse to update.**
- `Offspring-Setup.exe` — the unversioned forever-link asset for
  marketing pages: `https://github.com/Second-March/offspring/releases/latest/download/Offspring-Setup.exe`.
- `Offspring-Setup.exe.minisig` — sig for the forever-link, so the
  marketing-site one-click download is also signature-verifiable.

After publishing, sanity-check the page in a browser. Make sure all
four files are listed. Without the versioned sidecar, the in-app
updater on every existing install will see "signature missing →
refuse to install" and the release will be a de facto soft brick.

## Repo location

The canonical repo is **github.com/Second-March/offspring**. The
older `github.com/honear/offspring` URL still resolves via GitHub's
permanent 301 redirect (using the immutable numeric repo ID, so it's
transfer-proof). Older installs that hardcoded the `honear` slug
continue to find new releases via that redirect.

**Do not** create a new repo named `offspring` under the `honear`
account or delete that account — either action kills the redirect
and breaks the update path for all existing 0.4.2-and-earlier
installs.

The in-app `GITHUB_SLUG` constant in `src-tauri/src/updates.rs` was
flipped to `second-march/offspring` in 0.4.3. Fresh installs hit the
new URL directly; older installs continue to redirect.

## Key handling

The minisign signing key lives at one of:

- `installer\.minisign\offspring.key` inside the repo (gitignored). **This
  is the canonical location now**; `sign-release.ps1` resolves to this
  by default with zero args.
- A path outside the repo, with the location pointed at by the
  `$env:OFFSPRING_MINISIGN_KEY` environment variable. Setting it in
  your PowerShell profile means `pwsh tools\sign-release.ps1` Just
  Works without per-invocation flags:

  ```powershell
  # In $PROFILE (run `notepad $PROFILE` to edit)
  $env:OFFSPRING_MINISIGN_KEY = "C:\path\to\offspring.key"
  ```

Outside-the-repo storage is structurally safer — no gitignore mistake
can ever expose it. But the in-repo `.minisign/` directory is
gitignored three ways over (the dir + `*.key` + `*.pub`) so it's
also a defensible default.

**Never commit the key file under any circumstance.**

You should also have:

- The matching public key file (`offspring.pub`) — this is the source
  of truth for the constant pasted into
  `src-tauri/src/updates.rs:UPDATE_MINISIGN_PUBKEY`. Keeping the file
  alongside the private key is fine; it's also fine to discard it
  since the constant is the authoritative copy.
- An offline backup of the private key + its password. A USB stick
  in a drawer is the floor; an encrypted backup somewhere is better.
  Without these, if your machine dies, future updates can never be
  signed under this identity → users would see "signature did not
  verify" errors and be unable to auto-update. Recovery in that
  case is a manual key-rotation announcement + new release with a
  new pubkey, and existing installs would need to be re-installed
  by the user.

### Rotating the key

If the private key is ever compromised, lost, or you simply want to
change it:

1. Generate a new keypair (`minisign -G ...`).
2. Update `UPDATE_MINISIGN_PUBKEY` in
   `src-tauri/src/updates.rs` with the new public key.
3. Cut a new release **signed with the old key**, containing the
   new pubkey. Existing installs will accept this update because
   they still trust the old key.
4. From the next release onward, sign with the new key.
5. Optionally, post a security advisory if the rotation was
   compromise-driven.

This is exactly the chicken-and-egg property the threat model relies
on — there's no way for an unrelated party to rotate the key without
already being trusted.

## Troubleshooting

**`build-release.ps1` complains about a "could not parse current
version".** The version string in `package.json` is in an unexpected
shape. The bumper expects `X.Y.Z` or `X.Y.Z-bNNNN`. Fix by hand and
re-run.

**`sign-release.ps1` says "minisign.exe not found on PATH".** Install
it: `winget install jedisct1.minisign` (closes & reopens the
terminal so PATH picks up the new exe).

**The in-app updater on a freshly-installed copy says "signature did
not verify".** Either you forgot to upload the `.minisig`, or you
re-built the installer after signing (changing its bytes invalidates
the old signature). Re-run `pwsh tools\sign-release.ps1`, re-copy the
.minisig to the latest alias, and re-upload all four files together.

**`gh release create` fails with a tag-not-found error.** Push the
tag first (`git push origin vX.Y.Z`).

**The release shows the wrong "latest" filename.** The unversioned
`Offspring-Setup.exe` is just a copy `build-release.ps1 -Release`
makes. Re-run that step and re-attach.

**`gh release create` says "repository moved".** Update the local
remote: `git remote set-url origin https://github.com/Second-March/offspring.git`.
The push succeeded via the redirect, but it's cleaner to use the
canonical URL going forward.

**Windows SmartScreen warns "Windows protected your PC" on the
installer.** Expected — we don't ship an Authenticode (code-signing)
certificate. Users have to click *More info → Run anyway*. The
minisign signature is independent and verifies fine; the SmartScreen
warning is a reputation system, not a malware detection. Mention
this in launch posts so users aren't surprised.
