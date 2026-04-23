//! Shell integrations — the places where Offspring shows up outside the app
//! window.
//!
//! Two back-ends live here:
//!
//! * `sendto` — writes .lnk files into the user's SendTo folder. Works on
//!   every Windows version but on Windows 11 it's buried behind "Show more
//!   options" (one extra click). Toggled off by default in Settings.
//!
//! * `context_menu` — writes an `HKCU\Software\Classes\*\shell\Offspring`
//!   registry tree with an `ExtendedSubCommandsKey` pointing at per-preset
//!   verbs. This is the classic ("Show more options") context menu. It is
//!   _not_ the Windows 11 modern right-click menu — that requires a COM
//!   shell-extension shipped in an MSIX package, which lives behind a
//!   separate opt-in toggle (see Phase 3+ of the shell-extension feature).
//!
//! Both modules expose the same pair: `sync(presets)` and `cleanup()`. Keep
//! them idempotent — the first-run hook in `cli.rs` and the uninstaller's
//! `cleanup` command both call them unconditionally.

pub mod context_menu;
pub mod sendto;
