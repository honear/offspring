//! Top-level "Offspring" entry + flyout enumerator.
//!
//! `RootCommand` is what Explorer constructs via our `IClassFactory`. It
//! reports `ECF_HASSUBCOMMANDS` so Explorer draws a flyout arrow, then
//! calls `EnumSubCommands` to walk the list. We hand back a fresh
//! `SubEnum` each time so the cursor state is per-flyout-expansion.

use std::cell::Cell;

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::*;

use crate::child::ChildCommand;
use crate::compare::CompareCommand;
use crate::grayscale::GrayscaleCommand;
use crate::merge::MergeCommand;
use crate::overlay::OverlayCommand;
use crate::presets::{load_presets, read_exe_path};
use crate::settings::SettingsCommand;
use crate::util::cotaskmem_wstr;

#[implement(IExplorerCommand)]
pub struct RootCommand;

impl RootCommand {
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
        // Tools live at the end so they sit below the preset list. Each
        // tool's GetState hides it when its toggle is off, so users who
        // don't want them never see them. The Settings entry anchors
        // the bottom and is always visible.
        out.push(MergeCommand::new().into());
        out.push(GrayscaleCommand::new().into());
        out.push(CompareCommand::new().into());
        out.push(OverlayCommand::new().into());
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
        let children = RootCommand::build_children();
        let en: IEnumExplorerCommand = SubEnum {
            items: children,
            cursor: Cell::new(0),
        }
        .into();
        Ok(en)
    }
}

#[implement(IEnumExplorerCommand)]
struct SubEnum {
    items: Vec<IExplorerCommand>,
    cursor: Cell<usize>,
}

impl IEnumExplorerCommand_Impl for SubEnum_Impl {
    fn Next(
        &self,
        celt: u32,
        rgelt: *mut Option<IExplorerCommand>,
        pceltfetched: *mut u32,
    ) -> HRESULT {
        let start = self.cursor.get();
        let end = (start + celt as usize).min(self.items.len());
        let count = end - start;
        unsafe {
            for i in 0..count {
                *rgelt.add(i) = Some(self.items[start + i].clone());
            }
            if !pceltfetched.is_null() {
                *pceltfetched = count as u32;
            }
        }
        self.cursor.set(end);
        if count == celt as usize {
            S_OK
        } else {
            S_FALSE
        }
    }

    fn Skip(&self, celt: u32) -> Result<()> {
        let next = self.cursor.get().saturating_add(celt as usize);
        if next > self.items.len() {
            self.cursor.set(self.items.len());
            Err(S_FALSE.into())
        } else {
            self.cursor.set(next);
            Ok(())
        }
    }

    fn Reset(&self) -> Result<()> {
        self.cursor.set(0);
        Ok(())
    }

    fn Clone(&self) -> Result<IEnumExplorerCommand> {
        let clone = SubEnum {
            items: self.items.clone(),
            cursor: Cell::new(self.cursor.get()),
        };
        Ok(clone.into())
    }
}

// `#[implement]` auto-generates the `From<Self> for <iface>` conversions,
// so `RootCommand.into()` and `SubEnum.into()` just work.
