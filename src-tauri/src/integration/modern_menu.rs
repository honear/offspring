//! Windows 11 modern (top-level) right-click menu — MSIX sparse package
//! backed by the `IExplorerCommand` shell extension in `shell-ext/`.
//!
//! `sync` is idempotent: it imports the bundled cert into the per-user
//! TrustedPeople store (the only step that can prompt the user), then
//! runs `Add-AppxPackage -ExternalLocation <install dir>`. Repeat calls
//! short-circuit when the package is already registered.
//!
//! `cleanup` unregisters the package but intentionally leaves the cert
//! in TrustedPeople. The cert is signed by our private key only, so
//! leaving it trusted doesn't broaden the attack surface — and pulling
//! it out on every toggle flip would re-prompt the user next time they
//! flip the toggle back on, which is exactly the friction this surface
//! is trying to avoid.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};

use crate::presets::Preset;

/// Matches `<Identity Name>` in `installer/msix/AppxManifest.xml`.
const PACKAGE_NAME: &str = "SecondMarch.Offspring.ShellExt";

/// Filenames produced by `installer/msix/build-msix.ps1` and bundled into
/// the installer's `{app}\` directory.
const MSIX_FILENAME: &str = "OffspringShellExt.msix";
const CER_FILENAME: &str = "OffspringShellExt.cer";

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
    let cer = dir.join(CER_FILENAME);
    let msix = dir.join(MSIX_FILENAME);

    if !cer.exists() || !msix.exists() {
        // Installer didn't ship the MSIX artifacts — probably a dev
        // build where the MSIX pipeline hasn't run. Don't error; just
        // no-op so the Settings toggle save doesn't blow up on devs.
        return Ok(());
    }

    trust_cert(&cer)?;
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

fn trust_cert(cer: &Path) -> Result<()> {
    ps(&format!(
        "Import-Certificate -FilePath '{}' \
         -CertStoreLocation 'Cert:\\CurrentUser\\TrustedPeople' | Out-Null",
        cer.display()
    ))
    .context("importing cert into TrustedPeople")
}

fn register_package(msix: &Path, external_location: &Path) -> Result<()> {
    ps(&format!(
        "Add-AppxPackage -Path '{}' -ExternalLocation '{}'",
        msix.display(),
        external_location.display()
    ))
    .context("Add-AppxPackage")
}
