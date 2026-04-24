//! Image-sequence detection + ffmpeg-input helpers.
//!
//! When the user right-clicks a single image file, we want to recognize
//! things like `render_0001.png` as one frame of a numbered sequence and
//! hand ffmpeg the whole run via its `image2` demuxer, instead of
//! converting the single frame. Detection is conservative:
//!
//!   * Input must be an image extension we know the demuxer handles.
//!   * The filename stem must end in N or more zero-padded digits, where
//!     N = `SequenceTool::min_digits`. A single `r01` (2 digits) version
//!     tag doesn't trigger detection at the default (4); a four-digit
//!     `_0001` does.
//!   * At least one sibling file must share the exact same stem prefix,
//!     same digit count, and same extension. One lone frame isn't a
//!     sequence.
//!
//! The stem comparison is exact-match by design — `file1_0001.png` and
//! `file2_0002.png` are different stems (`file1_` vs `file2_`), so they
//! are never collapsed into one sequence.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Image extensions ffmpeg's `image2` demuxer reliably reads via a
/// `%04d` pattern. Kept narrow — exotic formats (heic, avif) can be
/// added as demand appears, but silently auto-sequencing a .tga a user
/// didn't mean to batch is worse than not sequencing at all.
pub const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "exr", "tif", "tiff", "bmp", "webp",
];

/// Everything ffmpeg needs to read the sequence back, plus enough
/// metadata for callers to name the output and log what was detected.
#[derive(Debug, Clone)]
pub struct SequenceInfo {
    /// Directory the frames live in (absolute).
    pub dir: PathBuf,
    /// Leading portion of the filename before the digit run, including
    /// any separator char like `_` or `.`. Empty is legal (pure digit
    /// filenames like `0001.png`).
    pub stem_prefix: String,
    /// Number of zero-padded digits in the frame number (e.g. 4 for
    /// `_0001`). ffmpeg needs this to build the `%04d` pattern.
    pub digits: u32,
    /// Lowercase extension without leading dot (`png`, `exr`).
    pub ext: String,
    /// Lowest frame number found on disk — ffmpeg's `-start_number`.
    pub start_number: u32,
    /// Total number of siblings matching the pattern (including the
    /// file that triggered detection). Useful for UI / logging.
    pub frame_count: u32,
}

impl SequenceInfo {
    /// Path pattern ffmpeg expects after `-i`, e.g.
    /// `C:\work\render\render_%04d.png`. We stringify via `Path::join`
    /// so the separator matches the platform.
    pub fn ffmpeg_input_pattern(&self) -> PathBuf {
        let fname = format!(
            "{prefix}%0{digits}d.{ext}",
            prefix = self.stem_prefix,
            digits = self.digits,
            ext = self.ext,
        );
        self.dir.join(fname)
    }

    /// Stem to use when naming the output file — `render_0001.png`
    /// becomes `render` (trailing separator + digits stripped). Falls
    /// back to the prefix verbatim if it's empty or all-separator.
    pub fn output_stem(&self) -> String {
        let trimmed = self.stem_prefix.trim_end_matches(['_', '-', '.', ' ']);
        if trimmed.is_empty() {
            // Sequence like `0001.png` with no prefix — best we can do
            // is a generic name rather than silently writing to `.gif`.
            "sequence".to_string()
        } else {
            trimmed.to_string()
        }
    }
}

