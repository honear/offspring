//! "Compare" leaf entry inside the Offspring root flyout.
//!
//! A/B compare needs at least two inputs — hidden for single-file
//! selections. Off via the Compare tool toggle.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::*;

use crate::launch;
use crate::presets::{load_settings, read_exe_path};
use crate::util::cotaskmem_wstr;

#[implement(IExplorerCommand)]
pub struct CompareCommand;

impl CompareCommand {
    pub fn new() -> Self {
        Self
    }
}

impl IExplorerCommand_Impl for CompareCommand_Impl {
    fn GetTitle(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Ok(cotaskmem_wstr("Compare"))
    }

    fn GetIcon(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        match read_exe_path() {
            Some(exe) => Ok(cotaskmem_wstr(&format!("{exe},0"))),
            None => Err(E_NOTIMPL.into()),
        }
    }

    fn GetToolTip(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(GUID::zeroed())
    }

    fn GetState(&self, items: Option<&IShellItemArray>, _okaysub: BOOL) -> Result<u32> {
        let count = unsafe { items.and_then(|arr| arr.GetCount().ok()).unwrap_or(0) };
        let enabled = load_settings().tools.compare.enabled;
        if count < 2 || !enabled {
            Ok(ECS_HIDDEN.0 as u32)
        } else {
            Ok(ECS_ENABLED.0 as u32)
        }
    }

    fn Invoke(&self, items: Option<&IShellItemArray>, _bind: Option<&IBindCtx>) -> Result<()> {
        let paths = launch::items_to_paths(items);
        launch::spawn_compare(&paths);
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}
