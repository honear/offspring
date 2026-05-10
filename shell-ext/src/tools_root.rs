//! "Offspring Tools" — top-level Win11 modern-menu entry registered
//! by the `OffspringShellExt.Tools.msix` package when the user has
//! the split-layout setting on.
//!
//! Sibling of `PresetsRootCommand`. Lists the 8 tool entries plus a
//! trailing Settings command (mirrors classic-menu's `zz_settings`
//! anchor at the bottom of the Tools flyout). Children sit at depth 1
//! from this command's perspective, the same depth that works
//! reliably in `RootCommand`.
//!
//! Visibility is gated at the MSIX-package level: when the user is in
//! unified mode this package isn't registered, so the entry doesn't
//! exist at all. `GetState` therefore returns `ECS_ENABLED`
//! unconditionally; each leaf still does its own per-tool `enabled`
//! check.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::*;

use crate::compare::CompareCommand;
use crate::grayscale::GrayscaleCommand;
use crate::invert::InvertCommand;
use crate::make_square::MakeSquareCommand;
use crate::merge::MergeCommand;
use crate::modify::ModifyCommand;
use crate::overlay::OverlayCommand;
use crate::presets::read_exe_path;
use crate::settings::SettingsCommand;
use crate::sub_enum::SubEnum;
use crate::trim::TrimCommand;
use crate::util::cotaskmem_wstr;

#[implement(IExplorerCommand)]
pub struct ToolsRootCommand;

impl ToolsRootCommand {
    pub fn new() -> Self {
        Self
    }

    fn build_children() -> Vec<IExplorerCommand> {
        vec![
            GrayscaleCommand::new().into(),
            OverlayCommand::new().into(),
            MergeCommand::new().into(),
            CompareCommand::new().into(),
            TrimCommand::new().into(),
            InvertCommand::new().into(),
            MakeSquareCommand::new().into(),
            ModifyCommand::new().into(),
            SettingsCommand::new().into(),
        ]
    }
}

impl IExplorerCommand_Impl for ToolsRootCommand_Impl {
    fn GetTitle(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Ok(cotaskmem_wstr("Offspring Tools"))
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
        Ok(crate::TOOLS_ROOT_CLSID)
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
        Ok(SubEnum::into_iface(ToolsRootCommand::build_children()))
    }
}
