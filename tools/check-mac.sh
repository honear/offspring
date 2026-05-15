#!/usr/bin/env bash
# One-shot macOS sanity check.
#
# Run from anywhere — script auto-locates the repo root relative to
# its own path:
#   chmod +x tools/check-mac.sh
#   tools/check-mac.sh 2>&1 | tee mac-check.log
#   # or, with absolute path from anywhere:
#   ~/whatever/offspring/tools/check-mac.sh 2>&1 | tee /tmp/mac-check.log
#
# Pipe the whole output back into the Claude conversation on your PC.
# It tells me exactly which #[cfg(windows)] gates are missing, whether
# the Tauri build pipeline runs end-to-end, and what your Mac's FFmpeg
# situation looks like. No code changes, all read-only.

set -uo pipefail   # NOT `set -e` — we want each section to report
                   # independently even if an earlier one fails

# Resolve repo root from this script's own location, regardless of
# where the user invoked it from. Follows symlinks so `tools/check-mac.sh`
# works even if the script is symlinked elsewhere on the user's PATH.
SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
while [ -L "$SCRIPT_PATH" ]; do
    SCRIPT_PATH="$(readlink "$SCRIPT_PATH")"
done
REPO_ROOT="$(cd "$(dirname "$SCRIPT_PATH")/.." && pwd)"

if [ ! -f "$REPO_ROOT/src-tauri/Cargo.toml" ]; then
    echo "ERROR: src-tauri/Cargo.toml not found under $REPO_ROOT"
    echo "       The script located itself at $SCRIPT_PATH"
    echo "       but the parent directory doesn't look like the Offspring repo."
    echo "       Are you running this from a clone that's missing files?"
    exit 1
fi

cd "$REPO_ROOT"
echo "(repo root: $REPO_ROOT)"
echo ""

echo "============================================================"
echo " Offspring macOS sanity check"
echo " Date: $(date)"
echo " Host: $(uname -a)"
echo " CPU:  $(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo unknown)"
echo "============================================================"
echo ""

# --- 1. Toolchain inventory -------------------------------------------
echo "--- toolchain ---"
echo "rustc:    $(rustc --version 2>&1 || echo 'NOT INSTALLED')"
echo "cargo:    $(cargo --version 2>&1 || echo 'NOT INSTALLED')"
echo "node:     $(node --version 2>&1 || echo 'NOT INSTALLED')"
echo "npm:      $(npm --version 2>&1 || echo 'NOT INSTALLED')"
echo "tauri:    $(cargo tauri --version 2>&1 || echo 'NOT INSTALLED — npm i -g @tauri-apps/cli')"
echo "ffmpeg:   $(which ffmpeg 2>/dev/null || echo 'NOT ON PATH')"
echo "ffmpeg v: $(ffmpeg -version 2>&1 | head -1 || echo 'n/a')"
echo "xcode-clt: $(xcode-select -p 2>&1 || echo 'NOT INSTALLED — xcode-select --install')"
echo ""

# --- 2. Rust targets ---------------------------------------------------
echo "--- rust targets ---"
echo "Installed targets:"
rustup target list --installed 2>&1 || echo "  (rustup not available)"
echo ""
echo "Required for universal macOS builds: x86_64-apple-darwin + aarch64-apple-darwin"
echo "Install missing with: rustup target add x86_64-apple-darwin aarch64-apple-darwin"
echo ""

# --- 3. cargo check for both Mac archs --------------------------------
echo "--- cargo check x86_64-apple-darwin ---"
( cd src-tauri && cargo check --target x86_64-apple-darwin 2>&1 ) | tail -100
echo ""

echo "--- cargo check aarch64-apple-darwin ---"
( cd src-tauri && cargo check --target aarch64-apple-darwin 2>&1 ) | tail -100
echo ""

# --- 4. cargo check studio feature for both archs ---------------------
# Lower priority since we don't ship Mac Studio, but worth seeing if it
# even compiles — confirms our cfg gates are clean both ways.
echo "--- cargo check --features studio aarch64-apple-darwin ---"
( cd src-tauri && cargo check --features studio --target aarch64-apple-darwin 2>&1 ) | tail -40
echo ""

# --- 5. Frontend build pipeline ---------------------------------------
echo "--- vite + svelte-check ---"
echo "Skipping npm install (run it manually if node_modules is missing)."
if [ -d node_modules ]; then
    npm run check 2>&1 | tail -30
else
    echo "  node_modules absent. Run: npm ci"
fi
echo ""

# --- 6. Tauri config sanity -------------------------------------------
echo "--- tauri.conf.json identifier + bundle.targets ---"
if command -v jq >/dev/null 2>&1; then
    jq '.identifier, .bundle.targets, .productName' src-tauri/tauri.conf.json 2>&1
else
    grep -E '"identifier"|"productName"|"targets"' src-tauri/tauri.conf.json
fi
echo ""

# --- 7. Files that scream Windows-only --------------------------------
echo "--- files that may need macOS branches ---"
echo "Files containing winreg / Win32 imports:"
grep -rl --include='*.rs' -E 'use winreg|use windows::|extern crate winapi' src-tauri/src 2>/dev/null | sort
echo ""
echo "Files with #[cfg(windows)] gates:"
grep -rl --include='*.rs' '#\[cfg(windows)\]' src-tauri/src 2>/dev/null | sort
echo ""

# --- 8. Disk space check ----------------------------------------------
echo "--- disk space ---"
df -h . | head -2
echo "  (Tauri Mac build needs ~3GB free for both arch slices)"
echo ""

echo "============================================================"
echo " Done. Paste this whole log back to Claude on the PC."
echo "============================================================"
