use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(name = "Offspring", version, about = "Right-click convert videos/images with FFmpeg")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Encode files with a named preset (headless mode)
    Preset {
        /// Preset id (e.g. "gif_500_mq_25")
        #[arg(long)]
        id: String,
        /// Input files
        files: Vec<PathBuf>,
    },
    /// Open the custom tweak dialog pre-filled with last settings
    Custom {
        /// Input files
        files: Vec<PathBuf>,
    },
    /// Merge multiple video/GIF files into one. Files are concatenated in
    /// filename order via ffmpeg's concat demuxer. Output format and
    /// encode settings are inherited from the first file — no preset
    /// picker. Invoked from the shell extension's Merge entry (and from
    /// a single "Offspring Merge" SendTo shortcut when that's enabled).
    Merge {
        /// Input files (must be ≥2).
        files: Vec<PathBuf>,
    },
    /// Convert video(s) or GIF(s) to greyscale, preserving format and
    /// source settings (dimensions, fps). Each file is encoded
    /// independently — no multi-file merging. Invoked from the shell
    /// extension's Greyscale entry and the "Offspring Greyscale" SendTo
    /// shortcut when that's enabled.
    Grayscale {
        /// Input files.
        files: Vec<PathBuf>,
    },
    /// Side-by-side A/B compare: stack N input videos horizontally into
    /// a single output for visual comparison. All inputs are normalized
    /// to the first file's height and framerate; output format matches
    /// the first file. Named `<first-stem>_compare.<ext>`.
    Compare {
        /// Input files (must be ≥2).
        files: Vec<PathBuf>,
    },
    /// Rich overlay: per-corner filename / timecode / custom text, with
    /// optional border strip and aspect-ratio guides. All corners share
    /// color + opacity; configured under the Overlay tool settings.
    /// Output is named `<stem>_overlay.<ext>` per input.
    Overlay {
        /// Input files.
        files: Vec<PathBuf>,
    },
    /// Open the main Offspring settings window. Wired to a trailing
    /// "Offspring settings" entry in the right-click menu so users can
    /// reach the UI without launching the app from Start.
    Settings,
    /// First-run: seed default presets + populate SendTo folder
    FirstRun,
    /// Remove all SendTo shortcuts created by this app
    Cleanup,
}
