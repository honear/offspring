//! Windows 11 modern (top-level) right-click menu — three MSIX sparse
//! packages backed by the `IExplorerCommand` shell extension in
//! `shell-ext/`.
//!
//! THREE packages, ONE shared DLL:
//!
//!   * `SecondMarch.Offspring.ShellExt`           — "Offspring"           (Unified)
//!   * `SecondMarch.Offspring.PresetsShellExt`    — "Offspring Presets"   (split mode)
//!   * `SecondMarch.Offspring.ToolsShellExt`      — "Offspring Tools"     (split mode)
//!
//! Each package has a distinct `Identity Name` + `DisplayName` so Win11
//! doesn't auto-group the verbs under one parent. The split-layout
//! setting decides which packages are registered at any given time —
//! `sync` registers either {Unified} or {Presets, Tools} and
//! unregisters whatever's in the OTHER group, so the user only sees
//! the entries they asked for.
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

use crate::presets::{Preset, Settings};

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

/// One package = one (Identity Name, MSIX filename) pair. The Identity
/// matches `<Identity Name>` in the manifest variant — used to look up
/// the registered package via `Get-AppxPackage -Name`. The filename is
/// what `installer/msix/build-msix.ps1` produces and what the Inno
/// installer drops into `{app}\`.
struct Package {
    name: &'static str,
    msix: &'static str,
}

const PKG_UNIFIED: Package = Package {
    name: "SecondMarch.Offspring.ShellExt",
    msix: "OffspringShellExt.msix",
};
const PKG_PRESETS: Package = Package {
    name: "SecondMarch.Offspring.PresetsShellExt",
    msix: "OffspringShellExt.Presets.msix",
};
const PKG_TOOLS: Package = Package {
    name: "SecondMarch.Offspring.ToolsShellExt",
    msix: "OffspringShellExt.Tools.msix",
};

/// Every package this module knows about. Iterated by `cleanup` and
/// used to compute the "should be unregistered" set in `sync`.
const ALL_PACKAGES: &[&Package] = &[&PKG_UNIFIED, &PKG_PRESETS, &PKG_TOOLS];

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

/// Bring the three modern-menu packages into the state requested by
/// the user's split-layout setting. Registers whichever packages
/// SHOULD be installed and unregisters whichever ones SHOULDN'T —
/// flipping the toggle in Settings is therefore a single sync call
/// away from the right shell surface.
///
/// Explorer caches the modern-menu handler list aggressively, so a
/// freshly-registered manifest may not appear until Explorer
/// re-launches. We don't kill it here — the frontend calls
/// `restart_explorer` after a successful toggle.
pub fn sync(_presets: &[Preset], settings: &Settings) -> Result<()> {
    let dir = install_dir()?;
    let split = settings.modern_menu_split_layout.unwrap_or(false);
    let wanted: &[&Package] = if split {
        &[&PKG_PRESETS, &PKG_TOOLS]
    } else {
        &[&PKG_UNIFIED]
    };

    // 1. Unregister anything we don't want. This handles the toggle-
    //    flip case (was unified, now split — drop Unified before adding
    //    Presets+Tools so we don't transiently show three entries).
    for pkg in ALL_PACKAGES {
        if !wanted.iter().any(|w| std::ptr::eq(*w, *pkg)) {
            let _ = unregister(pkg.name);
        }
    }

    // 2. Register everything we want. Skip files that aren't shipped
    //    (dev builds may not have run the MSIX pipeline) rather than
    //    erroring — the toggle save shouldn't blow up on developers.
    for pkg in wanted {
        let msix = dir.join(pkg.msix);
        if !msix.exists() {
            continue;
        }
        register_package(&msix, &dir)?;
    }
    Ok(())
}

/// Remove every modern-menu package this module knows about. Called by
/// the global `cleanup_all` at uninstall time, and by `sync_all` when
/// the user disables the modern menu entirely.
pub fn cleanup() -> Result<()> {
    for pkg in ALL_PACKAGES {
        let _ = unregister(pkg.name);
    }
    Ok(())
}

/// `Remove-AppxPackage` the named identity, silently no-op'ing if it's
/// not installed. Returns the underlying PowerShell error verbatim if
/// the cmdlet fails for some other reason (permissions, COM-server in
/// use, …) so the caller can surface it.
fn unregister(name: &str) -> Result<()> {
    ps(&format!(
        "Get-AppxPackage -Name '{}' -ErrorAction SilentlyContinue | \
         Remove-AppxPackage -ErrorAction SilentlyContinue",
        ps_escape_single(name)
    ))
}

fn register_package(msix: &Path, external_location: &Path) -> Result<()> {
    // Single-quote escape both paths so a username with `'` in it
    // (`C:\Users\O'Brien\…`) can't break out of the PowerShell literal.
    //
    // Tolerate the "package already installed at this version" error
    // path: Add-AppxPackage throws when asked to install a version
    // equal to the one already registered. That's a normal outcome
    // every time we sync after install (e.g. user toggles Settings
    // with no version bump in between), so we ignore it. Anything
    // else propagates as a real failure.
    let script = format!(
        "$ErrorActionPreference = 'Stop'; \
         try {{ Add-AppxPackage -Path '{}' -ExternalLocation '{}' }} \
         catch {{ \
           $m = $_.Exception.Message; \
           if ($m -match 'already installed' -or \
               $m -match 'higher version' -or \
               $m -match '0x80073D06') {{ exit 0 }}; \
           throw \
         }}",
        ps_escape_single(&msix.display().to_string()),
        ps_escape_single(&external_location.display().to_string())
    );
    ps(&script).context("Add-AppxPackage")
}

/// Kill and relaunch Explorer so it re-reads the modern-menu handler
/// list. Exposed as a Tauri command — the frontend calls it after
/// every toggle save.
pub fn restart_explorer() -> Result<()> {
    ps("Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue; \
        Start-Sleep -Milliseconds 300; \
        if (-not (Get-Process -Name explorer -ErrorAction SilentlyContinue)) { Start-Process explorer }")
}
