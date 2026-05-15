//! macOS Services integration.
//!
//! Declares an Objective-C class (`OffspringServicesProvider`) that
//! macOS's `pbs` daemon invokes when the user picks "Offspring…" from
//! the right-click → Services submenu in Finder. The provider reads
//! the selected file URLs from the system pasteboard and opens the
//! picker window with that file list pre-populated.
//!
//! The full flow:
//!   1. User right-clicks one or more files in Finder.
//!   2. Services menu shows "Offspring…" (because we declared it in
//!      Info.plist with `NSSendTypes = ["public.file-url"]`).
//!   3. User picks it. macOS launches Offspring if not running, then
//!      calls `[provider openOffspringPicker:userData:error:]` on the
//!      registered service provider — our class below.
//!   4. We read the file paths off NSPasteboard and open the picker
//!      window via the existing `open_pick_window` command, which
//!      stages files in app state for the route to consume.
//!
//! ## Registration timing
//!
//! `register` runs from the Tauri `setup()` hook, on the main thread.
//! By that point NSApp is initialised but the runloop hasn't started.
//! `setServicesProvider:` is safe to call at this stage; macOS buffers
//! any in-flight service events until the provider is in place.
//!
//! ## Lifetime
//!
//! `setServicesProvider:` does **not** retain its argument. We use
//! `mem::forget` on the `Retained<…>` to leak the provider so it
//! lives for the lifetime of the process. Storing it in a static
//! would require Send+Sync impls that objc2 doesn't supply, so the
//! leak is the cleanest option.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{declare_class, msg_send, msg_send_id, mutability, ClassType, DeclaredClass};
use objc2_app_kit::{NSApp, NSPasteboard};
use objc2_foundation::{MainThreadMarker, NSArray, NSString};
use std::sync::OnceLock;
use tauri::AppHandle;

/// Stored once setup() registers our provider. The service-handler
/// callback (which runs on the main thread when Cocoa invokes it)
/// reaches in here to find the Tauri AppHandle for opening windows.
/// AppHandle is Send+Sync so OnceLock<AppHandle> works without unsafe
/// impls.
static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

declare_class!(
    /// The Objective-C class that NSApp calls into when the user picks
    /// our Services-menu entry. Empty struct — state lives in the
    /// APP_HANDLE static, since Cocoa doesn't give us a clean way to
    /// thread Rust state through the dispatch.
    pub struct OffspringServicesProvider;

    unsafe impl ClassType for OffspringServicesProvider {
        type Super = NSObject;
        type Mutability = mutability::InteriorMutable;
        const NAME: &'static str = "OffspringServicesProvider";
    }

    impl DeclaredClass for OffspringServicesProvider {}

    unsafe impl OffspringServicesProvider {
        /// Service handler. Selector name MUST match the `NSMessage`
        /// value in Info.plist (`openOffspringPicker`).
        ///
        /// Cocoa calling convention for NSServices methods:
        ///     - (void)<message>:(NSPasteboard *)pboard
        ///                userData:(NSString *)userData
        ///                error:(NSString **)error;
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
/// Uses the legacy `NSFilenamesPboardType` path: `propertyListForType:`
/// returns an NSArray<NSString> of filenames, which is much simpler
/// than the modern NSURL-based API (no class-array dance to construct
/// the type filter, no NSURL → path conversion). The type is
/// deprecated in 10.14 but still works on every current macOS; we'll
/// switch to the modern API once we have a Mac in hand for testing
/// the bindings.
fn read_file_paths(pboard: &NSPasteboard) -> Vec<String> {
    unsafe {
        let type_str = NSString::from_str("NSFilenamesPboardType");
        let plist: Option<Retained<NSArray<NSString>>> =
            msg_send_id![pboard, propertyListForType: &*type_str];

        let Some(plist) = plist else {
            return Vec::new();
        };

        // NSArray<NSString> in objc2-foundation 0.2 doesn't expose
        // .iter() directly on Retained<…>. Use the underlying
        // count + objectAtIndex: ObjC selectors via msg_send.
        let count: usize = msg_send![&*plist, count];
        let mut result = Vec::with_capacity(count);
        for i in 0..count {
            let s: Retained<NSString> = msg_send_id![&*plist, objectAtIndex: i];
            result.push(s.to_string());
        }
        result
    }
}

/// Open the picker window with the selected files. Service handler
/// callback runs on the main thread per Cocoa convention, which is
/// the same thread Tauri runs its UI work on — calling
/// WebviewWindowBuilder directly here is safe.
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

/// Register the services provider with NSApp. Call once from the
/// Tauri setup() hook. Safe to call multiple times — APP_HANDLE's
/// OnceLock guards against double-registration.
pub fn register(app: &AppHandle) {
    if APP_HANDLE.set(app.clone()).is_err() {
        return; // already registered
    }

    // SAFETY: Tauri's setup() hook runs on the main thread. There's
    // no safe MainThreadMarker constructor in objc2-foundation 0.2;
    // new_unchecked is the documented escape hatch when the caller
    // can guarantee main-thread context.
    let mtm = unsafe { MainThreadMarker::new_unchecked() };

    unsafe {
        // Instantiate the provider. Allocated::new + init is the
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

        // Leak the Retained<…> so the provider survives for the
        // lifetime of the process. setServicesProvider: doesn't
        // retain, and we can't put Retained<…> in a static (no
        // Send+Sync impl). mem::forget skips the Drop, which would
        // have released our retain — so the count stays at +1
        // forever, exactly what we want.
        std::mem::forget(provider);
    }
}
