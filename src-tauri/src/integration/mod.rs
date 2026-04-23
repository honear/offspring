//! Shell integrations — the places where Offspring shows up outside the app
//! window.
//!
//! Three back-ends live here:
//!
//! * `context_menu` — writes `HKCU\Software\Classes\*\shell\Offspring` with
//!   an `ExtendedSubCommandsKey` pointing at per-preset verbs. This is the
//!   classic ("Show more options") right-click menu. Always on; it's the
//!   default surface that replaces SendTo for Windows 11 users.
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

pub mod context_menu;
pub mod modern_menu;
pub mod sendto;

use anyhow::Result;

use crate::presets::{Preset, Settings};

/// Reconcile every shell surface with `presets` + `settings`. For opt-in
/// integrations that are toggled off, this actively _removes_ whatever
/// they previously installed — so flipping a toggle off in Settings
/// cleans up immediately instead of at uninstall time.
pub fn sync_all(presets: &[Preset], settings: &Settings) -> Result<()> {
    context_menu::sync(presets)?;

    if settings.sendto_enabled.unwrap_or(false) {
        sendto::sync(presets)?;
    } else {
        sendto::cleanup()?;
    }

    if settings.modern_menu_enabled.unwrap_or(false) {
        modern_menu::sync(presets)?;
    } else {
        modern_menu::cleanup()?;
    }

    Ok(())
}

/// Remove everything this module installs. Called by the uninstaller's
/// `cleanup` subcommand.
pub fn cleanup_all() -> Result<()> {
    let _ = context_menu::cleanup();
    let _ = sendto::cleanup();
    let _ = modern_menu::cleanup();
    Ok(())
}
