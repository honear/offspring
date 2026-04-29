//! "Trim..." leaf entry inside the Offspring root flyout.
//!
//! Mirrors the Custom entry's role: a single click here doesn't run an
//! encode — it spawns `offspring.exe trim <files>`, which opens the
//! Trim mini-dialog. The user enters how many frames to strip from the
//! start and end, hits the Trim button, and the dialog navigates its
//! own webview to /progress/ (avoiding a second WebView2 window).
//!
//! Hides itself when the Trim tool is toggled off in settings. Unlike
//! Merge/Compare, Trim works on a single file too — a 1-file
//! selection is the most common use-case for trimming a tail off a
//! GIF — so we don't gate visibility on file count.

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::*;

use crate::launch;
use crate::presets::{load_settings, read_exe_path};
use crate::util::cotaskmem_wstr;

#[implement(IExplorerCommand)]
pub struct TrimCommand;

impl TrimCommand {
    pub fn new() -> Self {
        Self
    }
}

impl IExplorerCommand_Impl for TrimCommand_Impl {
    fn GetTitle(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Ok(cotaskmem_wstr("Trim..."))
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
        let enabled = load_settings().tools.trim.enabled;
        if enabled {
            Ok(ECS_ENABLED.0 as u32)
        } else {
            Ok(ECS_HIDDEN.0 as u32)
        }
    }

    fn Invoke(&self, items: Option<&IShellItemArray>, _bind: Option<&IBindCtx>) -> Result<()> {
        let paths = launch::items_to_paths(items);
        launch::spawn_trim(&paths);
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}
