# Local release build for Offspring. Produces the installer that ships
# to users, so you can test the exact binary before publishing.
#
# Steps (mirrors .github/workflows/release.yml):
#   0. bump-version.ps1              -> updates package.json + Cargo.tomls
#                                     + tauri.conf.json + offspring.iss
#                                     to a new version
#   1. npm run tauri build           -> src-tauri/target/release/offspring.exe
#   2. cargo build --release in      -> shell-ext/target/release/offspring_shell_ext.dll
#      shell-ext/
#   3. Copy the shell-ext DLL        -> src-tauri/target/release/
#      alongside offspring.exe (Inno [Files] picks it up from there)
#   4. build-msix.ps1                -> installer/msix/dist/OffspringShellExt.msix + .cer
#   5. iscc.exe installer/offspring.iss -> installer/dist/Offspring-Setup-<ver>.exe
#
# Version handling:
#   default            "0.3.41"       -> "0.3.41-b0001"    (local iteration)
#                      "0.3.41-b0007" -> "0.3.41-b0008"    (counter bump)
#   -Release           "0.3.41-b0007" -> "0.3.42"          (strip suffix, patch+1)
#                      "0.3.41"       -> "0.3.42"          (patch+1)
#   -Version X.Y.Z[-bNNNN]                                 (explicit override)
#
# Local iterations get a "-bNNNN" suffix so the installer filename, the
# AppVersion in installed metadata, and every Cargo/npm crate version
# stay traceable per build. The MSIX manifest gets a four-numeric form
# automatically (-bNNNN -> .NNNN). The "b" prefix is required because
# strict SemVer 2.0.0 forbids leading zeroes on numeric pre-release
# identifiers; alphanumeric ones (like "b0001") are fine. Use -Release
# when you're ready to publish a build to GitHub.
#
# Prerequisites (same as CI):
#   Node 20+, Rust stable, Windows 10 SDK, Inno Setup 6 (iscc.exe on PATH
#   or at default location). MSIX cert is auto-generated on first run.

[CmdletBinding()]
param(
    [string]$Version,
    [switch]$Release,       # bump patch and strip -NNNN suffix
    [switch]$SkipInstall,   # skip `npm ci` (faster on repeat builds)
    [switch]$OpenOutput     # open Explorer on installer/dist/ when done
)

$ErrorActionPreference = "Stop"
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")

