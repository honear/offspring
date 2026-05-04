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

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};

use crate::presets::Preset;

/// Win32 process-creation flag. Suppresses the console window that
/// `powershell.exe` would otherwise flash in front of our GUI for each
/// integration call (toggle save, first-run, etc.).
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Resolve the absolute path to Windows PowerShell (5.x) under
/// `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`. Using
/// the absolute path avoids PATH-hijack scenarios where a malicious
/// `powershell.exe` planted in a writable PATH entry (or the current
/// working directory) would be invoked instead. Falls back to the bare
/// `powershell.exe` name if `%SystemRoot%` is somehow unset, so the call
/// still has a chance of working on a hardened/locked-down system.
fn powershell_exe() -> PathBuf {
    if let Some(sysroot) = std::env::var_os("SystemRoot") {
        let p = PathBuf::from(sysroot)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        if p.exists() {
            return p;
        }
    }
    PathBuf::from("powershell.exe")
}

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
    let output = Command::new(powershell_exe())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy", "Bypass",
            "-Command", script,
        ])
        .creation_flags(CREATE_NO_WINDOW)
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

/// Escape an arbitrary string for use inside a PowerShell single-quoted
/// literal: `'` becomes `''`, which is PowerShell's only in-quote escape
/// for single quotes. Anything else is preserved verbatim — single
/// quotes don't honour backslash, `$`, or backtick escapes, so a path
/// like `C:\Users\O'Brien` round-trips cleanly as `'C:\Users\O''Brien'`.
fn ps_escape_single(s: &str) -> String {
    s.replace('\'', "''")
}

fn is_registered() -> bool {
    let Ok(output) = Command::new(powershell_exe())
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "if (Get-AppxPackage -Name '{}' -ErrorAction SilentlyContinue) {{ 'yes' }}",
                ps_escape_single(PACKAGE_NAME)
            ),
        ])
        .creation_flags(CREATE_NO_WINDOW)
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
    // Explorer caches the modern-menu handler list aggressively, so the
    // new entry may not appear until Explorer re-launches. We don't do
    // that here — killing Explorer wipes the user's open windows. The
    // frontend prompts after a successful toggle and calls
    // `restart_explorer` only if the user opts in.
    Ok(())
}

pub fn cleanup() -> Result<()> {
    // -ErrorAction SilentlyContinue because the uninstaller calls this
    // unconditionally; the package may not be registered at all if the
    // user never flipped the toggle on.
    let _ = ps(&format!(
        "Get-AppxPackage -Name '{}' -ErrorAction SilentlyContinue | \
         Remove-AppxPackage -ErrorAction SilentlyContinue",
        ps_escape_single(PACKAGE_NAME)
    ));
    Ok(())
}

fn register_package(msix: &Path, external_location: &Path) -> Result<()> {
    // Single-quote escape both paths so a username with `'` in it
    // (`C:\Users\O'Brien\…`) can't break out of the PowerShell literal.
    ps(&format!(
        "Add-AppxPackage -Path '{}' -ExternalLocation '{}'",
        ps_escape_single(&msix.display().to_string()),
        ps_escape_single(&external_location.display().to_string())
    ))
    .context("Add-AppxPackage")
}

/// Kill and relaunch Explorer so it re-reads the modern-menu handler
/// list. Exposed as a Tauri command — the frontend calls it after the
/// user confirms via a dialog, never silently from `sync`.
pub fn restart_explorer() -> Result<()> {
    ps("Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue; \
        Start-Sleep -Milliseconds 300; \
        if (-not (Get-Process -Name explorer -ErrorAction SilentlyContinue)) { Start-Process explorer }")
}
