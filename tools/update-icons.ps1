# Regenerate every icon target from the master Offspring_Icon source set.
#
# Source of truth: Offspring_Icon\Offspring_Icon-1024x1024@1x.png
# Resizes that master to every required pixel size via System.Drawing's
# HighQualityBicubic interpolation, writes them to the right targets,
# and rebuilds a multi-resolution icon.ico (16/24/32/48/64/128/256).
#
# Targets (Windows-only paths — iOS/Android trees are not used by this
# Windows build and are left alone):
#   src-tauri\icons\32x32.png  64x64.png  128x128.png  128x128@2x.png
#   src-tauri\icons\icon.png             (512x512)
#   src-tauri\icons\icon.ico             (multi-res)
#   src-tauri\icons\Square{30,44,71,89,107,142,150,284,310}x*Logo.png
#   src-tauri\icons\StoreLogo.png        (50x50)
#   installer\msix\stage\Assets\Square150x150Logo.png
#   installer\msix\stage\Assets\Square44x44Logo.png
#   installer\msix\stage\Assets\StoreLogo.png
#   static\favicon.png                   (128x128 — matches existing)
#
# Re-run safely. Source PNGs are read-only inputs.

[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")
# Glob for whichever 1024×1024 PNG lives in icon\ — naming has varied
# over time (Offspring_Icon-…, Offspring_Logo_v08-iOS-Default-…, now
# Offspring_Logo_v08-…). Anything ending in `-1024x1024@1x.png` counts
# as the master. If multiple match we pick the most recently modified,
# which lines up with the "I just dropped a revised icon" workflow.
$iconDir = Join-Path $repoRoot "icon"
$candidates = Get-ChildItem -Path $iconDir -Filter "*-1024x1024@1x.png" -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending
if (-not $candidates) {
    throw "No master icon found — expected a *-1024x1024@1x.png in $iconDir"
}
$source = $candidates[0].FullName
Write-Host "Master: $source" -ForegroundColor Cyan

# Load the master once. We resize from the largest available so every
# target gets clean downscaling rather than chained re-downscales.
$master = [System.Drawing.Image]::FromFile($source)

function Save-Resized {
    param([int]$Size, [string]$Dest)
    $bmp = New-Object System.Drawing.Bitmap $Size, $Size
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode     = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.PixelOffsetMode   = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)
    $g.DrawImage($master, 0, 0, $Size, $Size)
    $g.Dispose()
    $destDir = Split-Path $Dest -Parent
    if (-not (Test-Path $destDir)) { New-Item -ItemType Directory -Force -Path $destDir | Out-Null }
    $bmp.Save($Dest, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    Write-Host "  $Size x $Size  -> $Dest" -ForegroundColor DarkGray
}

# (size, relative path) pairs.
$targets = @(
    @{ size=32;  path="src-tauri\icons\32x32.png" },
    @{ size=64;  path="src-tauri\icons\64x64.png" },
    @{ size=128; path="src-tauri\icons\128x128.png" },
    @{ size=256; path="src-tauri\icons\128x128@2x.png" },
    @{ size=512; path="src-tauri\icons\icon.png" },

    @{ size=30;  path="src-tauri\icons\Square30x30Logo.png" },
    @{ size=44;  path="src-tauri\icons\Square44x44Logo.png" },
    @{ size=71;  path="src-tauri\icons\Square71x71Logo.png" },
    @{ size=89;  path="src-tauri\icons\Square89x89Logo.png" },
    @{ size=107; path="src-tauri\icons\Square107x107Logo.png" },
    @{ size=142; path="src-tauri\icons\Square142x142Logo.png" },
    @{ size=150; path="src-tauri\icons\Square150x150Logo.png" },
    @{ size=284; path="src-tauri\icons\Square284x284Logo.png" },
    @{ size=310; path="src-tauri\icons\Square310x310Logo.png" },
    @{ size=50;  path="src-tauri\icons\StoreLogo.png" },

    @{ size=44;  path="installer\msix\stage\Assets\Square44x44Logo.png" },
    @{ size=150; path="installer\msix\stage\Assets\Square150x150Logo.png" },
    @{ size=50;  path="installer\msix\stage\Assets\StoreLogo.png" },

    @{ size=128; path="static\favicon.png" }
)

Write-Host "Resizing master to PNG targets..." -ForegroundColor Yellow
foreach ($t in $targets) {
    Save-Resized -Size $t.size -Dest (Join-Path $repoRoot $t.path)
}

# --- icon.ico ---------------------------------------------------------
# Build a multi-resolution Windows .ico. Each entry in the ICO directory
# is a small fixed header followed by the PNG bytes (Vista+ supports
# embedded PNG entries — much cleaner than packing BMPs with masks, and
# every modern Explorer/Taskbar consumer reads them fine).
Write-Host "Building icon.ico..." -ForegroundColor Yellow
$icoSizes = @(16, 24, 32, 48, 64, 128, 256)
$icoPath  = Join-Path $repoRoot "src-tauri\icons\icon.ico"

# Render each size to an in-memory PNG buffer.
$entries = @()
foreach ($sz in $icoSizes) {
    $bmp = New-Object System.Drawing.Bitmap $sz, $sz
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.SmoothingMode     = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
    $g.PixelOffsetMode   = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    $g.Clear([System.Drawing.Color]::Transparent)
    $g.DrawImage($master, 0, 0, $sz, $sz)
    $g.Dispose()
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    $entries += [pscustomobject]@{ Size = $sz; Bytes = $ms.ToArray() }
    $ms.Dispose()
}

# ICONDIR (6 bytes) + ICONDIRENTRY (16 bytes per image) + image data.
$out = New-Object System.IO.MemoryStream
$bw  = New-Object System.IO.BinaryWriter $out
$bw.Write([uint16]0)            # reserved
$bw.Write([uint16]1)            # type 1 = .ICO
$bw.Write([uint16]$entries.Count)

# Compute payload offsets (after 6-byte header + N*16-byte directory).
$offset = 6 + ($entries.Count * 16)
foreach ($e in $entries) {
    # Width/height: a single byte per dimension, with 0 meaning 256.
    $w = if ($e.Size -ge 256) { 0 } else { $e.Size }
    $h = $w
    $bw.Write([byte]$w)
    $bw.Write([byte]$h)
    $bw.Write([byte]0)              # color count (0 = no palette / truecolor)
    $bw.Write([byte]0)              # reserved
    $bw.Write([uint16]1)            # color planes
    $bw.Write([uint16]32)           # bits per pixel
    $bw.Write([uint32]$e.Bytes.Length)
    $bw.Write([uint32]$offset)
    $offset += $e.Bytes.Length
}
foreach ($e in $entries) {
    $bw.Write($e.Bytes)
}
$bw.Flush()
[System.IO.File]::WriteAllBytes($icoPath, $out.ToArray())
$bw.Dispose()
$out.Dispose()
Write-Host "  $($entries.Count) sizes -> $icoPath" -ForegroundColor DarkGray

$master.Dispose()

Write-Host ""
Write-Host "Icons updated." -ForegroundColor Green
