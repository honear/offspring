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
//! Cert trust: `Add-AppxPackage` validates MSIX signatures against
//! `CurrentUser\TrustedPeople` (or `LocalMachine\TrustedPeople` for
//! per-machine packages). The Inno installer imports the cert into the
//! per-user store during its `[Run]` phase, scoped to the invoking
//! user — no admin required.
//!
//! The `trust_cert_user_scope` function below is the in-app
//! equivalent: it lets users who opted out at install time, or who
//! are a second user on a shared PC, install the cert + register the
//! MSIX from inside the app without re-running the installer.

// The MSIX registration + cert-import code paths are gated out of the
// `studio` build entirely. Studio gets thin no-op stubs (see the
// bottom of this file) so callers in `commands.rs` / `integration::mod.rs`
// don't need to branch on build variant — they always get the same
// `sync` / `cleanup` / `trust_cert_user_scope` / `restart_explorer`
// surface, it just does nothing useful in studio.
#[cfg(not(feature = "studio"))]
use std::os::windows::process::CommandExt;
#[cfg(not(feature = "studio"))]
use std::path::{Path, PathBuf};
#[cfg(not(feature = "studio"))]
use std::process::Command;

use anyhow::Result;
#[cfg(not(feature = "studio"))]
use anyhow::{anyhow, Context};

use crate::presets::{Preset, Settings};

/// Win32 process-creation flag. Suppresses the console window that
/// `powershell.exe` would otherwise flash in front of our GUI for each
/// integration call (toggle save, first-run, etc.).
#[cfg(not(feature = "studio"))]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Resolve the absolute path to Windows PowerShell (5.x) under
/// `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`. Using
/// the absolute path avoids PATH-hijack scenarios where a malicious
/// `powershell.exe` planted in a writable PATH entry (or the current
/// working directory) would be invoked instead. Falls back to the bare
/// `powershell.exe` name if `%SystemRoot%` is somehow unset, so the call
/// still has a chance of working on a hardened/locked-down system.
#[cfg(not(feature = "studio"))]
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
#[cfg(not(feature = "studio"))]
struct Package {
    name: &'static str,
    msix: &'static str,
}

#[cfg(not(feature = "studio"))]
const PKG_UNIFIED: Package = Package {
    name: "SecondMarch.Offspring.ShellExt",
    msix: "OffspringShellExt.msix",
};
#[cfg(not(feature = "studio"))]
const PKG_PRESETS: Package = Package {
    name: "SecondMarch.Offspring.PresetsShellExt",
    msix: "OffspringShellExt.Presets.msix",
};
#[cfg(not(feature = "studio"))]
const PKG_TOOLS: Package = Package {
    name: "SecondMarch.Offspring.ToolsShellExt",
    msix: "OffspringShellExt.Tools.msix",
};

/// Every package this module knows about. Iterated by `cleanup` and
/// used to compute the "should be unregistered" set in `sync`.
#[cfg(not(feature = "studio"))]
const ALL_PACKAGES: &[&Package] = &[&PKG_UNIFIED, &PKG_PRESETS, &PKG_TOOLS];

