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

/// Two top-level verbs instead of one. The classic shell flyout cap
/// is much tighter than the documented 16 — empirically around 9-10
/// items on some Windows builds — and even one level of nesting
/// (the previous "Tools ►" sub-flyout) didn't reliably keep both
/// flyouts under the cap. Splitting the surface across two
/// top-level entries gives each its own item budget.
///
/// Layout in Explorer's right-click menu:
///
///   Offspring Presets ►       ← user-defined preset list + "Custom..."
///     GIF 420px LQ 20fps
///     ...
///     Custom...
///   Offspring Tools ►         ← built-in tool verbs + Settings
///     Greyscale
///     ...
///     Modify...
///     Offspring settings
const ROOT_VERB_PRESETS: &str = "OffspringPresets";
const ROOT_VERB_TOOLS: &str = "OffspringTools";

const SUB_KEY_NAME_PRESETS: &str = "Offspring.SubCommands.Presets";
const SUB_KEY_NAME_TOOLS: &str = "Offspring.SubCommands.Tools";

/// Legacy keys from earlier versions when there was a single
/// "Offspring" top-level. We delete these on every sync so users
/// upgrading from the old layout don't end up with both menus
/// stacked on top of each other.
const LEGACY_ROOT_VERB: &str = "Offspring";
const LEGACY_SUB_KEY_NAME: &str = "Offspring.SubCommands";

fn current_exe_string() -> Result<String> {
    let exe = std::env::current_exe().context("getting current exe path")?;
    Ok(exe.to_string_lossy().into_owned())
}

