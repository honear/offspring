//! "Greyscale" leaf entry inside the Offspring root flyout.
//!
//! Sibling of the Merge entry. Works on any count of selected files
//! (≥1) and hides itself when the Greyscale tool is toggled off in
//! settings.
//!
//! Leaf command — clicking it spawns `offspring.exe grayscale <files>`
//! directly. Each input is encoded independently, inheriting format +
//! dimensions + fps from its own source. For quality-tuned greyscale
//! conversions, users can set a per-preset `grayscale: true` flag on a
//! saved preset instead.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::*;

use crate::launch;
use crate::presets::{load_settings, read_exe_path};
use crate::util::cotaskmem_wstr;

#[implement(IExplorerCommand)]
pub struct GrayscaleCommand;

impl GrayscaleCommand {
    pub fn new() -> Self {
        Self
    }
}

impl IExplorerCommand_Impl for GrayscaleCommand_Impl {
    fn GetTitle(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Ok(cotaskmem_wstr("Greyscale"))
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

    /// Hide when the tool is disabled. Unlike Merge we show for any
    /// selection count ≥1 because greyscaling a single file is a
    /// perfectly reasonable operation.
    fn GetState(&self, items: Option<&IShellItemArray>, _okaysub: BOOL) -> Result<u32> {
        let count = unsafe { items.and_then(|arr| arr.GetCount().ok()).unwrap_or(0) };
        let enabled = load_settings().tools.grayscale.enabled;
        if count < 1 || !enabled {
            Ok(ECS_HIDDEN.0 as u32)
        } else {
            Ok(ECS_ENABLED.0 as u32)
        }
    }

    fn Invoke(&self, items: Option<&IShellItemArray>, _bind: Option<&IBindCtx>) -> Result<()> {
        let paths = launch::items_to_paths(items);
        launch::spawn_grayscale(&paths);
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}
