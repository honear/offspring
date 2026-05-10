//! "Merge" leaf entry inside the Offspring root flyout.
//!
//! Sibling of the preset rows. Hides itself via `ECS_HIDDEN` when fewer
//! than two items are selected or when the Merge tool is toggled off in
//! settings — single-file right-clicks stay uncluttered and users who
//! haven't enabled the tool never see it.
//!
//! Unlike the preset children this is a LEAF: no sub-flyout, no preset
//! picker. Clicking it spawns `offspring.exe merge <files>` directly,
//! and the Rust side derives output format + settings from the first
//! selected file. The tradeoff — one verb with no per-preset choice —
//! is what keeps the menu from doubling in size.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::*;

use crate::launch;
use crate::presets::{load_settings, read_exe_path};
use crate::util::cotaskmem_wstr;

#[implement(IExplorerCommand)]
pub struct MergeCommand;

impl MergeCommand {
    pub fn new() -> Self {
        Self
    }
}

impl IExplorerCommand_Impl for MergeCommand_Impl {
    fn GetTitle(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Ok(cotaskmem_wstr("Merge"))
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

    /// Hide unless at least two files are selected AND the user has
    /// turned the Merge tool on. `ECS_HIDDEN` removes the entry from
    /// the flyout entirely — contrast with `ECS_DISABLED` which would
    /// leave a greyed-out row.
    ///
    /// `items: None` is treated as permissive — see `grayscale.rs`
    /// for the Win11 nested-sub-flyout quirk that necessitates this.
    /// Invoke validates the real count and emits a clear error if a
    /// user clicks Merge with only one file selected.
    fn GetState(&self, items: Option<&IShellItemArray>, _okaysub: BOOL) -> Result<u32> {
        let enabled = load_settings().tools.merge.enabled;
        if !enabled {
            return Ok(ECS_HIDDEN.0 as u32);
        }
        let count_ok = match items {
            Some(arr) => unsafe { arr.GetCount().ok().unwrap_or(2) >= 2 },
            None => true,
        };
        if count_ok { Ok(ECS_ENABLED.0 as u32) } else { Ok(ECS_HIDDEN.0 as u32) }
    }

    fn Invoke(&self, items: Option<&IShellItemArray>, _bind: Option<&IBindCtx>) -> Result<()> {
        let paths = launch::items_to_paths(items);
        launch::spawn_merge(&paths);
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        // Leaf entry — no `ECF_HASSUBCOMMANDS`. Explorer invokes us
        // directly when the user picks the row.
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}
