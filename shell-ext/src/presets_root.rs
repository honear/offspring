//! "Offspring Presets" — top-level Win11 modern-menu entry registered
//! by the `OffspringShellExt.Presets.msix` package when the user has
//! the split-layout setting on.
//!
//! Sibling of `ToolsRootCommand`. Lists every enabled preset plus a
//! trailing `Custom…`. Children sit at depth 1 from this command's
//! perspective, the same depth that works reliably in `RootCommand`,
//! so we stay clear of the depth-2 empty-flyout bug.
//!
//! Visibility is gated at the MSIX-package level: when the user is in
//! unified mode this package isn't registered, so the entry doesn't
//! exist at all. `GetState` therefore returns `ECS_ENABLED`
//! unconditionally.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::*;

use crate::child::ChildCommand;
use crate::presets::{load_presets, read_exe_path};
use crate::sub_enum::SubEnum;
use crate::util::cotaskmem_wstr;

#[implement(IExplorerCommand)]
pub struct PresetsRootCommand;

impl PresetsRootCommand {
    pub fn new() -> Self {
        Self
    }

    fn build_children() -> Vec<IExplorerCommand> {
        let mut out: Vec<IExplorerCommand> = load_presets()
            .into_iter()
            .filter(|p| p.enabled)
            .map(|p| ChildCommand::new_preset(p).into())
            .collect();
        out.push(ChildCommand::new_custom().into());
        out
    }
}

impl IExplorerCommand_Impl for PresetsRootCommand_Impl {
    fn GetTitle(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Ok(cotaskmem_wstr("Offspring Presets"))
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
        Ok(crate::PRESETS_ROOT_CLSID)
    }

    fn GetState(&self, _items: Option<&IShellItemArray>, _okaysub: BOOL) -> Result<u32> {
        Ok(ECS_ENABLED.0 as u32)
    }

    fn Invoke(&self, _items: Option<&IShellItemArray>, _bind: Option<&IBindCtx>) -> Result<()> {
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_HASSUBCOMMANDS.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Ok(SubEnum::into_iface(PresetsRootCommand::build_children()))
    }
}
