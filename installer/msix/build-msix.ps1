# Builds and signs the Offspring shell-extension sparse MSIX package.
#
# Inputs (relative to repo root):
#   installer/msix/AppxManifest.xml
#   src-tauri/icons/StoreLogo.png + Square150x150Logo + Square44x44Logo
#   <cargo target>/release/offspring_shell_ext.dll  (built by cargo in
#                                                    shell-ext/ before
#                                                    this script runs)
#
# Outputs:
#   installer/msix/dist/OffspringShellExt.msix            (signed)
#   installer/msix/dist/OffspringShellExt.cer             (public cert)
#
# The .cer is consumed by the installer — it goes into the user's
# `Cert:\CurrentUser\TrustedPeople` store when they toggle the modern
# right-click menu on. Without it Windows refuses to register the MSIX.
#
# Certificate lifecycle:
#   - First build on a fresh machine creates $certPath (a .pfx) if it
#     doesn't already exist. Subsequent builds reuse it.
#   - The .pfx is kept OUT of the repo (see .gitignore).
#   - The public .cer is safe to ship with the installer — it's just a
#     trust anchor, not a signing key.

[CmdletBinding()]
param(
    [string]$Configuration = "Release",
    [string]$Version       = "0.2.0.0",
    # Default to whatever's in $env:OFFSPRING_PFX_PASSWORD so secrets stay
    # out of the source tree and out of process listings. Falls back to
    # the legacy dev password ONLY when no env var is set; release builds
    # should always export OFFSPRING_PFX_PASSWORD before invoking this.
    [string]$PfxPassword   = $(if ($env:OFFSPRING_PFX_PASSWORD) { $env:OFFSPRING_PFX_PASSWORD } else { "offspring-dev" })
)

$ErrorActionPreference = "Stop"

if ($PfxPassword -eq "offspring-dev") {
    Write-Warning "Using the built-in dev PFX password. Set `$env:OFFSPRING_PFX_PASSWORD before publishing a release build."
}

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$msixDir  = Join-Path $repoRoot "installer\msix"
$distDir  = Join-Path $msixDir "dist"
$stageDir = Join-Path $msixDir "stage"
$iconsSrc = Join-Path $repoRoot "src-tauri\icons"
# CI / hardware-token path: when $env:OFFSPRING_PFX_PATH is set, the
# script reads the signing PFX from that path (e.g. a CI secret cache,
# a TPM-backed key on a build machine) and never touches the in-repo
# `.cert` directory. Without the override we fall back to the legacy
# dev location so local "first build on a fresh checkout" still works.
$certDir  = Join-Path $msixDir ".cert"
if ($env:OFFSPRING_PFX_PATH) {
    $certPath = $env:OFFSPRING_PFX_PATH
    Write-Host "Using PFX from `$env:OFFSPRING_PFX_PATH: $certPath"
} else {
    $certPath = Join-Path $certDir "offspring-shellext.pfx"
}
$certCer  = Join-Path $distDir "OffspringShellExt.cer"
$msixOut  = Join-Path $distDir "OffspringShellExt.msix"

# -- tool discovery ----------------------------------------------------
# MakeAppx / SignTool live under the Windows 10 SDK. Look up the newest
# installed SDK and pin paths so a missing SDK fails fast and loud.
$sdkRoot  = "C:\Program Files (x86)\Windows Kits\10\bin"
if (-not (Test-Path $sdkRoot)) {
    throw "Windows 10 SDK not found at $sdkRoot. Install the Win10 SDK via Visual Studio Installer or standalone."
}
$sdkVer   = Get-ChildItem $sdkRoot -Directory |
            Where-Object { $_.Name -match '^10\.' } |
            Sort-Object Name -Descending |
            Select-Object -First 1
if (-not $sdkVer) { throw "No 10.x SDK found under $sdkRoot." }
$makeAppx = Join-Path $sdkVer.FullName "x64\MakeAppx.exe"
$signTool = Join-Path $sdkVer.FullName "x64\SignTool.exe"
foreach ($t in @($makeAppx, $signTool)) {
    if (-not (Test-Path $t)) { throw "Missing SDK tool: $t" }
}

# -- cert (create once, reuse forever) --------------------------------
New-Item -ItemType Directory -Force -Path $certDir, $distDir, $stageDir | Out-Null

