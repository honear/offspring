# Releasing Offspring

End-to-end release flow. This document is the single source of truth
for "how do I cut a new version?"; if it's wrong, fix this first and
then follow the corrected steps.

## TL;DR

```powershell
# Iterate locally as many times as you like — each build gets a -bNNNN suffix.
tools\build-release.ps1

# When the iteration is good, cut the actual release:
tools\build-release.ps1 -Release
tools\sign-release.ps1                         # produces .minisig sidecar

# Commit, tag, push, and publish:
git add -A
git commit -m "vX.Y.Z: <one-liner>"
git tag vX.Y.Z
git push origin main
git push origin vX.Y.Z
gh release create vX.Y.Z `
  installer\dist\Offspring-Setup-X.Y.Z.exe `
  installer\dist\Offspring-Setup-X.Y.Z.exe.minisig `
  installer\dist\Offspring-Setup.exe `
  --title "Offspring X.Y.Z" `
  --notes "..."
```

## Versioning scheme

- `0.3.42` — the last clean checkpoint, normally the previous published release.
- `0.3.42-b0001`, `0.3.42-b0002`, … — local iteration builds. The
  installer filename carries the suffix, and `Offspring-Setup.exe`
  (the public "latest" symlink) is **not** touched. These exist only
  so we can reproducibly bisect "the build I tried Tuesday" without
  burning patch versions on every tweak.
- `0.3.43` — the next published release, produced by
  `build-release.ps1 -Release`. The bumper strips the `-bNNNN`
  suffix and increments the patch number; the public
  `Offspring-Setup.exe` is refreshed to point at this build.

The `b` prefix on the counter exists because strict SemVer 2.0.0
forbids leading zeroes on numeric pre-release identifiers, which
breaks Tauri's config validator. `b0001` is alphanumeric and
parses cleanly.

`installer\offspring.iss` carries a separate `AppVersionMsix`
define in the four-numeric `MAJOR.MINOR.PATCH.BUILD` form Inno's
`VersionInfoVersion=` requires. The bumper writes both.

Files the bumper updates:

- `package.json`
- `src-tauri/Cargo.toml` (and its lock file)
- `src-tauri/tauri.conf.json`
- `shell-ext/Cargo.toml` (and its lock file)
- `installer/offspring.iss` (`AppVersion` and `AppVersionMsix`)
- `package-lock.json` (synced via `npm install --package-lock-only`)

## Step-by-step release

### 1. Iterate locally

```powershell
tools\build-release.ps1
```

Each invocation:

1. Bumps the build counter (`0.3.42` → `0.3.42-b0001` → `0.3.42-b0002`).
2. Runs `npm run tauri build`.
3. Builds the shell-extension DLL.
4. Builds and signs the sparse MSIX (with the dev cert at
   `installer/msix/.cert/offspring-shellext.pfx` — separate from the
   minisign signing key; this is for Windows shell-extension trust).
5. Compiles the Inno Setup installer.

Output: `installer\dist\Offspring-Setup-0.3.42-bNNNN.exe`. Install it
and test. Repeat as needed.

### 2. Cut the release

```powershell
tools\build-release.ps1 -Release
```

Same five steps, but with the version finalised: `0.3.42-bNNNN` →
`0.3.43`. The unversioned `Offspring-Setup.exe` symlink is refreshed
because this is the build we'll actually publish.

### 3. Sign the installer

```powershell
tools\sign-release.ps1
```

Produces `installer\dist\Offspring-Setup-0.3.43.exe.minisig` next to
the installer. The script:

- Looks for the private key at `installer\.minisign\offspring.key`
  (path overridable via `-KeyPath`).
- Prompts for the key's password — set
  `MINISIGN_KEY_PASSWORD` in the environment if you want to skip the
  prompt for batch use, but this is rare in practice.
- Refuses to overwrite an existing `.minisig` (which would silently
  re-sign and confuse later verification). If you need to re-sign,
  delete the old `.minisig` first.

Verify locally before publishing:

```powershell
minisign -Vm installer\dist\Offspring-Setup-0.3.43.exe `
         -p installer\.minisign\offspring.pub
```

You should see "Signature and comment signature verified". If not,
**stop** — something is wrong with either the build or the key.

### 4. Commit, tag, push

```powershell
git add -A
git commit -m "v0.3.43: <one-liner describing user-visible change>"
git tag v0.3.43
git push origin main
git push origin v0.3.43
```

The commit will include all the version-file bumps the bumper made.
The tag format is `vX.Y.Z` — the in-app updater's tag filter
(`is_plausible_tag` in [updates.rs](src-tauri/src/updates.rs))
expects that exact shape.

### 5. Publish on GitHub

```powershell
gh release create v0.3.43 `
  installer\dist\Offspring-Setup-0.3.43.exe `
  installer\dist\Offspring-Setup-0.3.43.exe.minisig `
  installer\dist\Offspring-Setup.exe `
  --title "Offspring 0.3.43" `
  --notes-file release-notes-0.3.43.md
```

Three assets attached:

- `Offspring-Setup-0.3.43.exe` — the versioned installer.
- `Offspring-Setup-0.3.43.exe.minisig` — the signature the in-app
  updater will fetch and verify. **Without this, every existing
  install will refuse to update.**
- `Offspring-Setup.exe` — the unversioned forever-link asset for
  marketing pages: `https://github.com/.../releases/latest/download/Offspring-Setup.exe`.

After publishing, sanity-check the page in a browser. Make sure
both the installer and the `.minisig` are listed. Without the
sidecar, the in-app updater on every existing install will see
"signature missing → refuse to install" and the release will be a
de facto soft brick.

## Key handling

The minisign signing key lives at one of:

- `installer\.minisign\offspring.key` inside the repo (gitignored), OR
- A path outside the repo (recommended), with the location pointed at
  by the `OFFSPRING_MINISIGN_KEY` environment variable. Setting it
  permanently in your PowerShell profile means `tools\sign-release.ps1`
  Just Works without per-invocation flags:

  ```powershell
  # In $PROFILE (run `notepad $PROFILE` to edit)
  $env:OFFSPRING_MINISIGN_KEY = "C:\Users\You\installer\.minisign\offspring.key"
  ```

  Outside-the-repo storage is structurally safer — no gitignore mistake
  can ever expose it.

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
the old signature). Re-run `tools\sign-release.ps1` and re-upload
both files together.

**`gh release create` fails with a tag-not-found error.** Push the
tag first (`git push origin vX.Y.Z`).

**The release shows the wrong "latest" filename.** The unversioned
`Offspring-Setup.exe` is just a copy `build-release.ps1 -Release`
makes. Re-run that step and re-attach.
