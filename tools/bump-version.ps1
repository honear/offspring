# Offspring version bumper.
#
# Modes (mutually exclusive):
#   default      "0.3.41"          -> "0.3.41-b0001"
#                "0.3.41-b0007"    -> "0.3.41-b0008"
#                Bumps the local build counter. Use this for every local
#                iteration build that isn't going out as a release.
#
#   -Release     "0.3.41-b0007"    -> "0.3.42"
#                "0.3.41"          -> "0.3.42"
#                Strips the build-counter suffix and bumps the patch
#                number. Use this when you're about to push to a GitHub
#                release: the resulting version is the one users will see.
#
#   -Set X.Y.Z[-bNNNN]             Explicit override. Skips both modes.
#
# Writes the new version into all five places it lives:
#   package.json
#   src-tauri/Cargo.toml          (only the [package] version line)
#   src-tauri/tauri.conf.json
#   shell-ext/Cargo.toml          (only the [package] version line)
#   installer/offspring.iss       (both AppVersion and AppVersionMsix)
#
# The MSIX manifest needs MAJOR.MINOR.BUILD.REVISION four-numeric, so we
# also derive that form: "0.3.41-b0007" -> "0.3.41.7", "0.3.41" -> "0.3.41.0".
# The .iss file gets this as a separate `AppVersionMsix` define so its
# `VersionInfoVersion=` line stays valid even when `AppVersion` carries
# a pre-release tag.
#
# Returns the new semver version string (for the caller to pipe into
# build-release.ps1 etc.).

[CmdletBinding(DefaultParameterSetName = "Bump")]
param(
    [Parameter(ParameterSetName = "Release")]
    [switch]$Release,

    [Parameter(ParameterSetName = "Set", Mandatory = $true)]
    [string]$Set
)

$ErrorActionPreference = "Stop"
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..")

# --- helpers ---------------------------------------------------------

function Get-CurrentVersion {
    $pkgPath = Join-Path $repoRoot "package.json"
    $pkg = Get-Content $pkgPath -Raw | ConvertFrom-Json
    return [string]$pkg.version
}

# Returns a hashtable with major, minor, patch, build (all strings;
# build is "" if the input had no -bNNNN suffix). Throws if the input
# isn't in our X.Y.Z[-bNNNN] form.
#
# Counter format note — the suffix is `bNNNN` (alphanumeric) rather than
# bare `NNNN`. Strict SemVer 2.0.0 forbids leading zeroes on numeric
# pre-release identifiers, so "0.3.41-0001" parses as invalid in
# Tauri's config validator (and `cargo publish` would reject it too).
# Prefixing with `b` makes the identifier alphanumeric, which has no
# leading-zero rule — and the 4-digit width still sorts correctly as a
# string ("b0001" < "b0002" < ... < "b9999").
function Split-Semver([string]$v) {
    if ($v -notmatch '^(\d+)\.(\d+)\.(\d+)(?:-b(\d+))?$') {
        throw "Not a recognized version string: $v"
    }
    return @{
        Major = $Matches[1]
        Minor = $Matches[2]
        Patch = $Matches[3]
        Build = if ($Matches[4]) { $Matches[4] } else { "" }
    }
}

function Get-MsixVersion([string]$semver) {
    $p = Split-Semver $semver
    $build = if ($p.Build -eq "") { 0 } else { [int]$p.Build }
    return "$($p.Major).$($p.Minor).$($p.Patch).$build"
}

