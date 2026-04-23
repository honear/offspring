//! Legacy ("Show more options") right-click submenu via the user-hive
//! registry.
//!
//! Writes:
//!
//! ```text
//! HKCU\Software\Classes\*\shell\Offspring
//!   MUIVerb                = "Offspring"
//!   Icon                   = "<exe>,0"
//!   ExtendedSubCommandsKey = "Offspring.SubCommands"
//!
//! HKCU\Software\Classes\Offspring.SubCommands\shell\01_<preset-id>
//!   MUIVerb = "<preset.name>"
//!   Icon    = "<preset.icon>" (if set)
//!   \command
//!     (Default) = "\"<exe>\" preset --id <preset.id> \"%1\""
//!
//! ...<one per enabled preset, zero-padded so menu order is stable>...
//!
//! HKCU\Software\Classes\Offspring.SubCommands\shell\99_custom
//!   MUIVerb = "Custom..."
//!   \command
//!     (Default) = "\"<exe>\" custom \"%1\""
//! ```
//!
//! HKCU means no elevation, per-user install. `*` matches every file type —
//! same surface as the SendTo shortcuts we used to rely on exclusively.
//!
//! This is NOT the Windows 11 modern (top-level) right-click menu. That
//! requires a COM shell extension packaged in an MSIX, which is a separate
//! opt-in behind its own Settings toggle.

use anyhow::{Context, Result};
use winreg::enums::*;
use winreg::RegKey;

use crate::presets::Preset;

/// Name of the top-level verb under `HKCU\Software\Classes\*\shell\`. Must
/// match `SUB_KEY_NAME` via the `ExtendedSubCommandsKey` value below.
const ROOT_VERB: &str = "Offspring";

/// Free-standing key that holds the per-preset verbs. Must be unique enough
/// to not collide with anything else under `HKCU\Software\Classes\`.
const SUB_KEY_NAME: &str = "Offspring.SubCommands";

fn current_exe_string() -> Result<String> {
    let exe = std::env::current_exe().context("getting current exe path")?;
    Ok(exe.to_string_lossy().into_owned())
}

/// Write the root verb + one child per enabled preset + a trailing Custom…
pub fn sync(presets: &[Preset]) -> Result<()> {
    // Start clean — the easiest way to handle renames/removed presets is to
    // nuke the whole subtree and rebuild. These keys are entirely ours.
    let _ = cleanup();

    let exe = current_exe_string()?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // Publish the install path so the MSIX shell-extension DLL (Phase 3+)
    // can resolve offspring.exe without hard-coding a program-files path.
    // The DLL runs inside Explorer.exe, has no relation to our install
    // directory, and the MSIX package location isn't ours either — this
    // registry value is the contract between the two.
    let (root_key, _) = hkcu.create_subkey(r"Software\Offspring")?;
    root_key.set_value("ExePath", &exe)?;

    // Top-level verb that Explorer will show as "Offspring" with a ►.
    let root_path = format!(r"Software\Classes\*\shell\{ROOT_VERB}");
    let (root, _) = hkcu
        .create_subkey(&root_path)
        .with_context(|| format!("creating {root_path}"))?;
    root.set_value("MUIVerb", &"Offspring")?;
    root.set_value("Icon", &format!("{exe},0"))?;
    root.set_value("ExtendedSubCommandsKey", &SUB_KEY_NAME)?;

    // Parent for all the child verbs. Explorer resolves
    // ExtendedSubCommandsKey relative to HKCU\Software\Classes (or HKCR),
    // so the path below is just `<SUB_KEY_NAME>\shell\...`.
    let sub_root_path = format!(r"Software\Classes\{SUB_KEY_NAME}\shell");
    let (_sub_root, _) = hkcu
        .create_subkey(&sub_root_path)
        .with_context(|| format!("creating {sub_root_path}"))?;

    // Child verbs, zero-padded so Explorer's alphabetical sort preserves
    // the preset order the user set in the app.
    for (idx, preset) in presets.iter().filter(|p| p.enabled).enumerate() {
        let key_name = format!("{:02}_{}", idx + 1, sanitize_id(&preset.id));
        let verb_path = format!(r"{sub_root_path}\{key_name}");
        let (verb, _) = hkcu
            .create_subkey(&verb_path)
            .with_context(|| format!("creating {verb_path}"))?;
        verb.set_value("MUIVerb", &preset.name)?;
        if let Some(icon) = preset.icon.as_ref().filter(|s| !s.is_empty()) {
            verb.set_value("Icon", icon)?;
        }

        let (cmd, _) = hkcu
            .create_subkey(format!(r"{verb_path}\command"))
            .context("creating command subkey")?;
        let cmdline = format!("\"{exe}\" preset --id {} \"%1\"", preset.id);
        cmd.set_value("", &cmdline)?;
    }

    // Trailing "Custom..." entry. 99_ prefix keeps it at the bottom even if
    // the user ever has 90+ presets (which would be unusual).
    let custom_path = format!(r"{sub_root_path}\99_custom");
    let (custom, _) = hkcu
        .create_subkey(&custom_path)
        .with_context(|| format!("creating {custom_path}"))?;
    custom.set_value("MUIVerb", &"Custom...")?;
    let (custom_cmd, _) = hkcu
        .create_subkey(format!(r"{custom_path}\command"))
        .context("creating custom command subkey")?;
    custom_cmd.set_value("", &format!("\"{exe}\" custom \"%1\""))?;

    Ok(())
}

/// Remove our entire subtree. Safe to call when nothing is installed —
/// missing keys are ignored. Does NOT remove `HKCU\Software\Offspring`
/// (the shared ExePath key read by the shell-extension DLL) because the
/// MSIX integration may still need it; `integration::cleanup_all` is
/// what fully removes it at uninstall time.
pub fn cleanup() -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\*\shell\{ROOT_VERB}"));
    let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\{SUB_KEY_NAME}"));
    Ok(())
}

/// Registry key names can't contain `\`, so scrub anything dangerous from
/// the preset id before using it as part of a key name. Preset ids are
/// normally already safe (ascii + underscores) but this is a cheap hedge.
fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect()
}
