//! Windows 11 modern (top-level) right-click menu — MSIX sparse package
//! backed by the `IExplorerCommand` shell extension in `shell-ext/`.
//!
//! `sync` is idempotent: it runs `Add-AppxPackage -ExternalLocation
//! <install dir>`. Repeat calls short-circuit when the package is
//! already registered.
//!
//! Cert trust is NOT handled here. `Add-AppxPackage` validates MSIX
//! signatures against `LocalMachine\TrustedPeople`, which requires
//! admin rights to populate — so the installer imports the cert during
//! its elevated `[Run]` phase and the uninstaller removes it. Per-user
//! (non-elevated) installs skip the trust step, and flipping this
//! toggle on will surface Windows' own `0x800B0109` untrusted-root
//! error.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};

use crate::presets::Preset;

/// Matches `<Identity Name>` in `installer/msix/AppxManifest.xml`.
const PACKAGE_NAME: &str = "SecondMarch.Offspring.ShellExt";

/// Filename produced by `installer/msix/build-msix.ps1` and bundled into
/// the installer's `{app}\` directory. The `.cer` sibling is consumed by
/// the installer's elevated `[Run]` step and not referenced from here.
const MSIX_FILENAME: &str = "OffspringShellExt.msix";

fn install_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("resolving current exe")?;
    Ok(exe
        .parent()
        .ok_or_else(|| anyhow!("current_exe has no parent"))?
        .to_path_buf())
}

/// Run a single-command PowerShell snippet. Returns Ok on exit code 0,
/// surfacing stderr otherwise. `-NoProfile` + `-NonInteractive` keep
/// cargo-test and background invocations from stalling.
fn ps(script: &str) -> Result<()> {
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy", "Bypass",
            "-Command", script,
        ])
        .output()
        .context("launching powershell.exe")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "powershell exited {}: {}",
            output.status,
            stderr.trim()
        ));
    }
    Ok(())
}

fn is_registered() -> bool {
    let Ok(output) = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "if (Get-AppxPackage -Name '{PACKAGE_NAME}' -ErrorAction SilentlyContinue) {{ 'yes' }}"
            ),
        ])
        .output()
    else {
        return false;
    };
    let out = String::from_utf8_lossy(&output.stdout);
    out.trim() == "yes"
}

pub fn sync(_presets: &[Preset]) -> Result<()> {
    if is_registered() {
        // Already installed. The DLL reads presets.json on every flyout
        // expansion, so we don't need to re-register on preset changes.
        return Ok(());
    }

    let dir = install_dir()?;
    let msix = dir.join(MSIX_FILENAME);

    if !msix.exists() {
        // Installer didn't ship the MSIX artifact — probably a dev
        // build where the MSIX pipeline hasn't run. Don't error; just
        // no-op so the Settings toggle save doesn't blow up on devs.
        return Ok(());
    }

    register_package(&msix, &dir)?;
    Ok(())
}

pub fn cleanup() -> Result<()> {
    // -ErrorAction SilentlyContinue because the uninstaller calls this
    // unconditionally; the package may not be registered at all if the
    // user never flipped the toggle on.
    let _ = ps(&format!(
        "Get-AppxPackage -Name '{PACKAGE_NAME}' -ErrorAction SilentlyContinue | \
         Remove-AppxPackage -ErrorAction SilentlyContinue"
    ));
    Ok(())
}

fn register_package(msix: &Path, external_location: &Path) -> Result<()> {
    ps(&format!(
        "Add-AppxPackage -Path '{}' -ExternalLocation '{}'",
        msix.display(),
        external_location.display()
    ))
    .context("Add-AppxPackage")
}
