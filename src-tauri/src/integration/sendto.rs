use anyhow::{Context, Result};
use mslnk::ShellLink;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::paths;
use crate::presets::Preset;

/// Legacy filename prefixes. Any .lnk in the user's SendTo folder whose stem
/// starts with one of these followed by " - " is treated as ours and cleaned
/// up on sync. This lets pre-existing installs upgrade cleanly to the new
/// unadorned naming scheme ("GIF 720p.lnk" instead of "Offspring - GIF 720p.lnk").
const LEGACY_PREFIXES: &[&str] = &["Offspring", "toGIF"];

/// On-disk record of which SendTo shortcut filenames belong to us. Without
/// a filename prefix we have no other way to identify our .lnks vs the user's
/// other SendTo entries (e.g. Bluetooth, 7-Zip, Desktop).
#[derive(Serialize, Deserialize, Default)]
struct Manifest {
    shortcuts: Vec<String>,
}

impl Manifest {
    fn load() -> Manifest {
        let Ok(path) = paths::sendto_manifest_path() else { return Manifest::default() };
        if !path.exists() {
            return Manifest::default();
        }
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save(&self) -> Result<()> {
        let path = paths::sendto_manifest_path()?;
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json).context("writing sendto manifest")?;
        Ok(())
    }

    fn clear() -> Result<()> {
        let path = paths::sendto_manifest_path()?;
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        Ok(())
    }
}

fn shortcut_path(name: &str) -> Result<PathBuf> {
    Ok(paths::sendto_dir()?.join(format!("{name}.lnk")))
}

pub fn current_exe() -> Result<PathBuf> {
    std::env::current_exe().context("getting current exe path")
}

pub fn write_preset_shortcut(preset: &Preset) -> Result<PathBuf> {
    let path = shortcut_path(&preset.name)?;
    let exe = current_exe()?;
    let mut link = ShellLink::new(exe.to_string_lossy().as_ref())
        .context("creating shell link")?;
    link.set_arguments(Some(format!("preset --id {}", preset.id)));
    link.set_working_dir(Some(String::new())); // empty so output goes next to source
    if let Some(ref icon) = preset.icon {
        link.set_icon_location(Some(icon.clone()));
    }
    link.create_lnk(&path).context("writing .lnk")?;
    Ok(path)
}

pub fn write_custom_shortcut() -> Result<PathBuf> {
    let path = shortcut_path("Custom...")?;
    let exe = current_exe()?;
    let mut link = ShellLink::new(exe.to_string_lossy().as_ref())
        .context("creating shell link")?;
    link.set_arguments(Some("custom".to_string()));
    link.set_working_dir(Some(String::new()));
    link.create_lnk(&path).context("writing .lnk")?;
    Ok(path)
}

/// Remove any leftover pre-manifest shortcuts from the user's SendTo folder.
/// These are the old "Offspring - *.lnk" / "toGIF - *.lnk" naming we used
/// before switching to unadorned preset names. Safe to run on every sync.
fn remove_legacy_shortcuts() -> Result<()> {
    let dir = paths::sendto_dir()?;
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&dir)?.flatten() {
        let p = entry.path();
        if p.extension().map(|e| e != "lnk").unwrap_or(true) {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
        let looks_legacy = LEGACY_PREFIXES
            .iter()
            .any(|pre| stem.starts_with(&format!("{pre} - ")));
        if looks_legacy {
            let _ = std::fs::remove_file(&p);
        }
    }
    Ok(())
}

/// Remove every shortcut listed in the current manifest. Missing files are
/// silently skipped — the user may have deleted them manually, which is fine.
fn remove_manifest_shortcuts(manifest: &Manifest) -> Result<()> {
    let dir = paths::sendto_dir()?;
    for name in &manifest.shortcuts {
        let path = dir.join(name);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}

pub fn sync(presets: &[Preset]) -> Result<()> {
    // Remove legacy prefix-style shortcuts from any previous version, then
    // remove everything our current manifest claims. This catches renames:
    // the preset "Fast GIF" → "Fast.gif" means the old "Fast GIF.lnk" is in
    // the manifest and gets cleaned up before we write the new name.
    remove_legacy_shortcuts()?;
    let old = Manifest::load();
    remove_manifest_shortcuts(&old)?;

    // Write new shortcuts, collecting their basenames for the manifest.
    // De-dup by name (case-insensitive on Windows) so two presets with the
    // same name don't double-write and fight over the same .lnk.
    let mut written: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let push_if_new = |p: &Path, out: &mut Vec<String>, seen: &mut HashSet<String>| {
        if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
            let key = name.to_lowercase();
            if seen.insert(key) {
                out.push(name.to_string());
            }
        }
    };

    for preset in presets.iter().filter(|p| p.enabled) {
        let p = write_preset_shortcut(preset)?;
        push_if_new(&p, &mut written, &mut seen);
    }
    let cp = write_custom_shortcut()?;
    push_if_new(&cp, &mut written, &mut seen);

    Manifest { shortcuts: written }.save()?;
    Ok(())
}

pub fn cleanup() -> Result<()> {
    remove_legacy_shortcuts()?;
    let m = Manifest::load();
    remove_manifest_shortcuts(&m)?;
    Manifest::clear()?;
    Ok(())
}
