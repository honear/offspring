//! Windows 11 modern (top-level) right-click menu via an MSIX sparse
//! package registering an `IExplorerCommand` COM handler.
//!
//! This module is a **stub** for Phase 2. `sync` and `cleanup` are no-ops
//! that always succeed, so the Settings toggle can be wired end-to-end
//! without the MSIX plumbing in place yet. Phases 3–5 flesh this out:
//!
//! * Phase 3 — build the shell-extension DLL (`IExplorerCommand` cdylib).
//! * Phase 4 — MSIX manifest + self-signed cert (CN=Second March).
//! * Phase 5 — `sync` shells out to `Add-AppxPackage`, `cleanup` to
//!   `Remove-AppxPackage`, and installs/uninstalls the cert from
//!   `Cert:\CurrentUser\TrustedPeople` on the fly.
//!
//! Keep the signatures stable so the Phase 5 swap is a body change, not
//! an API change.

use anyhow::Result;

use crate::presets::Preset;

pub fn sync(_presets: &[Preset]) -> Result<()> {
    Ok(())
}

pub fn cleanup() -> Result<()> {
    Ok(())
}