/// Write two top-level verbs ("Offspring Presets" + "Offspring Tools")
/// each with their own ExtendedSubCommandsKey flyout. Each flyout
/// stays well under the empirical ~10-item per-flyout cap that
/// trips up the classic shell.
pub fn sync(presets: &[Preset], settings: &Settings) -> Result<()> {
    // Start clean — easiest way to handle renames/removed presets is to
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

    // Helper closure: write a leaf verb at <sub_root_path>\<key_name>.
    // MultiSelectModel=Player tells Explorer to batch all selected
    // files into one invocation; `%1` gets the right-clicked file and
    // the shell appends the rest of the selection as additional argv
    // entries — clap's `files: Vec<PathBuf>` captures them all.
    let write_leaf = |sub_root_path: &str,
                      key_name: &str,
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

    // ---- Top-level "Offspring Presets" verb --------------------------
    let presets_root_path = format!(r"Software\Classes\*\shell\{ROOT_VERB_PRESETS}");
    let (presets_root, _) = hkcu
        .create_subkey(&presets_root_path)
        .with_context(|| format!("creating {presets_root_path}"))?;
    presets_root.set_value("MUIVerb", &"Offspring Presets")?;
    presets_root.set_value("Icon", &format!("{exe},0"))?;
    presets_root.set_value("MultiSelectModel", &"Player")?;
    presets_root.set_value("ExtendedSubCommandsKey", &SUB_KEY_NAME_PRESETS)?;

    let presets_sub_path = format!(r"Software\Classes\{SUB_KEY_NAME_PRESETS}\shell");
    hkcu.create_subkey(&presets_sub_path)
        .with_context(|| format!("creating {presets_sub_path}"))?;

    // Preset verbs, zero-padded so Explorer's alphanumeric sort
    // preserves the user's preset order from the app.
    for (idx, preset) in presets.iter().filter(|p| p.enabled).enumerate() {
        let key_name = format!("{:02}_{}", idx + 1, sanitize_id(&preset.id));
        let icon = preset
            .icon
            .as_ref()
            .filter(|s| !s.is_empty())
            .map(|s| s.as_str());
        let cmd_tail = format!("preset --id {}", preset.id);
        write_leaf(&presets_sub_path, &key_name, &preset.name, icon, &cmd_tail)?;
    }

    // Trailing "Custom..." inside the Presets flyout. 99_ keeps it at
    // the bottom even with many user presets.
    write_leaf(&presets_sub_path, "99_custom", "Custom...", None, "custom")?;

    // ---- Top-level "Offspring Tools" verb ----------------------------
    let tools_root_path = format!(r"Software\Classes\*\shell\{ROOT_VERB_TOOLS}");
    let (tools_root, _) = hkcu
        .create_subkey(&tools_root_path)
        .with_context(|| format!("creating {tools_root_path}"))?;
    tools_root.set_value("MUIVerb", &"Offspring Tools")?;
    tools_root.set_value("Icon", &format!("{exe},0"))?;
    tools_root.set_value("MultiSelectModel", &"Player")?;
    tools_root.set_value("ExtendedSubCommandsKey", &SUB_KEY_NAME_TOOLS)?;

    let tools_sub_path = format!(r"Software\Classes\{SUB_KEY_NAME_TOOLS}\shell");
    hkcu.create_subkey(&tools_sub_path)
        .with_context(|| format!("creating {tools_sub_path}"))?;

    // Tool entries. Order here = order in the flyout. The boolean is
    // the per-tool enabled flag; disabled tools are skipped (not
    // hidden via ECS_HIDDEN — that's a modern-menu concept; classic
    // menu just doesn't write them).
    let tool_entries: [(bool, &str, &str); 8] = [
        (settings.tools.grayscale.enabled,    "Greyscale",    "grayscale"),
        (settings.tools.overlay.enabled,      "Overlay",      "overlay"),
        (settings.tools.merge.enabled,        "Merge",        "merge"),
        (settings.tools.compare.enabled,      "Compare",      "compare"),
        (settings.tools.trim.enabled,         "Trim...",      "trim"),
        (settings.tools.invert.enabled,       "Invert",       "invert"),
        (settings.tools.make_square.enabled,  "Make Square",  "make-square"),
        (settings.tools.modify.enabled,       "Modify...",    "modify"),
    ];
    let mut tool_idx = 0;
    for (enabled, label, cmd_tail) in tool_entries.iter() {
        if !enabled {
            continue;
        }
        tool_idx += 1;
        let key_name = format!("{:02}_{}", tool_idx, sanitize_id(cmd_tail));
        write_leaf(&tools_sub_path, &key_name, label, None, cmd_tail)?;
    }

    // "Offspring settings" anchored at the end of the Tools flyout.
    // `zz_` sorts after `01_..09_` numeric keys. No `%1` arg because
    // the Settings command opens the main UI regardless of selection.
    let settings_path = format!(r"{tools_sub_path}\zz_settings");
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

/// Remove our entire registry subtree from both the current layout
/// (two top-level entries) and any earlier layout we might be
/// upgrading from (single "Offspring" entry, single sub-flyout, etc).
/// Safe to call when nothing is installed — missing keys are ignored.
/// Does NOT remove `HKCU\Software\Offspring` (the shared ExePath key
/// read by the shell-ext DLL) because the MSIX integration may still
/// need it; `integration::cleanup_all` is what fully removes it at
/// uninstall time.
pub fn cleanup() -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    // Current layout
    let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\*\shell\{ROOT_VERB_PRESETS}"));
    let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\*\shell\{ROOT_VERB_TOOLS}"));
    let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\{SUB_KEY_NAME_PRESETS}"));
    let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\{SUB_KEY_NAME_TOOLS}"));
    // Legacy single-top-level layout (everything under one Offspring entry)
    let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\*\shell\{LEGACY_ROOT_VERB}"));
    let _ = hkcu.delete_subkey_all(format!(r"Software\Classes\{LEGACY_SUB_KEY_NAME}"));
    // Older sub-flyout key from the b0004/b0005 attempt
    let _ = hkcu.delete_subkey_all(r"Software\Classes\Offspring.SubCommands.Tools");
    // Even older top-level multi-file verbs (0.3.27)
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
