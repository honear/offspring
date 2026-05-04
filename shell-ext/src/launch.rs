//! Spawn `offspring.exe` with the right CLI args when the user picks a
//! preset from the flyout.
//!
//! Explorer hands us an `IShellItemArray` of the selected items; we
//! convert those to filesystem paths, build the command-line, and
//! launch. Fire-and-forget — the child runs independently and we
//! return quickly so the menu animation doesn't hitch.

use std::path::PathBuf;
use std::process::Command;

use windows::Win32::UI::Shell::*;

use crate::presets::read_exe_path;

/// Pull every filesystem path out of an `IShellItemArray`. Items that
/// don't have a filesystem path (virtual items, library folders) are
/// skipped — we have nothing to do with them anyway.
pub fn items_to_paths(items: Option<&IShellItemArray>) -> Vec<PathBuf> {
    let Some(arr) = items else { return Vec::new() };
    let count = unsafe { arr.GetCount().unwrap_or(0) };
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        unsafe {
            if let Ok(item) = arr.GetItemAt(i) {
                if let Ok(pwstr) = item.GetDisplayName(SIGDN_FILESYSPATH) {
                    if !pwstr.is_null() {
                        let s = pwstr.to_string().unwrap_or_default();
                        if !s.is_empty() {
                            out.push(PathBuf::from(s));
                        }
                        windows::Win32::System::Com::CoTaskMemFree(Some(pwstr.0 as _));
                    }
                }
            }
        }
    }
    out
}

pub fn spawn_preset(preset_id: &str, files: &[PathBuf]) {
    let Some(exe) = read_exe_path() else { return };
    let mut cmd = Command::new(&exe);
    cmd.arg("preset").arg("--id").arg(preset_id);
    for f in files {
        cmd.arg(f);
    }
    let _ = cmd.spawn();
}

pub fn spawn_custom(files: &[PathBuf]) {
    let Some(exe) = read_exe_path() else { return };
    let mut cmd = Command::new(&exe);
    cmd.arg("custom");
    for f in files {
        cmd.arg(f);
    }
    let _ = cmd.spawn();
}

pub fn spawn_merge(files: &[PathBuf]) {
    let Some(exe) = read_exe_path() else { return };
    let mut cmd = Command::new(&exe);
    cmd.arg("merge");
    for f in files {
        cmd.arg(f);
    }
    let _ = cmd.spawn();
}

pub fn spawn_grayscale(files: &[PathBuf]) {
    let Some(exe) = read_exe_path() else { return };
    let mut cmd = Command::new(&exe);
    cmd.arg("grayscale");
    for f in files {
        cmd.arg(f);
    }
    let _ = cmd.spawn();
}

pub fn spawn_compare(files: &[PathBuf]) {
    let Some(exe) = read_exe_path() else { return };
    let mut cmd = Command::new(&exe);
    cmd.arg("compare");
    for f in files {
        cmd.arg(f);
    }
    let _ = cmd.spawn();
}

pub fn spawn_overlay(files: &[PathBuf]) {
    let Some(exe) = read_exe_path() else { return };
    let mut cmd = Command::new(&exe);
    cmd.arg("overlay");
    for f in files {
        cmd.arg(f);
    }
    let _ = cmd.spawn();
}

pub fn spawn_trim(files: &[PathBuf]) {
    let Some(exe) = read_exe_path() else { return };
    let mut cmd = Command::new(&exe);
    cmd.arg("trim");
    for f in files {
        cmd.arg(f);
    }
    let _ = cmd.spawn();
}

pub fn spawn_invert(files: &[PathBuf]) {
    let Some(exe) = read_exe_path() else { return };
    let mut cmd = Command::new(&exe);
    cmd.arg("invert");
    for f in files {
        cmd.arg(f);
    }
    let _ = cmd.spawn();
}

pub fn spawn_make_square(files: &[PathBuf]) {
    let Some(exe) = read_exe_path() else { return };
    let mut cmd = Command::new(&exe);
    cmd.arg("make-square");
    for f in files {
        cmd.arg(f);
    }
    let _ = cmd.spawn();
}

/// Launch the main Offspring UI (the Settings window). No file args —
/// the CLI `settings` verb ignores any selection and always shows the
/// configuration surface.
pub fn spawn_settings() {
    let Some(exe) = read_exe_path() else { return };
    let mut cmd = Command::new(&exe);
    cmd.arg("settings");
    let _ = cmd.spawn();
}
