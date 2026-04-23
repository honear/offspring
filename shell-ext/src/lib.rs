//! Offspring — Windows 11 modern right-click menu handler.
//!
//! Builds a COM in-proc DLL that Windows Explorer loads when the user
//! right-clicks a file. Implements `IExplorerCommand` for the root
//! "Offspring" entry and `IEnumExplorerCommand` for the flyout that
//! lists each enabled preset plus a trailing `Custom…`.
//!
//! Registration is driven by the MSIX sparse package that ships this
//! DLL; there is no `DllRegisterServer` path needed because MSIX
//! manifests declare the COM surface declaratively.
//!
//! The DLL is deliberately lean on side effects: when Explorer asks for
//! the flyout it reads `%APPDATA%\Offspring\presets.json` and the
//! `HKCU\Software\Offspring\ExePath` value, then spawns `offspring.exe`
//! on invoke. No long-lived state, no background threads.

#![allow(non_snake_case, non_camel_case_types, clippy::missing_safety_doc)]

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;

mod child;
mod launch;
mod presets;
mod root;
mod util;

/// Class ID exposed to Windows. Must match the value in the MSIX manifest
/// (`<com:Class Id="…">`) — Explorer looks up handlers by CLSID.
///
/// If this ever needs to change, bump it in lockstep with the MSIX
/// manifest AND `Remove-AppxPackage` the old version before installing
/// the new one.
pub(crate) const ROOT_CLSID: GUID =
    GUID::from_u128(0x4A8F1E2B_6C9D_4E1F_8A2B_3C4D5E6F7A8B);

/// Single-instance `IClassFactory`. `RootCommand` is cheap to construct
/// and carries no expensive state, so we don't bother caching instances.
#[implement(IClassFactory)]
struct ClassFactory;

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Option<&IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut core::ffi::c_void,
    ) -> Result<()> {
        if punkouter.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        let cmd: IExplorerCommand = root::RootCommand::new().into();
        unsafe { cmd.query(riid, ppvobject).ok() }
    }

    fn LockServer(&self, _flock: BOOL) -> Result<()> {
        Ok(())
    }
}

#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> HRESULT {
    if rclsid.is_null() || *rclsid != ROOT_CLSID {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    let factory: IClassFactory = ClassFactory.into();
    factory.query(riid, ppv)
}

/// Explorer polls this periodically to decide if the DLL can be unloaded.
/// Returning `S_FALSE` (busy) is the safe default — Explorer will retry,
/// and the process goes away when Explorer does. Tracking a real refcount
/// across the `implement` boundary isn't worth the complexity here.
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

// MSIX-registered COM servers do NOT need DllRegisterServer /
// DllUnregisterServer — the manifest is the source of truth. We export
// stubs so legacy `regsvr32` invocations at least return cleanly instead
// of crashing Explorer in diagnostic scenarios.

#[no_mangle]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    S_OK
}

#[no_mangle]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    S_OK
}

// Use IExplorerCommand from Win32::UI::Shell here in lib.rs so the
// top-level ClassFactory impl can see it.
use windows::Win32::UI::Shell::IExplorerCommand;