if (-not (Test-Path $certPath)) {
    if ($env:OFFSPRING_PFX_PATH) {
        # Caller pointed us at a specific PFX (CI secret, hardware
        # token, etc.) but it doesn't exist. Don't silently create a
        # new self-signed dev cert at that location — that would be a
        # surprising side effect. Surface the misconfiguration loudly.
        throw "OFFSPRING_PFX_PATH is set to '$certPath' but no file exists there. Provision the PFX before running this script, or unset the env var to fall back to the dev cert."
    }
    Write-Host "Creating self-signed certificate (CN=Second March)..."
    # Cap the dev cert at 2 years so a leaked PFX has a hard horizon.
    # Long enough for normal release cycles, short enough to limit blast
    # radius. Producers of real release builds should replace this with
    # a code-signing cert from a trusted CA + hardware-token storage.
    $notBefore = Get-Date
    $notAfter  = $notBefore.AddYears(2)
    $cert = New-SelfSignedCertificate `
        -Type Custom `
        -Subject "CN=Second March" `
        -KeyUsage DigitalSignature `
        -FriendlyName "Offspring Shell Ext Dev Cert" `
        -CertStoreLocation "Cert:\CurrentUser\My" `
        -NotBefore $notBefore `
        -NotAfter $notAfter `
        -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
    $pwd = ConvertTo-SecureString -String $PfxPassword -Force -AsPlainText
    Export-PfxCertificate -Cert $cert -FilePath $certPath -Password $pwd | Out-Null
    Export-Certificate    -Cert $cert -FilePath $certCer                  | Out-Null
    Write-Host "  PFX: $certPath"
    Write-Host "  CER: $certCer"
    Write-Host ("  Valid: {0:yyyy-MM-dd} → {1:yyyy-MM-dd}" -f $notBefore, $notAfter)
} else {
    Write-Host "Reusing existing cert: $certPath"
    # Keep the public .cer in dist/ up to date in case the distribution
    # folder was wiped between builds.
    if (-not (Test-Path $certCer)) {
        $pwd = ConvertTo-SecureString -String $PfxPassword -Force -AsPlainText
        $pfx = Get-PfxCertificate -FilePath $certPath
        Export-Certificate -Cert $pfx -FilePath $certCer | Out-Null
    }
}

# -- stage package contents --------------------------------------------
# Only the manifest + icons go into the MSIX. The DLL and offspring.exe
# live in the install directory (ExternalLocation) and are picked up at
# Add-AppxPackage time.
Remove-Item -Recurse -Force $stageDir -ErrorAction SilentlyContinue | Out-Null
New-Item -ItemType Directory -Force -Path "$stageDir\Assets" | Out-Null

Copy-Item (Join-Path $msixDir "AppxManifest.xml") $stageDir -Force

$logos = @(
    "StoreLogo.png",
    "Square150x150Logo.png",
    "Square44x44Logo.png"
)
foreach ($logo in $logos) {
    $src = Join-Path $iconsSrc $logo
    if (-not (Test-Path $src)) {
        throw "Missing icon $src — expected from Tauri icon generation."
    }
    Copy-Item $src (Join-Path $stageDir "Assets\$logo") -Force
}

# Rewrite the <Identity Version=""> in-place so tags drive the version
# without a second source of truth.
$manifestDst = Join-Path $stageDir "AppxManifest.xml"
(Get-Content $manifestDst -Raw) `
    -replace 'Version="\d+\.\d+\.\d+\.\d+"', ('Version="{0}"' -f $Version) |
    Set-Content $manifestDst -Encoding UTF8

# -- pack --------------------------------------------------------------
Remove-Item -Force $msixOut -ErrorAction SilentlyContinue
Write-Host "Packing MSIX..."
& $makeAppx pack /d $stageDir /p $msixOut /nv
if ($LASTEXITCODE -ne 0) { throw "MakeAppx failed ($LASTEXITCODE)" }

# -- sign --------------------------------------------------------------
Write-Host "Signing MSIX..."
& $signTool sign /fd SHA256 /a /f $certPath /p $PfxPassword $msixOut
if ($LASTEXITCODE -ne 0) { throw "SignTool failed ($LASTEXITCODE)" }

Write-Host ""
Write-Host "OK. Outputs:"
Write-Host "  $msixOut"
Write-Host "  $certCer"
