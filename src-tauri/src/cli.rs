use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "Offspring", version, about = "Right-click convert videos/images with FFmpeg")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
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
    /// First-run: seed default presets + populate SendTo folder
    FirstRun,
    /// Remove all SendTo shortcuts created by this app
    Cleanup,
}
