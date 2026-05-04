//! Legacy ("Show more options") right-click submenu via the user-hive
//! registry.
//!
//! Writes:
//!
//! ```text
//! HKCU\Software\Classes\*\shell\Offspring
//!   MUIVerb                = "Offspring"
//!   Icon                   = "<exe>,0"
//!   MultiSelectModel       = "Player"
//!   ExtendedSubCommandsKey = "Offspring.SubCommands"
//!
//! HKCU\Software\Classes\Offspring.SubCommands\shell\NN_<verb>
//!   MUIVerb          = "<label>"
//!   MultiSelectModel = "Player"
//!   Icon             = "<icon>" (preset verbs only, if set)
//!   \command
//!     (Default) = "\"<exe>\" <subcommand> [--id <preset.id>] \"%1\""
//! ```
//!
//! Every action verb (presets, merge, compare, grayscale, overlay,
//! custom) carries `MultiSelectModel=Player` so Explorer batches all
//! selected files into a single invocation — argv gets the right-clicked
//! file via `%1` plus every other selected path appended by the shell.
//! Offspring's CLI already accepts `files: Vec<PathBuf>` for each
//! subcommand and the progress window iterates them sequentially, so
//! multi-select "just works" end-to-end once the shell cooperates.
//!
//! The one verb that does NOT need `Player` is `Offspring settings` —
//! it takes no file argument.
//!
//! HKCU means no elevation, per-user install. `*` matches every file
//! type — same surface as the SendTo shortcuts this replaces.
//!
//! This is NOT the Windows 11 modern (top-level) right-click menu. That
//! requires a COM shell extension packaged in an MSIX, which is a
//! separate opt-in behind its own Settings toggle.

use anyhow::{Context, Result};
use winreg::enums::*;
use winreg::RegKey;

use crate::presets::{Preset, Settings};

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

