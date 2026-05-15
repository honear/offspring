//! Shell integrations — the places where Offspring shows up outside the app
//! window.
//!
//! Three back-ends live here:
//!
//! * `context_menu` — writes `HKCU\Software\Classes\*\shell\Offspring` with
//!   an `ExtendedSubCommandsKey` pointing at per-preset verbs. This is the
//!   classic ("Show more options") right-click menu. Default-on, but
//!   automatically disabled when `modern_menu` is enabled — otherwise the
//!   two surfaces stack (modern entry at the top level AND classic entry
//!   under "Show more options"), which looks like a duplicate install.
//!
//! * `sendto` — writes .lnk files into the user's SendTo folder. Opt-in via
//!   `Settings.sendto_enabled`. Off by default because SendTo is buried
//!   under "Show more options" on Windows 11 anyway, making the registry
//!   menu strictly better for that audience.
//!
//! * `modern_menu` (see `modern_menu` submodule — landing in Phase 5) — an
//!   MSIX sparse package registering an `IExplorerCommand` COM handler to
//!   surface Offspring in the Windows 11 _top-level_ right-click menu.
//!   Opt-in via `Settings.modern_menu_enabled` because enabling it prompts
//!   the user to trust our self-signed cert.
//!
//! `sync_all(presets, settings)` is the single entry point that callers
//! (first-run hook, save_presets/save_settings commands) should use — it
//! applies each integration according to the toggles and leaves the OS in
//! a state that matches the current app config.

// All three back-ends are Windows-only: context_menu writes HKCU
// registry keys, sendto creates Win32 .lnk files via mslnk, and
// modern_menu drives MSIX/PowerShell. None of them have a macOS
// analogue today — the eventual Mac right-click surface (NSServices,
// Info.plist) lives in a separate module written when that work
// lands. Until then, the macOS build sees stubbed sync_all / cleanup_all
// implementations below.
#[cfg(windows)]
pub mod context_menu;
#[cfg(windows)]
pub mod modern_menu;
#[cfg(windows)]
pub mod sendto;

use anyhow::Result;

use crate::presets::{Preset, Settings};

/// Reconcile every shell surface with `presets` + `settings`. For opt-in
/// integrations that are toggled off, this actively _removes_ whatever
/// they previously installed — so flipping a toggle off in Settings
/// cleans up immediately instead of at uninstall time.
#[cfg(windows)]
pub fn sync_all(presets: &[Preset], settings: &Settings) -> Result<()> {
    // Modern menu and classic menu are mutually exclusive. If we wrote both,
    // Windows 11 would show the modern entry at the top level AND the
    // classic submenu under "Show more options" — same app, two menus. The
    // modern toggle claims the top slot; the classic registry submenu is
    // only installed when modern is off.
    if settings.modern_menu_enabled.unwrap_or(false) {
        context_menu::cleanup()?;
        modern_menu::sync(presets, settings)?;
    } else {
        modern_menu::cleanup()?;
        context_menu::sync(presets, settings)?;
    }

    if settings.sendto_enabled.unwrap_or(false) {
        sendto::sync(presets, settings)?;
    } else {
        sendto::cleanup()?;
    }

    Ok(())
}

/// macOS stub. The eventual Mac integration (NSServices Info.plist
/// generation, Finder document-type association) will be written as
/// a `macos` submodule and dispatched from here. Today this is a
/// no-op so build pipelines and first-run hooks can call sync_all
/// without branching at every call site.
#[cfg(not(windows))]
pub fn sync_all(_presets: &[Preset], _settings: &Settings) -> Result<()> {
    Ok(())
}

/// Remove everything this module installs. Called by the uninstaller's
/// `cleanup` subcommand.
#[cfg(windows)]
pub fn cleanup_all() -> Result<()> {
    let _ = context_menu::cleanup();
    let _ = sendto::cleanup();
    let _ = modern_menu::cleanup();
    // Drop the shared ExePath key only at full uninstall — individual
    // per-feature cleanups leave it alone so toggling a surface off
    // doesn't break the others.
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let _ = hkcu.delete_subkey_all(r"Software\Offspring");
    Ok(())
}

/// macOS stub for cleanup_all. Nothing to remove today.
#[cfg(not(windows))]
pub fn cleanup_all() -> Result<()> {
    Ok(())
}