# Replace `version = "..."` ONLY at the top of the [package] section.
# Cargo.toml dependencies often have version = "X" inside { } on the same
# line; those don't start at column 0, so a `^version\s*=` regex skips
# them. Belt-and-braces: we also section-track to refuse anything outside
# [package].
function Set-CargoPackageVersion([string]$path, [string]$newVersion) {
    $lines = Get-Content $path
    $inPackage = $false
    $replaced = $false
    $out = New-Object System.Collections.Generic.List[string]
    foreach ($line in $lines) {
        $trimmed = $line.TrimStart()
        if ($trimmed -match '^\[package\]') {
            $inPackage = $true
            $out.Add($line) | Out-Null
            continue
        }
        if ($trimmed -match '^\[' -and $trimmed -notmatch '^\[\[') {
            $inPackage = $false
        }
        if ($inPackage -and -not $replaced -and $line -match '^version\s*=\s*"[^"]*"') {
            $out.Add("version = `"$newVersion`"") | Out-Null
            $replaced = $true
        } else {
            $out.Add($line) | Out-Null
        }
    }
    if (-not $replaced) { throw "No [package].version line found in $path" }
    # Preserve trailing newline behavior of the original file
    $eol = if ((Get-Content $path -Raw) -match "`r`n") { "`r`n" } else { "`n" }
    [System.IO.File]::WriteAllText($path, ($out -join $eol) + $eol)
}

function Set-PackageJsonVersion([string]$path, [string]$newVersion) {
    # Targeted regex replace so we don't reformat the whole file (which
    # ConvertTo-Json on PowerShell 5.1 happily would).
    $content = Get-Content $path -Raw
    $pattern = '("version"\s*:\s*")[^"]+(")'
    if ($content -notmatch $pattern) { throw "version field not found in $path" }
    $new = $content -replace $pattern, "`${1}$newVersion`${2}"
    [System.IO.File]::WriteAllText($path, $new)
}

function Set-TauriConfigVersion([string]$path, [string]$newVersion) {
    $content = Get-Content $path -Raw
    $pattern = '("version"\s*:\s*")[^"]+(")'
    if ($content -notmatch $pattern) { throw "version field not found in $path" }
    $new = $content -replace $pattern, "`${1}$newVersion`${2}"
    [System.IO.File]::WriteAllText($path, $new)
}

function Set-IssVersion([string]$path, [string]$newVersion, [string]$msixVersion) {
    $content = Get-Content $path -Raw
    # AppVersion — already exists, just swap the value. We use -notmatch
    # to detect "field missing" rather than checking if the file changed
    # post-replace, because re-running with the SAME version would be a
    # no-op replacement that looks identical to "field missing" otherwise.
    $appVerPattern = '(#define\s+AppVersion\s+")[^"]+(")'
    if ($content -notmatch $appVerPattern) { throw "AppVersion define not found in $path" }
    $new = $content -replace $appVerPattern, "`${1}$newVersion`${2}"
    # AppVersionMsix — may not exist yet on first run. Insert it right
    # after AppVersion if missing; otherwise update the value.
    if ($new -match '#define\s+AppVersionMsix\s+"[^"]+"') {
        $new = $new -replace '(#define\s+AppVersionMsix\s+")[^"]+(")', "`${1}$msixVersion`${2}"
    } else {
        $new = $new -replace '(#define\s+AppVersion\s+"[^"]+"\s*(?:\r?\n))', "`${1}#define AppVersionMsix `"$msixVersion`"`r`n"
    }
    [System.IO.File]::WriteAllText($path, $new)
}

# --- decide new version ----------------------------------------------

$current = Get-CurrentVersion
$parts = Split-Semver $current
$baseCounter = if ($parts.Build -eq "") { $null } else { [int]$parts.Build }

switch ($PSCmdlet.ParameterSetName) {
    "Release" {
        # Strip suffix, bump patch.
        $newVersion = "$($parts.Major).$($parts.Minor).$([int]$parts.Patch + 1)"
    }
    "Set" {
        $null = Split-Semver $Set   # validates
        $newVersion = $Set
    }
    default {
        # Counter bump.
        $next = if ($null -eq $baseCounter) { 1 } else { $baseCounter + 1 }
        if ($next -gt 9999) { throw "Build counter would overflow 9999. Run -Release or pick an explicit -Set." }
        $newVersion = "{0}.{1}.{2}-b{3:D4}" -f $parts.Major, $parts.Minor, $parts.Patch, $next
    }
}

$msixVersion = Get-MsixVersion $newVersion

# --- write everything ------------------------------------------------

Write-Host "  $current -> $newVersion (msix: $msixVersion)" -ForegroundColor Cyan

Set-PackageJsonVersion  (Join-Path $repoRoot "package.json")            $newVersion
Set-CargoPackageVersion (Join-Path $repoRoot "src-tauri\Cargo.toml")    $newVersion
Set-TauriConfigVersion  (Join-Path $repoRoot "src-tauri\tauri.conf.json") $newVersion
Set-CargoPackageVersion (Join-Path $repoRoot "shell-ext\Cargo.toml")    $newVersion
Set-IssVersion          (Join-Path $repoRoot "installer\offspring.iss") $newVersion $msixVersion

# package-lock.json carries the root project's version field too. `npm
# install --package-lock-only` syncs it without touching node_modules,
# which is what the build script's `npm ci` step will check against.
# Skip silently if npm isn't on PATH (e.g. someone running this script
# in isolation on a non-dev shell).
$npm = Get-Command npm -ErrorAction SilentlyContinue
if ($npm) {
    Push-Location $repoRoot
    try {
        & $npm.Source install --package-lock-only --silent 2>&1 | Out-Null
    } finally {
        Pop-Location
    }
}

# Output the new version so callers can capture it via:
#   $v = & tools/bump-version.ps1
return $newVersion
