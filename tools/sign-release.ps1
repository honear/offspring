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

    # Explicit list of files to sign. Overrides version-based discovery
    # entirely — useful from CI where the artifact set isn't necessarily
    # the standard+studio Windows pair (e.g. a single .dmg on macOS).
    [string[]]$Files,

    # Which signing tool to invoke.
    #   - minisign       : the reference C implementation (jedisct1/minisign).
    #                      Interactive passphrase prompt via TTY. The local
    #                      default; what's installed via winget on the dev box.
    #   - offspring-sign : custom in-tree Rust binary (tools/offspring-sign).
    #                      Reads MINISIGN_PASSWORD env var directly via the
    #                      `minisign` crate's API — no TTY, no stdin pipe,
    #                      no CLI password-parsing nightmares. The CI mode.
    #
    # The previous rsign2 attempt is gone: rsign2's CLI has no
    # stdin-or-env password input (its `-W` flag means "passwordless
    # key", not "read password from stdin"), so it's structurally
    # unsuitable for CI without a pseudo-TTY.
    [ValidateSet('minisign','offspring-sign')]
    [string]$Tool = 'minisign',

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

# Build the list of installers to sign. Resolution order:
#   1. -Files (explicit array, used from CI for arbitrary artifact sets)
#   2. -InstallerPath (single-file local override)
#   3. version-based discovery for both Windows installers (default)
$installers = @()
if ($Files -and $Files.Count -gt 0) {
    foreach ($f in $Files) {
        if (-not (Test-Path -LiteralPath $f)) {
            throw "File to sign not found: $f"
        }
        $installers += (Resolve-Path -LiteralPath $f).Path
    }
} elseif ($InstallerPath) {
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

# Locate the signing tool. minisign (default) is the C reference impl
# installed via winget; offspring-sign (CI mode) is the in-tree Rust
# binary at tools/offspring-sign which uses the `minisign` crate
# directly. Both produce byte-compatible .minisig files.
$toolBin = Get-Command $Tool -ErrorAction SilentlyContinue
if (-not $toolBin) {
    if ($Tool -eq 'minisign') {
        throw @"
minisign.exe not found on PATH.

Install it with:
    winget install jedisct1.minisign

Close and reopen the terminal so PATH picks up the new exe, then re-run.
"@
    } else {
        throw @"
offspring-sign not found on PATH.

Build + install it from the repo root with:
    cargo install --path tools/offspring-sign --locked

In CI, the workflow installs it before invoking this script.
"@
    }
}

# CI mode pre-flight: offspring-sign reads the passphrase from
# $env:MINISIGN_PASSWORD. Refuse to start if it's missing rather than
# pass an empty password and get a confusing decrypt error.
if ($Tool -eq 'offspring-sign' -and [string]::IsNullOrEmpty($env:MINISIGN_PASSWORD)) {
    throw "Tool 'offspring-sign' requires `$env:MINISIGN_PASSWORD to be set."
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
if ($Tool -eq 'minisign') {
    Write-Host "minisign will prompt for the key password ONCE per file." -ForegroundColor DarkGray
    Write-Host "(Same passphrase across files — minisign doesn't cache it across invocations.)" -ForegroundColor DarkGray
} else {
    Write-Host "offspring-sign mode: passphrase read from `$env:MINISIGN_PASSWORD." -ForegroundColor DarkGray
}
Write-Host ""

# Sign each installer. The two tools take different argument shapes but
# produce the same .minisig output. Trusted comment carries a build-id
# so a sidecar can be tied back to a specific build attempt.
$signed = @()
foreach ($p in $installers) {
    $leaf = Split-Path $p -Leaf
    $buildId = "offspring-{0}-{1}" -f $leaf, (Get-Date -Format "yyyyMMdd-HHmmss")
    $variant = if ($leaf -match 'Offspring-Studio-') {
        "studio"
    } elseif ($leaf -match '\.dmg$') {
        "macos"
    } else {
        "standard"
    }
    $untrusted = "Offspring $variant release build"

    Write-Host "--- Signing $variant : $leaf ---" -ForegroundColor Yellow
    if ($Tool -eq 'minisign') {
        # `-Sm` signs the file in-place (writes <file>.minisig next to it).
        & $toolBin.Source -Sm $p -s $KeyPath -c $untrusted -t $buildId
        if ($LASTEXITCODE -ne 0) {
            throw "minisign signing failed (exit $LASTEXITCODE) on $p"
        }
    } else {
        # offspring-sign reads MINISIGN_PASSWORD straight from the
        # environment, so we just invoke it with the right args and
        # check the exit code. No stdin pipes, no encoding gotchas.
        $sigPath = "$p.minisig"
        Write-Host "  (password length: $($env:MINISIGN_PASSWORD.Length))" -ForegroundColor DarkGray
        & $toolBin.Source $KeyPath $p $sigPath $buildId $untrusted
        if ($LASTEXITCODE -ne 0) {
            throw "offspring-sign failed (exit $LASTEXITCODE) on $p"
        }
    }

    $sig = "$p.minisig"
    if (-not (Test-Path $sig)) {
        throw "$Tool reported success but $sig wasn't produced"
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
