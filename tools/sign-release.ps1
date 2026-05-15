# Sign the release installer(s) with minisign.
#
# Run AFTER `tools\build-release.ps1 -Release`. Produces `.minisig`
# sidecars that the in-app updater (standard build) fetches alongside
# the .exe and verifies against the pinned public key in
# `src-tauri/src/updates.rs:UPDATE_MINISIGN_PUBKEY`. Without the
# sidecar, every existing install will refuse to update (which is the
# point — that's what makes the unsigned-release scenario detectable).
#
# As of 0.4.4, every release ships TWO installers — standard +
# studio — and both need .minisig sidecars. Studio doesn't have an
# in-app updater (the code is compiled out), but the signature still
# lets a manual downloader verify the binary, and it keeps the asset
# layout symmetric on the GitHub releases page.
#
# Default behaviour (no -InstallerPath): finds and signs BOTH
# installers based on package.json's version. Pass -InstallerPath
# explicitly to sign just one (back-compat for partial re-signs).
#
# The private key lives at `installer\.minisign\offspring.key` by
# default (gitignored). Override with `-KeyPath` if you keep it
# elsewhere — e.g. on a removable drive that you mount only when
# signing. See `RELEASING.md` for the full flow.
#
# Verification (do this every time before publishing):
#
#   minisign -Vm installer\dist\Offspring-Setup-X.Y.Z.exe `
#            -p installer\.minisign\offspring.pub
#   minisign -Vm installer\dist\Offspring-Studio-Setup-X.Y.Z.exe `
#            -p installer\.minisign\offspring.pub
#
# Outputs the paths to every .minisig on success.

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

    # Path to a single installer to sign. When omitted, the script
    # signs BOTH the standard + studio installers for the current
    # package.json version. Pass this explicitly only when you need a
    # partial re-sign (e.g. you rebuilt just the studio variant).
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

# Build the list of installers to sign. If the caller passed an
# explicit path we honour it (single-file mode). Otherwise we pick
# up BOTH variants from package.json's version.
$installers = @()
if ($InstallerPath) {
    $installers += (Resolve-Path -LiteralPath $InstallerPath).Path
} else {
    $pkg = Get-Content (Join-Path $repoRoot "package.json") -Raw | ConvertFrom-Json
    $version = $pkg.version
    $standardPath = Join-Path $repoRoot "installer\dist\Offspring-Setup-$version.exe"
    $studioPath   = Join-Path $repoRoot "installer\dist\Offspring-Studio-Setup-$version.exe"
    foreach ($p in @($standardPath, $studioPath)) {
        if (-not (Test-Path $p)) {
            throw @"
Installer not found at $p.

Both standard and studio installers must exist before signing. Run
    tools\build-release.ps1 -Version $version
to produce them, then re-run this script.

If you intentionally rebuilt only one variant, pass -InstallerPath
explicitly to sign just that one.
"@
        }
        $installers += (Resolve-Path -LiteralPath $p).Path
    }
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

# Pre-flight: refuse if any target has an existing .minisig and
# -Force wasn't passed. We check all sidecars BEFORE signing any so
# we don't half-sign a release (one valid + one stale).
$sigPaths = $installers | ForEach-Object { "$_.minisig" }
if (-not $Force) {
    $existing = $sigPaths | Where-Object { Test-Path $_ }
    if ($existing) {
        $list = ($existing | ForEach-Object { "    $_" }) -join "`n"
        throw @"
Signature file(s) already exist:
$list

Re-signing a different build under the same name produces a
mismatch on the user side ("signature did not verify"). If you
genuinely want to re-sign — e.g. you tweaked the comment that goes
into the signature — pass -Force, or delete the existing .minisig
file(s) first.
"@
    }
}
# -Force path: clear stale sidecars up-front so a half-success
# (signed standard but minisign failed on studio) leaves a coherent
# state — both stale or both fresh.
foreach ($sig in $sigPaths) {
    if (Test-Path $sig) {
        Remove-Item -LiteralPath $sig -Force
    }
}

Write-Host ""
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host " Signing release installer(s)" -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan
foreach ($p in $installers) {
    Write-Host "  Installer: $p" -ForegroundColor Cyan
}
Write-Host "  Key:       $KeyPath" -ForegroundColor Cyan
Write-Host ""
Write-Host "minisign will prompt for the key password ONCE per file." -ForegroundColor DarkGray
Write-Host "(Same passphrase for both — minisign doesn't cache it across invocations.)" -ForegroundColor DarkGray
Write-Host ""

# `-Sm` signs the file in-place (writes <file>.minisig next to it).
# `-c` and `-t` set the trusted/untrusted comments — the trusted
# comment is signed and visible to verifiers; we put a build-id in it
# so a sidecar can be tied back to a specific build attempt.
$signed = @()
foreach ($p in $installers) {
    $buildId = "offspring-{0}-{1}" -f `
        (Split-Path $p -Leaf), `
        (Get-Date -Format "yyyyMMdd-HHmmss")
    $variant = if ($p -match 'Offspring-Studio-') { "studio" } else { "standard" }

    Write-Host "--- Signing $variant : $(Split-Path $p -Leaf) ---" -ForegroundColor Yellow
    & $minisign.Source -Sm $p -s $KeyPath `
        -c "Offspring $variant release build" `
        -t $buildId
    if ($LASTEXITCODE -ne 0) {
        throw "minisign signing failed (exit $LASTEXITCODE) on $p"
    }
    $sig = "$p.minisig"
    if (-not (Test-Path $sig)) {
        throw "minisign reported success but $sig wasn't produced"
    }
    $signed += $sig
    Write-Host ""
}

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

Write-Host ""
Write-Host "============================================================" -ForegroundColor Green
Write-Host " Signed" -ForegroundColor Green
Write-Host "============================================================" -ForegroundColor Green
foreach ($sig in $signed) {
    Write-Host "  Signature: $sig" -ForegroundColor Green
}
Write-Host ""
Write-Host "Verify locally (do this EVERY release before publishing):" -ForegroundColor Yellow
foreach ($p in $installers) {
    if (Test-Path $pubPath) {
        Write-Host "  minisign -Vm `"$p`" -p `"$pubPath`"" -ForegroundColor DarkGray
    } else {
        Write-Host "  minisign -Vm `"$p`" -p <path-to-offspring.pub>" -ForegroundColor DarkGray
    }
}
Write-Host ""
Write-Host "Then attach BOTH files for EACH variant to the GitHub release:" -ForegroundColor Yellow
foreach ($p in $installers) {
    Write-Host "  $p" -ForegroundColor DarkGray
    Write-Host "  $p.minisig" -ForegroundColor DarkGray
}
Write-Host ""
Write-Host "Without the .minisig, every existing standard install refuses to update." -ForegroundColor Yellow
Write-Host "Studio installs don't run the in-app updater, but the sidecar is" -ForegroundColor DarkGray
Write-Host "still useful for manual verification of the downloaded .exe." -ForegroundColor DarkGray
