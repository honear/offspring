//! Per-flyout-entry `IExplorerCommand`. Each one renders as one row in
//! the "Offspring ►" submenu. `Invoke` launches `offspring.exe` with
//! the appropriate CLI verb (either `preset --id <id>` or `custom`).

use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::IBindCtx;
use windows::Win32::UI::Shell::*;

use crate::launch;
use crate::presets::{read_exe_path, Preset};
use crate::util::cotaskmem_wstr;

/// Tag that picks which CLI verb `Invoke` spawns.
enum Action {
    Preset { id: String, icon: Option<String> },
    Custom,
}

#[implement(IExplorerCommand)]
pub struct ChildCommand {
    title: String,
    action: Action,
}

impl ChildCommand {
    pub fn new_preset(preset: Preset) -> Self {
        Self {
            title: preset.name,
            action: Action::Preset {
                id: preset.id,
                icon: preset.icon,
            },
        }
    }

    pub fn new_custom() -> Self {
        Self {
            title: "Custom...".to_string(),
            action: Action::Custom,
        }
    }
}

impl IExplorerCommand_Impl for ChildCommand_Impl {
    fn GetTitle(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Ok(cotaskmem_wstr(&self.title))
    }

    fn GetIcon(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        // Prefer the preset's custom icon if one was configured;
        // otherwise fall back to offspring.exe's default icon.
        let icon = match &self.action {
            Action::Preset { icon: Some(i), .. } if !i.is_empty() => Some(i.clone()),
            _ => None,
        };
        if let Some(i) = icon {
            return Ok(cotaskmem_wstr(&i));
        }
        match read_exe_path() {
            Some(exe) => Ok(cotaskmem_wstr(&format!("{exe},0"))),
            None => Err(E_NOTIMPL.into()),
        }
    }

    fn GetToolTip(&self, _items: Option<&IShellItemArray>) -> Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn GetCanonicalName(&self) -> Result<GUID> {
        // A stable GUID per child isn't required for basic operation;
        // returning GUID_NULL tells Explorer "no canonical identity",
        // which is fine for dynamic flyouts like ours.
        Ok(GUID::zeroed())
    }

    fn GetState(&self, _items: Option<&IShellItemArray>, _okaysub: BOOL) -> Result<u32> {
        Ok(ECS_ENABLED.0 as u32)
    }

    fn Invoke(
        &self,
        items: Option<&IShellItemArray>,
        _bind: Option<&IBindCtx>,
    ) -> Result<()> {
        let paths = launch::items_to_paths(items);
        match &self.action {
            Action::Preset { id, .. } => launch::spawn_preset(id, &paths),
            Action::Custom => launch::spawn_custom(&paths),
        }
        Ok(())
    }

    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }

    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(E_NOTIMPL.into())
    }
}

// NOTE: `#[implement(IExplorerCommand)]` auto-generates `From<ChildCommand>
// for IExplorerCommand`, so we just call `.into()` at call sites.