Push-Location $repoRoot
try {
    # --- 0. version bump ------------------------------------------------
    # Always run the bumper. Even an explicit -Version arg flows through
    # it so all five files (package.json, both Cargo.tomls, tauri.conf,
    # offspring.iss) plus package-lock.json end up in lockstep.
    $bumper = Join-Path $repoRoot "tools\bump-version.ps1"
    Write-Host ""
    Write-Host "[0/5] bump-version..." -ForegroundColor Yellow
    if ($Version) {
        $Version = & $bumper -Set $Version
    } elseif ($Release) {
        $Version = & $bumper -Release
    } else {
        $Version = & $bumper
    }
    if ($LASTEXITCODE -ne 0 -or -not $Version) { throw "bump-version.ps1 failed" }
    # bump-version returns the new semver via stdout; the four-numeric
    # MSIX form is what build-msix.ps1 wants. Derive it here from the
    # new semver (same logic as inside the bumper, kept duplicated to
    # avoid dot-sourcing across two scripts).
    if (-not ($Version -match '^(\d+)\.(\d+)\.(\d+)(?:-b(\d+))?$')) {
        throw "Bumper returned a version we can't parse: '$Version'"
    }
    $msixCounter = if ($Matches[4]) { [int]$Matches[4] } else { 0 }
    $msixVersion = "$($Matches[1]).$($Matches[2]).$($Matches[3]).$msixCounter"

    Write-Host ""
    Write-Host "============================================================" -ForegroundColor Cyan
    Write-Host " Offspring local release build - $Version" -ForegroundColor Cyan
    Write-Host "                          (msix: $msixVersion)" -ForegroundColor DarkCyan
    Write-Host "============================================================" -ForegroundColor Cyan
    Write-Host ""

    # --- 1. npm ----------------------------------------------------------
    if (-not $SkipInstall) {
        Write-Host "[1/5] npm ci..." -ForegroundColor Yellow
        npm ci
        if ($LASTEXITCODE -ne 0) { throw "npm ci failed" }
    } else {
        Write-Host "[1/5] npm ci (skipped)" -ForegroundColor DarkGray
    }

    # --- 2. tauri build --------------------------------------------------
    Write-Host ""
    Write-Host "[2/5] npm run tauri build..." -ForegroundColor Yellow
    npm run tauri build
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }
    # Cargo respects CARGO_TARGET_DIR for dev machines that share a target
    # directory across projects — on those, the binary lives at
    # $env:CARGO_TARGET_DIR\release, not src-tauri\target\release. Mirror
    # the same logic installer/offspring.iss uses so the two stay in sync.
    $targetRelease = if ($env:CARGO_TARGET_DIR) {
        Join-Path $env:CARGO_TARGET_DIR "release"
    } else {
        Join-Path $repoRoot "src-tauri\target\release"
    }
    $exe = Join-Path $targetRelease "offspring.exe"
    if (-not (Test-Path $exe)) { throw "offspring.exe not at $exe after tauri build" }

    # --- 3. shell-ext DLL ------------------------------------------------
    Write-Host ""
    Write-Host "[3/5] cargo build --release (shell-ext)..." -ForegroundColor Yellow
    Push-Location (Join-Path $repoRoot "shell-ext")
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw "shell-ext cargo build failed" }
    } finally {
        Pop-Location
    }
    # shell-ext lives in its own Cargo project with its own target dir,
    # but it ALSO respects CARGO_TARGET_DIR when set. Try the shared dir
    # first, then fall back to the per-project default.
    $dll = if ($env:CARGO_TARGET_DIR) {
        Join-Path $env:CARGO_TARGET_DIR "release\offspring_shell_ext.dll"
    } else {
        Join-Path $repoRoot "shell-ext\target\release\offspring_shell_ext.dll"
    }
    if (-not (Test-Path $dll)) {
        # Fallback for the case where shell-ext has its own target dir
        # (no shared CARGO_TARGET_DIR) but offspring uses the shared one.
        $alt = Join-Path $repoRoot "shell-ext\target\release\offspring_shell_ext.dll"
        if (Test-Path $alt) { $dll = $alt }
        else { throw "offspring_shell_ext.dll not found at $dll" }
    }
    # When CARGO_TARGET_DIR is shared across offspring + shell-ext (the
    # common dev setup), the DLL is already alongside offspring.exe — skip
    # the copy rather than erroring on "can't overwrite with itself".
    $dllDest = Join-Path $targetRelease "offspring_shell_ext.dll"
    if ((Resolve-Path $dll).Path -ne $dllDest) {
        Copy-Item $dll $targetRelease -Force
    }

    # --- 4. MSIX ---------------------------------------------------------
    # MSIX manifest schema rejects pre-release tags ("0.3.41-0007"
    # parses as bad). Pass the four-numeric form computed from the
    # bumper output instead.
    Write-Host ""
    Write-Host "[4/5] build-msix.ps1 (msix version $msixVersion)..." -ForegroundColor Yellow
    pwsh (Join-Path $repoRoot "installer\msix\build-msix.ps1") -Version $msixVersion
    if ($LASTEXITCODE -ne 0) { throw "build-msix.ps1 failed" }

    # --- 5. Inno Setup ---------------------------------------------------
    Write-Host ""
    Write-Host "[5/5] Inno Setup (iscc.exe)..." -ForegroundColor Yellow
    $iscc = $null
    # Per-machine installs land in Program Files; winget's default
    # per-user install lands under %LOCALAPPDATA%\Programs\. Check both.
    $candidates = @(
        "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
        "C:\Program Files\Inno Setup 6\ISCC.exe",
        (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe")
    )
    foreach ($cand in $candidates) {
        if (Test-Path $cand) { $iscc = $cand; break }
    }
    if (-not $iscc) {
        $onPath = Get-Command ISCC.exe -ErrorAction SilentlyContinue
        if ($onPath) { $iscc = $onPath.Source }
    }
    if (-not $iscc) {
        throw "iscc.exe not found. Install Inno Setup 6 (https://jrsoftware.org/isdl.php) or add iscc.exe to PATH."
    }
    & $iscc (Join-Path $repoRoot "installer\offspring.iss")
    if ($LASTEXITCODE -ne 0) { throw "iscc.exe failed" }

    $installer = Join-Path $repoRoot "installer\dist\Offspring-Setup-$Version.exe"
    if (-not (Test-Path $installer)) {
        throw "Expected installer at $installer but it wasn't produced"
    }

    # The unversioned "latest" copy is what gets attached to GitHub
    # releases for the forever-link
    #   https://github.com/honear/offspring/releases/latest/download/Offspring-Setup.exe
    # We only refresh it on -Release builds so local iteration builds
    # ("0.3.41-0007") don't masquerade as the published "latest" — the
    # marketing site link would otherwise serve a half-finished build.
    $isRelease = $Version -notmatch '-b\d+$'
    $installerLatest = Join-Path $repoRoot "installer\dist\Offspring-Setup.exe"
    if ($isRelease) {
        Copy-Item $installer $installerLatest -Force
    }

    Write-Host ""
    Write-Host "============================================================" -ForegroundColor Green
    Write-Host " Build OK" -ForegroundColor Green
    Write-Host "============================================================" -ForegroundColor Green
    Write-Host "  Installer: $installer" -ForegroundColor Green
    if ($isRelease) {
        Write-Host "  Latest:    $installerLatest  (refreshed)" -ForegroundColor Green
    } else {
        Write-Host "  Latest:    (skipped — local iteration build)" -ForegroundColor DarkGray
    }
    $size = (Get-Item $installer).Length / 1MB
    Write-Host ("  Size:      {0:N2} MB" -f $size) -ForegroundColor Green
    Write-Host ""
    if ($isRelease) {
        Write-Host "Install it locally to test, then say 'push' to publish." -ForegroundColor Cyan
    } else {
        Write-Host "Local iteration build. Re-run with -Release to cut a publishable build." -ForegroundColor Cyan
    }

    if ($OpenOutput) {
        Start-Process (Split-Path $installer -Parent)
    }
} finally {
    Pop-Location
}
