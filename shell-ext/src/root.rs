//! Top-level "Offspring" entry + flyout enumerator.
//!
//! `RootCommand` is what Explorer constructs via our `IClassFactory`. It
//! reports `ECF_HASSUBCOMMANDS` so Explorer draws a flyout arrow, then
//! calls `EnumSubCommands` to walk the list. We hand back a fresh
//! `SubEnum` each time so the cursor state is per-flyout-expansion.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::*;

use crate::child::ChildCommand;
use crate::compare::CompareCommand;
use crate::grayscale::GrayscaleCommand;
use crate::invert::InvertCommand;
use crate::make_square::MakeSquareCommand;
use crate::merge::MergeCommand;
use crate::modify::ModifyCommand;
use crate::overlay::OverlayCommand;
use crate::presets::{load_presets, read_exe_path};
use crate::settings::SettingsCommand;
use crate::sub_enum::SubEnum;
use crate::trim::TrimCommand;
use crate::util::cotaskmem_wstr;

#[implement(IExplorerCommand)]
pub struct RootCommand;

impl RootCommand {
    pub fn new() -> Self {
        Self
    }

    /// Flat list at level 1: enabled presets, Custom…, the 8 tool
    /// leaves, Settings. Per-tool `enabled` is enforced by each leaf's
    /// own `GetState` returning `ECS_HIDDEN`, so this builder doesn't
    /// pre-filter tool entries based on settings.
    ///
    /// The split-layout feature is delivered by registering separate
    /// `PresetsRootCommand` / `ToolsRootCommand` MSIX packages
    /// instead of this one — `RootCommand` itself is always the
    /// unified "Offspring" entry.
    fn build_children() -> Vec<IExplorerCommand> {
        let mut out: Vec<IExplorerCommand> = load_presets()
            .into_iter()
            .filter(|p| p.enabled)
            .map(|p| ChildCommand::new_preset(p).into())
            .collect();
        out.push(ChildCommand::new_custom().into());
        out.push(GrayscaleCommand::new().into());
        out.push(OverlayCommand::new().into());
        out.push(MergeCommand::new().into());
        out.push(CompareCommand::new().into());
        out.push(TrimCommand::new().into());
        out.push(InvertCommand::new().into());
        out.push(MakeSquareCommand::new().into());
        out.push(ModifyCommand::new().into());
        out.push(SettingsCommand::new().into());
        out
    }
}

impl IExplorerCommand_Impl for RootCommand_Impl {
    fn GetTitle(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Ok(cotaskmem_wstr("Offspring"))
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
        Ok(crate::ROOT_CLSID)
    }

    /// Always show the single Offspring entry — the split-layout
    /// setting reshapes the flyout contents (see `build_children`),
    /// not the entry itself.
    fn GetState(&self, _items: Option<&IShellItemArray>, _okaysub: BOOL) -> Result<u32> {
        Ok(ECS_ENABLED.0 as u32)
    }

    fn Invoke(
        &self,
        _items: Option<&IShellItemArray>,
        _bind: Option<&IBindCtx>,
    ) -> Result<()> {
        // Root isn't directly invocable — picking it just opens the
        // flyout. Return S_OK regardless.
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_HASSUBCOMMANDS.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Ok(SubEnum::into_iface(RootCommand::build_children()))
    }
}

// `#[implement]` auto-generates the `From<Self> for <iface>` conversion,
// so `RootCommand.into()` just works. `SubEnum` itself lives in
// `sub_enum.rs` and is reused by `tools_root.rs`.
