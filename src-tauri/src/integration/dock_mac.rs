//! macOS Dock-icon visibility control via `NSApplication`'s
//! activation policy.
//!
//! Background: we set `LSUIElement = true` in Info.plist so Offspring
//! starts as an "agent" / "accessory" application — no Dock icon, no
//! application-switcher entry, lives in the system menu bar instead.
//! That's the right default for a tool that's primarily invoked via
//! Finder right-click + a menu bar tray.
//!
//! But when the user explicitly opens the main settings window
//! (presets editor, etc.) we want a normal Dock presence for the
//! duration of that window — otherwise it feels weirdly headless and
//! they have no obvious way to bring it to the foreground after
//! switching apps. `NSApplication.setActivationPolicy()` lets us
//! flip between Regular (in Dock) and Accessory (no Dock) at runtime,
//! overriding the static LSUIElement setting on the fly.
//!
//! Threading: every NSApp call must happen on the main thread. Tauri
//! routes window lifecycle events to its own thread pool, so callers
//! should dispatch through `app.run_on_main_thread(|| set_*())`. The
//! free functions below assume the caller has already done that and
//! grab a `MainThreadMarker::new_unchecked()` accordingly.

use objc2_app_kit::{NSApp, NSApplicationActivationPolicy};
use objc2_foundation::MainThreadMarker;

/// Switch to `Regular` and explicitly bring the app to the
/// foreground. Call from the main settings window's open path so the
/// Dock icon appears alongside the window and the user can ⌘-Tab to
/// it like any other app.
pub fn set_regular() {
    // SAFETY: caller must dispatch via `app.run_on_main_thread`.
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let app = NSApp(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    // activateIgnoringOtherApps:YES is the legacy idiom but still the
    // simplest reliable path on Big Sur+. The newer activate() (no
    // args) only activates if we're already in the active-app list,
    // which we may not be when transitioning out of accessory mode.
    app.activateIgnoringOtherApps(true);
}

/// Switch to `Accessory` — Dock icon disappears, app stays running
/// for menu bar / Services. Call from the main settings window's
/// close path. Safe to call when already accessory.
pub fn set_accessory() {
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let app = NSApp(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

/// Bring the app to the foreground without changing activation
/// policy. Use this when opening transient windows from background
/// flows (Services pick → progress, menu bar tray click → popover):
/// the app needs to be foreground for the new window's setFocus to
/// land, but we don't want to flip to Regular and pollute the Dock.
pub fn activate_without_dock() {
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    let app = NSApp(mtm);
    app.activateIgnoringOtherApps(true);
}
