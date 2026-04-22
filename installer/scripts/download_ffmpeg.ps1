# Offspring — FFmpeg bootstrap
#
# Ensures ffmpeg.exe is available under %LOCALAPPDATA%\Offspring\ffmpeg\bin\.
# If it's already present (or on PATH), does nothing. Otherwise prompts the
# user and downloads the latest "essentials" build from gyan.dev.
#
# Run hidden by the Inno Setup installer (see offspring.iss).

$ErrorActionPreference = "Stop"

$Target    = Join-Path $env:LOCALAPPDATA "Offspring\ffmpeg"
$BinDir    = Join-Path $Target "bin"
$ExePath   = Join-Path $BinDir "ffmpeg.exe"

function Test-FfmpegOnPath {
    $cmd = Get-Command ffmpeg.exe -ErrorAction SilentlyContinue
    return [bool]$cmd
}

if (Test-Path $ExePath) {
    Write-Host "FFmpeg already installed: $ExePath"
    exit 0
}

if (Test-FfmpegOnPath) {
    Write-Host "FFmpeg found on PATH — skipping bundled download"
    exit 0
}

# Prompt the user (WinForms so it works even in runhidden context)
Add-Type -AssemblyName System.Windows.Forms
$prompt = [System.Windows.Forms.MessageBox]::Show(
    "Offspring needs FFmpeg to convert videos (~80 MB).`n`n" +
    "Download the latest LGPL build from gyan.dev now?`n" +
    "(You can also point Offspring at your own install later in Settings.)",
    "Offspring — Install FFmpeg",
    [System.Windows.Forms.MessageBoxButtons]::YesNo,
    [System.Windows.Forms.MessageBoxIcon]::Question)

if ($prompt -ne [System.Windows.Forms.DialogResult]::Yes) {
    Write-Host "User declined FFmpeg download"
    exit 0
}

# Permanent URL that always points to the latest essentials build
$Url     = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"
$TmpZip  = Join-Path $env:TEMP ("ffmpeg-offspring-" + [Guid]::NewGuid().ToString("N") + ".zip")
$TmpDir  = Join-Path $env:TEMP ("ffmpeg-offspring-extract-" + [Guid]::NewGuid().ToString("N"))

try {
    New-Item -ItemType Directory -Force -Path $Target | Out-Null
    New-Item -ItemType Directory -Force -Path $TmpDir | Out-Null

    Write-Host "Downloading $Url ..."
    Invoke-WebRequest -Uri $Url -OutFile $TmpZip -UseBasicParsing

    Write-Host "Extracting..."
    Expand-Archive -Path $TmpZip -DestinationPath $TmpDir -Force

    # gyan.dev ships a nested folder like ffmpeg-N.N.N-essentials_build/
    $nested = Get-ChildItem -Path $TmpDir -Directory | Select-Object -First 1
    if (-not $nested) { throw "Unexpected archive layout" }

    # Move/overwrite bin + presets directories
    foreach ($sub in @("bin", "presets", "doc")) {
        $src = Join-Path $nested.FullName $sub
        $dst = Join-Path $Target $sub
        if (Test-Path $src) {
            if (Test-Path $dst) { Remove-Item -Recurse -Force $dst }
            Move-Item -Path $src -Destination $dst
        }
    }
    # Also keep the LICENSE for LGPL compliance
    $license = Join-Path $nested.FullName "LICENSE"
    if (Test-Path $license) {
        Copy-Item -Path $license -Destination (Join-Path $Target "LICENSE") -Force
    }

    if (-not (Test-Path $ExePath)) {
        throw "ffmpeg.exe not present after extraction: expected $ExePath"
    }

    Write-Host "FFmpeg installed: $ExePath"
}
catch {
    Write-Warning "FFmpeg download failed: $($_.Exception.Message)"
    [System.Windows.Forms.MessageBox]::Show(
        "FFmpeg download failed:`n`n$($_.Exception.Message)`n`n" +
        "You can retry from Offspring → Settings, or point to your own FFmpeg install.",
        "Offspring — Install FFmpeg",
        [System.Windows.Forms.MessageBoxButtons]::OK,
        [System.Windows.Forms.MessageBoxIcon]::Warning) | Out-Null
    exit 0
}
finally {
    Remove-Item -Force -ErrorAction SilentlyContinue $TmpZip
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $TmpDir
}