/// Run detection on a single input path. Returns `None` for videos,
/// unknown extensions, filenames that don't end in enough digits, or
/// sequences with no siblings.
pub fn detect(path: &Path, min_digits: u32) -> Option<SequenceInfo> {
    let ext = path.extension().and_then(OsStr::to_str)?.to_ascii_lowercase();
    if !IMAGE_EXTS.iter().any(|e| *e == ext) {
        return None;
    }
    let file_stem = path.file_stem().and_then(OsStr::to_str)?;
    let (prefix, digit_run) = split_trailing_digits(file_stem)?;
    if (digit_run.len() as u32) < min_digits {
        return None;
    }
    let dir = path.parent()?.to_path_buf();
    let digits = digit_run.len() as u32;

    // Scan siblings. We require an exact match on:
    //   prefix + <exactly `digits` ASCII digits> + "." + ext
    // Other digit counts (e.g. 5-digit frames in the same dir) are
    // treated as separate sequences and ignored — lets users keep
    // multiple render passes side-by-side without them getting merged.
    let mut frames: Vec<u32> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else { return None };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(sib_ext) = p.extension().and_then(OsStr::to_str) else { continue };
        if !sib_ext.eq_ignore_ascii_case(&ext) {
            continue;
        }
        let Some(sib_stem) = p.file_stem().and_then(OsStr::to_str) else { continue };
        let Some((sib_prefix, sib_digits)) = split_trailing_digits(sib_stem) else { continue };
        if sib_prefix != prefix || sib_digits.len() != digits as usize {
            continue;
        }
        if let Ok(n) = sib_digits.parse::<u32>() {
            frames.push(n);
        }
    }
    // Sequence requires ≥2 total frames (the triggering file plus at
    // least one sibling). A single-frame "sequence" is just a file.
    if frames.len() < 2 {
        return None;
    }
    frames.sort_unstable();
    let start_number = frames[0];
    let frame_count = frames.len() as u32;

    Some(SequenceInfo {
        dir,
        stem_prefix: prefix.to_string(),
        digits,
        ext,
        start_number,
        frame_count,
    })
}

/// Split `"render_0042"` into `("render_", "0042")`. Returns None for
/// names that don't end in at least one ASCII digit. The digit run is
/// greedy — `v002_0042` splits into `("v002_", "0042")`, not `("v", "0020042")`.
fn split_trailing_digits(stem: &str) -> Option<(&str, &str)> {
    let bytes = stem.as_bytes();
    let mut i = bytes.len();
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    if i == bytes.len() {
        return None;
    }
    Some((&stem[..i], &stem[i..]))
}

/// Collapse a caller-supplied file list so frames of the same sequence
/// only trigger one encode. Preserves input order by keeping the first
/// occurrence of each sequence key and discarding later frames of it.
/// Non-sequence entries pass through untouched.
pub fn dedupe_sequence_frames(files: &[PathBuf], min_digits: u32) -> Vec<PathBuf> {
    let mut seen_keys: Vec<(PathBuf, String, u32, String)> = Vec::new();
    let mut out: Vec<PathBuf> = Vec::with_capacity(files.len());
    for f in files {
        match detect(f, min_digits) {
            Some(info) => {
                let key = (info.dir.clone(), info.stem_prefix.clone(), info.digits, info.ext.clone());
                if !seen_keys.contains(&key) {
                    seen_keys.push(key);
                    out.push(f.clone());
                }
            }
            None => out.push(f.clone()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_trailing_digits() {
        assert_eq!(split_trailing_digits("render_0042"), Some(("render_", "0042")));
        assert_eq!(split_trailing_digits("0001"), Some(("", "0001")));
        assert_eq!(split_trailing_digits("v002_0042"), Some(("v002_", "0042")));
        assert_eq!(split_trailing_digits("render"), None);
        assert_eq!(split_trailing_digits("render_"), None);
    }

    #[test]
    fn output_stem_strips_separator() {
        let info = SequenceInfo {
            dir: PathBuf::from("/tmp"),
            stem_prefix: "render_".into(),
            digits: 4,
            ext: "png".into(),
            start_number: 1,
            frame_count: 10,
        };
        assert_eq!(info.output_stem(), "render");
    }

    #[test]
    fn output_stem_handles_bare_digits() {
        let info = SequenceInfo {
            dir: PathBuf::from("/tmp"),
            stem_prefix: "".into(),
            digits: 4,
            ext: "png".into(),
            start_number: 1,
            frame_count: 10,
        };
        assert_eq!(info.output_stem(), "sequence");
    }
}
