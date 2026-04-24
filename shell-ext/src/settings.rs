//! "Offspring settings" leaf entry inside the Offspring root flyout.
//!
//! Always enabled (no per-tool toggle). Sits at the bottom of the
//! flyout so users can reach the app's configuration surface without
//! launching from Start. The command is independent of the selection —
//! it just launches the UI.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::*;

use crate::launch;
use crate::presets::read_exe_path;
use crate::util::cotaskmem_wstr;

#[implement(IExplorerCommand)]
pub struct SettingsCommand;

impl SettingsCommand {
    pub fn new() -> Self {
        Self
    }
}

impl IExplorerCommand_Impl for SettingsCommand_Impl {
    fn GetTitle(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Ok(cotaskmem_wstr("Offspring settings"))
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

    fn GetState(&self, _items: Option<&IShellItemArray>, _okaysub: BOOL) -> Result<u32> {
        Ok(ECS_ENABLED.0 as u32)
    }

    fn Invoke(&self, _items: Option<&IShellItemArray>, _bind: Option<&IBindCtx>) -> Result<()> {
        launch::spawn_settings();
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}
