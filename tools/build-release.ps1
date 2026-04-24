# Local release build for Offspring. Produces the installer that ships
# to users, so you can test the exact binary before publishing.
#
# Steps (mirrors .github/workflows/release.yml):
#   1. npm run tauri build           -> src-tauri/target/release/offspring.exe
#   2. cargo build --release in      -> shell-ext/target/release/offspring_shell_ext.dll
#      shell-ext/
#   3. Copy the shell-ext DLL        -> src-tauri/target/release/
#      alongside offspring.exe (Inno [Files] picks it up from there)
#   4. build-msix.ps1                -> installer/msix/dist/OffspringShellExt.msix + .cer
#   5. iscc.exe installer/offspring.iss -> installer/dist/Offspring-Setup-<ver>.exe
#
# Version is read from package.json unless you pass -Version.
#
# Prerequisites (same as CI):
#   Node 20+, Rust stable, Windows 10 SDK, Inno Setup 6 (iscc.exe on PATH
#   or at default location). MSIX cert is auto-generated on first run.

[CmdletBinding()]
param(
    [string]$Version,
    [switch]$SkipInstall,   # skip `npm ci` (faster on repeat builds)
    [switch]$OpenOutput     # open Explorer on installer/dist/ when done
)

$ErrorActionPreference = "Stop"
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")

Push-Location $repoRoot
try {
    if (-not $Version) {
        $pkg = Get-Content (Join-Path $repoRoot "package.json") -Raw | ConvertFrom-Json
        $Version = $pkg.version
    }
    Write-Host ""
    Write-Host "============================================================" -ForegroundColor Cyan
    Write-Host " Offspring local release build - $Version" -ForegroundColor Cyan
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
    Write-Host ""
    Write-Host "[4/5] build-msix.ps1 (version $Version.0)..." -ForegroundColor Yellow
    pwsh (Join-Path $repoRoot "installer\msix\build-msix.ps1") -Version "$Version.0"
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

    Write-Host ""
    Write-Host "============================================================" -ForegroundColor Green
    Write-Host " Build OK" -ForegroundColor Green
    Write-Host "============================================================" -ForegroundColor Green
    Write-Host "  Installer: $installer" -ForegroundColor Green
    $size = (Get-Item $installer).Length / 1MB
    Write-Host ("  Size:      {0:N2} MB" -f $size) -ForegroundColor Green
    Write-Host ""
    Write-Host "Install it locally to test, then say 'push' to publish." -ForegroundColor Cyan

    if ($OpenOutput) {
        Start-Process (Split-Path $installer -Parent)
    }
} finally {
    Pop-Location
}
