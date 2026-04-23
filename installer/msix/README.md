# MSIX sparse package — modern right-click menu

This directory builds the sparse MSIX that registers Offspring with the
Windows 11 top-level right-click menu (the one that appears _without_
clicking "Show more options").

## When this runs

Never on a user's machine. The `.msix` is produced at release time,
signed with our self-signed dev cert, and bundled into the Inno Setup
installer. The Settings toggle "Integrate with the Windows 11 modern
right-click menu" is what decides whether the user registers the
package on their own machine (see `integration::modern_menu` in the
main Rust code, Phase 5).

## Build locally

```powershell
# 1. Build the shell-ext DLL into offspring.exe's release dir (the
#    installer's CARGO_TARGET_DIR handling covers both paths).
cd shell-ext
cargo build --release

# 2. Pack + sign the sparse MSIX. Generates a dev cert on first run
#    and caches it at installer/msix/.cert/offspring-shellext.pfx.
cd ..
pwsh installer\msix\build-msix.ps1
```

Outputs:

```
installer\msix\dist\OffspringShellExt.msix   # signed sparse package
installer\msix\dist\OffspringShellExt.cer    # trust anchor, shipped to user
```

## What the user sees

When the user flips the modern-menu Settings toggle on, the app:

1. Imports `OffspringShellExt.cer` into `Cert:\CurrentUser\TrustedPeople`.
2. Calls `Add-AppxPackage -ExternalLocation "<install dir>" <msix>`.

Step 1 is the one that prompts for trust on first install — it's the
only UAC-like hit they get. Flipping the toggle off cleans up both.

## The CLSID contract

The CLSID `{4A8F1E2B-6C9D-4E1F-8A2B-3C4D5E6F7A8B}` appears in three places:

- `shell-ext/src/lib.rs::ROOT_CLSID`
- `installer/msix/AppxManifest.xml` (`<com:Class Id>` and `<desktop5:Verb Clsid>`)

Change any, change all, and `Remove-AppxPackage` the old identity before
installing the new one or Explorer will wedge on the stale registration.

## Why sparse and not a full MSIX

Offspring is distributed as a classic Win32 app via Inno Setup — moving
to a full MSIX would mean users can't pick their install path, can't use
portable-style configs, and the update story would go through the Store
model instead of GitHub Releases. Sparse gives us the shell-ext
surface without any of that lock-in: the MSIX is literally just this
manifest + a few logos, and all the real bits live in Program Files
alongside `offspring.exe`.