#[cfg(not(feature = "studio"))]
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
///
/// Deliberately no `-ExecutionPolicy` flag: `-Command` invocations of
/// built-in cmdlets (Get-AppxPackage, Import-Certificate, etc.) aren't
/// gated by ExecutionPolicy in the first place, so adding `Bypass`
/// only ever bought us a scary string in process-monitor logs.
/// Sandbox reviewers and security-curious users see that string and
/// reasonably assume the worst — removing it eliminates the headline
/// concern without changing a single bit of behavior.
#[cfg(not(feature = "studio"))]
fn ps(script: &str) -> Result<()> {
    let output = Command::new(powershell_exe())
        .args([
            "-NoProfile",
            "-NonInteractive",
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
#[cfg(not(feature = "studio"))]
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
#[cfg(not(feature = "studio"))]
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
#[cfg(not(feature = "studio"))]
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
#[cfg(not(feature = "studio"))]
fn unregister(name: &str) -> Result<()> {
    ps(&format!(
        "Get-AppxPackage -Name '{}' -ErrorAction SilentlyContinue | \
         Remove-AppxPackage -ErrorAction SilentlyContinue",
        ps_escape_single(name)
    ))
}

#[cfg(not(feature = "studio"))]
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

/// Import the shipped shell-extension signing cert into
/// `Cert:\CurrentUser\TrustedPeople` so `Add-AppxPackage` accepts our
/// MSIX manifests. Scoped to the invoking user — no machine-wide
/// change, no admin rights, no UAC prompt.
///
/// Idempotent: re-importing the same cert silently succeeds (the
/// thumbprint match makes it a no-op). The cert file is expected
/// alongside the exe (`{app}\OffspringShellExt.cer`); if missing —
/// which it will be in dev builds that skipped the MSIX pipeline —
/// returns an Err explaining that.
///
/// This is the in-app counterpart to the installer's certutil call
/// run entry. Surfaced via the Settings "Set up Windows 11 modern
/// menu" button so users who unchecked the modern-menu component at
/// install, or who are a second user on a shared PC, can opt in
/// later without re-running the installer.
#[cfg(not(feature = "studio"))]
pub fn trust_cert_user_scope() -> Result<()> {
    let dir = install_dir()?;
    let cer = dir.join("OffspringShellExt.cer");
    if !cer.exists() {
        return Err(anyhow!(
            "shell-extension certificate not found at {} — \
             this build was packaged without the modern-menu assets",
            cer.display()
        ));
    }
    // Use certutil.exe (Microsoft-signed System32 tool) instead of
    // spawning PowerShell + Import-Certificate. Same end-state —
    // cert lands in Cert:\CurrentUser\TrustedPeople — without
    // any of the PowerShell-related sandbox flags (script_tool,
    // ExecutionPolicy string, etc.). Resolve the absolute path
    // under %SystemRoot%\System32 first as a path-hijack defense.
    let certutil = certutil_exe();
    let output = Command::new(&certutil)
        .args([
            "-user",
            "-addstore",
            "TrustedPeople",
            &cer.display().to_string(),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("launching certutil.exe")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(anyhow!(
            "certutil exited {}: {} {}",
            output.status,
            stderr.trim(),
            stdout.trim()
        ));
    }
    Ok(())
}

/// Resolve `%SystemRoot%\System32\certutil.exe`. Mirrors the same
/// path-hijack defense as `powershell_exe`: a malicious `certutil.exe`
/// planted in a writable PATH entry shouldn't be picked up over the
/// real one.
#[cfg(not(feature = "studio"))]
fn certutil_exe() -> PathBuf {
    if let Some(sysroot) = std::env::var_os("SystemRoot") {
        let p = PathBuf::from(sysroot).join("System32").join("certutil.exe");
        if p.exists() {
            return p;
        }
    }
    PathBuf::from("certutil.exe")
}

/// Kill and relaunch Explorer so it re-reads the modern-menu handler
/// list. Exposed as a Tauri command — the frontend calls it after
/// every toggle save.
#[cfg(not(feature = "studio"))]
pub fn restart_explorer() -> Result<()> {
    ps("Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue; \
        Start-Sleep -Milliseconds 300; \
        if (-not (Get-Process -Name explorer -ErrorAction SilentlyContinue)) { Start-Process explorer }")
}

// ----- studio build: cert + MSIX paths replaced with no-op stubs ---
//
// All four public functions keep their original signatures so callers
// don't have to know which build they're compiled into. `sync` and
// `cleanup` no-op because studio has no MSIX packages to register or
// remove. `restart_explorer` no-ops for the same reason (it only
// matters when the modern-menu handler list has changed).
// `trust_cert_user_scope` returns an error because the in-app
// "Reinstall modern menu" button has no business firing in a studio
// build — the frontend hides that button via `get_build_variant`, but
// this stub catches the case anyway.
#[cfg(feature = "studio")]
pub fn sync(_presets: &[Preset], _settings: &Settings) -> Result<()> {
    Ok(())
}

#[cfg(feature = "studio")]
pub fn cleanup() -> Result<()> {
    Ok(())
}

#[cfg(feature = "studio")]
pub fn trust_cert_user_scope() -> Result<()> {
    Err(anyhow::anyhow!(
        "Offspring Studio does not register the Windows 11 modern menu and never imports certificates."
    ))
}

#[cfg(feature = "studio")]
pub fn restart_explorer() -> Result<()> {
    Ok(())
}
