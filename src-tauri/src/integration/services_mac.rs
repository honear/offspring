//! macOS Services integration.
//!
//! Declares an Objective-C class (`OffspringServicesProvider`) that
//! macOS's `pbs` daemon invokes when the user picks "Offspring…" from
//! the right-click → Services submenu in Finder. The provider reads
//! the selected file URLs from the system pasteboard and emits a Tauri
//! event (`services-files`) that the frontend's picker route listens
//! for.
//!
//! The full flow:
//!   1. User right-clicks one or more files in Finder.
//!   2. Services menu shows "Offspring…" (because we declared it in
//!      Info.plist with `NSSendTypes = ["public.file-url"]`).
//!   3. User picks it. macOS launches Offspring if not running, then
//!      calls `[provider openOffspringPicker:userData:error:]` on the
//!      registered service provider — our class below.
//!   4. We read the file URLs out of NSPasteboard, convert them to
//!      filesystem paths, and emit `services-files` with the path
//!      list as payload.
//!   5. The picker window (src/routes/pick) listens for the event and
//!      shows the user the list of enabled presets + tools.
//!
//! ## Registration timing
//!
//! `register` runs from the Tauri `setup()` hook. At that point NSApp
//! has already been initialised by tao but is not yet running its main
//! loop. `[NSApp setServicesProvider:]` is fine to call here; macOS
//! buffers service events that arrive during launch and replays them
//! once the provider is registered.
//!
//! ## Lifetime
//!
//! `setServicesProvider:` does **not** retain its argument. We hold
//! the provider in a `OnceLock<Id<…>>` to keep it alive for the
//! lifetime of the process. If the provider were dropped, the next
//! service invocation would call into freed memory and crash.

use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{declare_class, msg_send, msg_send_id, mutability, ClassType, DeclaredClass};
use objc2_app_kit::{NSApp, NSPasteboard};
use objc2_foundation::{MainThreadMarker, NSArray, NSString};
use tauri::AppHandle;

/// Stored once setup() registers our provider. The service-handler
/// callback (which runs on the main thread when Cocoa invokes it)
/// reaches in here to emit Tauri events.
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// Strong reference to the provider instance. Must outlive every
/// service invocation, hence static. The Tauri runtime owns the
/// AppHandle separately; we just borrow it from APP_HANDLE.
static PROVIDER: OnceLock<Retained<OffspringServicesProvider>> = OnceLock::new();

declare_class!(
    /// The Objective-C class that NSApp calls into when the user picks
    /// our Services-menu entry. Empty struct — all state lives in the
    /// static OnceLocks above, since Cocoa doesn't give us a clean way
    /// to thread Rust state through the dispatch.
    pub struct OffspringServicesProvider;

    unsafe impl ClassType for OffspringServicesProvider {
        type Super = NSObject;
        type Mutability = mutability::InteriorMutable;
        const NAME: &'static str = "OffspringServicesProvider";
    }

    impl DeclaredClass for OffspringServicesProvider {}

    unsafe impl OffspringServicesProvider {
        /// Service handler. Selector name MUST match the `NSMessage`
        /// value we declare in Info.plist (`openOffspringPicker`).
        ///
        /// Cocoa calling convention for NSServices methods:
        ///     - (void)<message>:(NSPasteboard *)pboard
        ///                userData:(NSString *)userData
        ///                error:(NSString **)error;
        ///
        /// `userData` is whatever string we put in Info.plist's
        /// `NSUserData` (none for us — left null). `error` is an
        /// out-param: we'd write an NSString into it on failure, but
        /// the Services menu currently has no way to surface that to
        /// the user, so we treat it as informational and just log.
        #[method(openOffspringPicker:userData:error:)]
        fn open_offspring_picker(
            &self,
            pboard: &NSPasteboard,
            _user_data: *const NSString,
            _error: *mut *mut NSString,
        ) {
            let paths = read_file_paths(pboard);
            open_picker_for(paths);
        }
    }
);

/// Pull the file paths out of the pasteboard.
///
/// Uses the legacy NSFilenamesPboardType path: `propertyListForType:`
/// with that type returns an NSArray of NSString filenames directly,
/// which is much simpler to handle than the modern NSURL-based API
/// (no class-array dance, no NSURL → path conversion). It's officially
/// deprecated in 10.14 but still works on every macOS through current
/// versions; we'll switch to the modern API once we have a Mac in hand
/// for testing the bindings.
///
/// Returns the paths as UTF-8 strings. Silently returns an empty vec
/// if the pasteboard doesn't contain anything we can read — the
/// picker window then renders "No files selected" rather than crashing.
fn read_file_paths(pboard: &NSPasteboard) -> Vec<String> {
    unsafe {
        let type_str = NSString::from_str("NSFilenamesPboardType");
        let plist: Option<Retained<NSArray<NSString>>> =
            msg_send_id![pboard, propertyListForType: &*type_str];

        let Some(plist) = plist else {
            return Vec::new();
        };

        let mut result = Vec::with_capacity(plist.len());
        for i in 0..plist.len() {
            let s = plist.objectAtIndex(i);
            result.push(s.to_string());
        }
        result
    }
}

/// Open the picker window with the selected files. Service handler
/// callback runs on the main thread per Cocoa convention, which is
/// the same thread Tauri runs its UI work on, so it's safe to call
/// WebviewWindowBuilder directly here.
fn open_picker_for(paths: Vec<String>) {
    let Some(handle) = APP_HANDLE.get() else {
        eprintln!(
            "services_mac: invoked before APP_HANDLE was registered ({} paths dropped)",
            paths.len()
        );
        return;
    };
    if let Err(e) = crate::commands::open_pick_window(handle, paths) {
        eprintln!("services_mac: open_pick_window failed: {e}");
    }
}

/// Register the services provider with NSApp. Call once from the Tauri
/// setup() hook. Safe to call after Tauri's runtime is up; idempotent
/// (subsequent calls are no-ops because the OnceLocks are already set).
pub fn register(app: &AppHandle) {
    if APP_HANDLE.set(app.clone()).is_err() {
        return; // already registered
    }

    let mtm = match MainThreadMarker::new() {
        Some(mtm) => mtm,
        None => {
            // setup() runs on the main thread in practice, but the
            // marker check is the documented way to prove it.
            eprintln!("services_mac: register() called off the main thread; skipping");
            return;
        }
    };

    unsafe {
        // Instantiate the provider — Allocated::new + init is the
        // standard objc2 idiom for "new instance of a custom class".
        let allocated = OffspringServicesProvider::alloc();
        let provider: Retained<OffspringServicesProvider> =
            msg_send_id![allocated, init];

        // Tell NSApp to dispatch service messages to us. Cast to
        // AnyObject because setServicesProvider: takes id, not our
        // specific class type.
        let provider_obj: &AnyObject = &**provider;
        let app = NSApp(mtm);
        let app_obj: &AnyObject = &*app;
        let _: () = msg_send![app_obj, setServicesProvider: provider_obj];

        // Keep the provider alive for the rest of the process —
        // setServicesProvider: doesn't retain its argument, and
        // letting the Retained<…> drop here would crash on the next
        // service invocation.
        let _ = PROVIDER.set(provider);
    }
}
