//! "Modify..." leaf entry inside the Offspring root flyout.
//!
//! Mirror of `TrimCommand` — opens the Crop mini dialog rather than
//! kicking off an encode directly. Visible whenever ≥1 file is
//! selected and the tool is enabled in settings; the dialog itself
//! shows the preview and gathers the crop rectangle from the user.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::*;

use crate::launch;
use crate::presets::{load_settings, read_exe_path};
use crate::util::cotaskmem_wstr;

#[implement(IExplorerCommand)]
pub struct ModifyCommand;

impl ModifyCommand {
    pub fn new() -> Self {
        Self
    }
}

impl IExplorerCommand_Impl for ModifyCommand_Impl {
    fn GetTitle(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Ok(cotaskmem_wstr("Modify..."))
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

    /// See `grayscale.rs` for the `items: None` permissive handling —
    /// nested sub-flyouts on Win11 don't always propagate selection.
    fn GetState(&self, items: Option<&IShellItemArray>, _okaysub: BOOL) -> Result<u32> {
        let enabled = load_settings().tools.modify.enabled;
        if !enabled {
            return Ok(ECS_HIDDEN.0 as u32);
        }
        let count_ok = match items {
            Some(arr) => unsafe { arr.GetCount().ok().unwrap_or(1) >= 1 },
            None => true,
        };
        if count_ok { Ok(ECS_ENABLED.0 as u32) } else { Ok(ECS_HIDDEN.0 as u32) }
    }

    fn Invoke(&self, items: Option<&IShellItemArray>, _bind: Option<&IBindCtx>) -> Result<()> {
        let paths = launch::items_to_paths(items);
        launch::spawn_modify(&paths);
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}
