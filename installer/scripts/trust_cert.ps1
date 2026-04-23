# Imports the shell-extension signing certificate into
# Cert:\LocalMachine\TrustedPeople so Add-AppxPackage (invoked later
# by the Settings toggle) can validate the MSIX signature. Must run
# as admin — the installer enforces that with PrivilegesRequired=admin.
#
# Logs success/failure to %ProgramData%\Offspring-install.log so a
# silent failure at install time is diagnosable afterwards.

param(
    [Parameter(Mandatory)][string]$CerPath,
    [string]$LogPath = (Join-Path $env:ProgramData 'Offspring-install.log')
)

$ts = (Get-Date).ToString('o')
try {
    if (-not (Test-Path $CerPath)) { throw "cert not found at $CerPath" }
    Import-Certificate -FilePath $CerPath `
        -CertStoreLocation Cert:\LocalMachine\TrustedPeople `
        -ErrorAction Stop | Out-Null
    "[$ts] trust_cert OK: imported $CerPath" |
        Out-File -FilePath $LogPath -Append -Encoding utf8
} catch {
    "[$ts] trust_cert FAIL: $($_.Exception.Message)" |
        Out-File -FilePath $LogPath -Append -Encoding utf8
    exit 1
}