/// Write the root verb + one child per enabled preset + tool entries +
/// a trailing Custom… + settings.
pub fn sync(presets: &[Preset], settings: &Settings) -> Result<()> {
    // Start clean — the easiest way to handle renames/removed presets is to
    // nuke the whole subtree and rebuild. These keys are entirely ours.
    let _ = cleanup();

    let exe = current_exe_string()?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // Publish the install path so the MSIX shell-extension DLL can resolve
    // offspring.exe without hard-coding a program-files path. The DLL runs
    // inside Explorer.exe, has no relation to our install directory, and
    // the MSIX package location isn't ours either — this registry value
    // is the contract between the two.
    let (root_key, _) = hkcu.create_subkey(r"Software\Offspring")?;
    root_key.set_value("ExePath", &exe)?;

    // Top-level verb that Explorer will show as "Offspring" with a ►.
    // MultiSelectModel on the parent is a belt-and-braces signal — the
    // actual multi-file batching is decided by the child verb we invoke,
    // but setting it here keeps Explorer from downgrading the flyout's
    // own multi-select handling on some shell versions.
    let root_path = format!(r"Software\Classes\*\shell\{ROOT_VERB}");
    let (root, _) = hkcu
        .create_subkey(&root_path)
        .with_context(|| format!("creating {root_path}"))?;
    root.set_value("MUIVerb", &"Offspring")?;
    root.set_value("Icon", &format!("{exe},0"))?;
    root.set_value("MultiSelectModel", &"Player")?;
    root.set_value("ExtendedSubCommandsKey", &SUB_KEY_NAME)?;

    // Parent for all the child verbs. Explorer resolves
    // ExtendedSubCommandsKey relative to HKCU\Software\Classes (or HKCR),
    // so the path below is just `<SUB_KEY_NAME>\shell\...`.
    let sub_root_path = format!(r"Software\Classes\{SUB_KEY_NAME}\shell");
    let (_sub_root, _) = hkcu
        .create_subkey(&sub_root_path)
        .with_context(|| format!("creating {sub_root_path}"))?;

    // Helper: write a nested sub-verb with MultiSelectModel=Player so
    // Explorer hands every selected file to one invocation. `%1` gets the
    // right-clicked file and the shell appends the remaining selection
    // as additional argv entries — clap's `files: Vec<PathBuf>` captures
    // them all.
    let write_nested_verb = |key_name: &str,
                             label: &str,
                             icon: Option<&str>,
                             command_tail: &str|
     -> Result<()> {
        let verb_path = format!(r"{sub_root_path}\{key_name}");
        let (verb, _) = hkcu
            .create_subkey(&verb_path)
            .with_context(|| format!("creating {verb_path}"))?;
        verb.set_value("MUIVerb", &label)?;
        verb.set_value("MultiSelectModel", &"Player")?;
        if let Some(icon) = icon {
            verb.set_value("Icon", &icon)?;
        }
        let (cmd, _) = hkcu
            .create_subkey(format!(r"{verb_path}\command"))
            .context("creating command subkey")?;
        cmd.set_value("", &format!("\"{exe}\" {command_tail} \"%1\""))?;
        Ok(())
    };

    // Preset verbs, zero-padded so Explorer's alphanumeric sort preserves
    // the preset order the user set in the app.
    for (idx, preset) in presets.iter().filter(|p| p.enabled).enumerate() {
        let key_name = format!("{:02}_{}", idx + 1, sanitize_id(&preset.id));
        let icon = preset
            .icon
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|s| s.as_str());
        let cmd_tail = format!("preset --id {}", preset.id);
        write_nested_verb(&key_name, &preset.name, icon, &cmd_tail)?;
    }

    // Tool verbs — all share the nested + Player approach. Numeric
    // prefixes 80–89 keep them grouped below presets (01–NN) and above
    // the trailing Custom (99) / settings (zz) entries.
    if settings.tools.grayscale.enabled {
        write_nested_verb("80_grayscale", "Greyscale", None, "grayscale")?;
    }
    if settings.tools.overlay.enabled {
        write_nested_verb("81_overlay", "Overlay", None, "overlay")?;
    }
    if settings.tools.merge.enabled {
        write_nested_verb("82_merge", "Merge", None, "merge")?;
    }
    if settings.tools.compare.enabled {
        write_nested_verb("83_compare", "Compare", None, "compare")?;
    }
    if settings.tools.trim.enabled {
        write_nested_verb("84_trim", "Trim...", None, "trim")?;
    }
    if settings.tools.invert.enabled {
        write_nested_verb("85_invert", "Invert", None, "invert")?;
    }
    if settings.tools.make_square.enabled {
        write_nested_verb("86_make_square", "Make Square", None, "make-square")?;
    }

    // Trailing "Custom..." entry. 99_ keeps it at the bottom even if the
    // user somehow has 90+ presets (which would be unusual).
    write_nested_verb("99_custom", "Custom...", None, "custom")?;

    // Trailing "Offspring settings" — `zz_` sorts after `99_`. No Player
    // and no `%1`: the verb opens the main UI regardless of what's
    // selected, and it doesn't need any file arg.
    let settings_path = format!(r"{sub_root_path}\zz_settings");
    let (settings_key, _) = hkcu
        .create_subkey(&settings_path)
        .with_context(|| format!("creating {settings_path}"))?;
    settings_key.set_value("MUIVerb", &"Offspring settings")?;
    let (settings_cmd, _) = hkcu
        .create_subkey(format!(r"{settings_path}\command"))
        .context("creating settings command subkey")?;
    settings_cmd.set_value("", &format!("\"{exe}\" settings"))?;

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
    // Legacy top-level multi-file verbs from 0.3.27 — removed now that
    // merge/compare live inside the flyout again.
    let _ = hkcu.delete_subkey_all(r"Software\Classes\*\shell\OffspringMerge");
    let _ = hkcu.delete_subkey_all(r"Software\Classes\*\shell\OffspringCompare");
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
