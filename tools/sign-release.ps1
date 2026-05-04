# Sign the release installer with minisign.
#
# Run AFTER `tools\build-release.ps1 -Release`. Produces a `.minisig`
# sidecar that the in-app updater fetches alongside the .exe and
# verifies against the pinned public key in
# `src-tauri/src/updates.rs:UPDATE_MINISIGN_PUBKEY`. Without the
# sidecar, every existing install will refuse to update (which is the
# point — that's what makes the unsigned-release scenario detectable).
#
# The private key lives at `installer\.minisign\offspring.key` by
# default (gitignored). Override with `-KeyPath` if you keep it
# elsewhere — e.g. on a removable drive that you mount only when
# signing. See `RELEASING.md` for the full flow.
#
# Verification step (do this every time before publishing):
#
#   minisign -Vm installer\dist\Offspring-Setup-X.Y.Z.exe `
#            -p installer\.minisign\offspring.pub
#
# Outputs the path to the .minisig on success.

[CmdletBinding()]
param(
    # Path to the private key file. Resolution order:
    #   1. -KeyPath argument (explicit override).
    #   2. $env:OFFSPRING_MINISIGN_KEY environment variable. Set this
    #      in your PowerShell profile if you keep the key outside the
    #      repo (recommended — gitignore is one mistake away from
    #      commit; a path outside the repo is structurally safe). E.g.
    #          $env:OFFSPRING_MINISIGN_KEY = "C:\Users\You\installer\.minisign\offspring.key"
    #   3. installer\.minisign\offspring.key inside the repo (the
    #      default the gitignore protects).
    [string]$KeyPath,

    # Path to the installer to sign. When omitted, defaults to
    # `installer\dist\Offspring-Setup-<package.json version>.exe` —
    # i.e. whatever `build-release.ps1 -Release` just produced.
    [string]$InstallerPath,

    # Force re-signing even if a .minisig already exists. Off by
    # default to prevent accidentally re-signing a build whose bytes
    # have changed since the last signature was made (which would
    # silently mismatch on the user side).
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")

if (-not $KeyPath) {
    if ($env:OFFSPRING_MINISIGN_KEY) {
        $KeyPath = $env:OFFSPRING_MINISIGN_KEY
    } else {
        $KeyPath = (Join-Path $repoRoot "installer\.minisign\offspring.key")
    }
}

# Resolve the installer path from package.json's version when the
# caller didn't pass an explicit one. This lets the script work as a
# zero-argument follow-up to build-release.ps1.
if (-not $InstallerPath) {
    $pkg = Get-Content (Join-Path $repoRoot "package.json") -Raw | ConvertFrom-Json
    $version = $pkg.version
    $InstallerPath = Join-Path $repoRoot "installer\dist\Offspring-Setup-$version.exe"
}
$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path

if (-not (Test-Path $InstallerPath)) {
    throw "Installer not found at $InstallerPath. Run tools\build-release.ps1 -Release first."
}
if (-not (Test-Path $KeyPath)) {
    throw @"
Private key not found at $KeyPath.

Generate one with:
    mkdir installer\.minisign -Force
    minisign -G -p installer\.minisign\offspring.pub -s installer\.minisign\offspring.key

Then paste the public-key file's second line into
src-tauri/src/updates.rs:UPDATE_MINISIGN_PUBKEY and back the private
key up offline. See RELEASING.md for details.
"@
}

# Locate minisign.exe. winget installs it under
# %LOCALAPPDATA%\Microsoft\WinGet\Links\ which is on PATH for new
# shells but might not be on PATH in the current one if winget just
# ran. Fall back to PATH search; winget's links shim handles forwarding.
$minisign = Get-Command minisign -ErrorAction SilentlyContinue
if (-not $minisign) {
    throw @"
minisign.exe not found on PATH.

Install it with:
    winget install jedisct1.minisign

Close and reopen the terminal so PATH picks up the new exe, then re-run.
"@
}

$sigPath = "$InstallerPath.minisig"
if ((Test-Path $sigPath) -and -not $Force) {
    throw @"
Signature file already exists at:
    $sigPath

Re-signing a different build under the same name produces a
mismatch on the user side ("signature did not verify"). If you
genuinely want to re-sign — e.g. you tweaked the comment that goes
into the signature — pass -Force, or delete the existing .minisig
first.
"@
}
if (Test-Path $sigPath) {
    Remove-Item -LiteralPath $sigPath -Force
}

Write-Host ""
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host " Signing release installer" -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "  Installer: $InstallerPath" -ForegroundColor Cyan
Write-Host "  Key:       $KeyPath" -ForegroundColor Cyan
Write-Host ""
Write-Host "minisign will prompt for the key password..." -ForegroundColor DarkGray

# `-Sm` signs the file in-place (writes <file>.minisig next to it).
# `-c` and `-t` set the trusted/untrusted comments — the trusted
# comment is signed and visible to verifiers; we put a build-id in it
# so a sidecar can be tied back to a specific build attempt.
$buildId = "offspring-{0}-{1}" -f `
    (Split-Path $InstallerPath -Leaf), `
    (Get-Date -Format "yyyyMMdd-HHmmss")

& $minisign.Source -Sm $InstallerPath -s $KeyPath `
    -c "Offspring release build" `
    -t $buildId

if ($LASTEXITCODE -ne 0) {
    throw "minisign signing failed (exit $LASTEXITCODE)"
}
if (-not (Test-Path $sigPath)) {
    throw "minisign reported success but $sigPath wasn't produced"
}

Write-Host ""
Write-Host "============================================================" -ForegroundColor Green
Write-Host " Signed" -ForegroundColor Green
Write-Host "============================================================" -ForegroundColor Green
Write-Host "  Signature: $sigPath" -ForegroundColor Green
Write-Host ""
Write-Host "Verify locally (do this EVERY release before publishing):" -ForegroundColor Yellow
# Look for the matching public key alongside the private key — that's
# where `minisign -G` places it by default. Falls back to the in-repo
# location if the key was loaded from elsewhere (env var) but no .pub
# sits next to it.
$pubCandidate = $KeyPath -replace '\.key$', '.pub'
$pubPath = if (Test-Path $pubCandidate) {
    $pubCandidate
} else {
    Join-Path $repoRoot "installer\.minisign\offspring.pub"
}
if (Test-Path $pubPath) {
    Write-Host "  minisign -Vm `"$InstallerPath`" -p `"$pubPath`"" -ForegroundColor DarkGray
} else {
    Write-Host "  minisign -Vm `"$InstallerPath`" -p <path-to-offspring.pub>" -ForegroundColor DarkGray
}
Write-Host ""
Write-Host "Then attach BOTH files to the GitHub release:" -ForegroundColor Yellow
Write-Host "  $InstallerPath" -ForegroundColor DarkGray
Write-Host "  $sigPath" -ForegroundColor DarkGray
Write-Host ""
Write-Host "Without the .minisig, every existing install refuses to update." -ForegroundColor Yellow
