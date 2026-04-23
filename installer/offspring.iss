; Offspring — Inno Setup script
; Builds a single-file installer for the Tauri-compiled offspring.exe,
; prompts the user to download FFmpeg on first install if not already
; present, and registers first-run/cleanup hooks for SendTo shortcuts.
;
; Expected layout before compiling this script (relative to this file):
;   app/src-tauri/target/release/offspring.exe
;   app/installer/scripts/download_ffmpeg.ps1
;
; Compile with the Inno Setup compiler (iscc.exe or the Inno Setup IDE).

#define AppName      "Offspring"
#define AppVersion   "0.3.9"
#define AppPublisher "Second March"
#define AppExeName   "offspring.exe"
#define AppId        "{{D8E5C6BC-5F10-4B29-A8A9-7D4D1A3B9C22}"

; Resolve where Cargo dropped offspring.exe. When CARGO_TARGET_DIR is set in
; the environment (as it is on dev machines sharing a target dir across
; projects), use <CARGO_TARGET_DIR>\release. Otherwise fall back to the
; repo-local path that Tauri uses by default on CI.
#define CargoTargetDirEnv GetEnv("CARGO_TARGET_DIR")
#if CargoTargetDirEnv == ""
  #define BinDir "..\src-tauri\target\release"
#else
  #define BinDir CargoTargetDirEnv + "\release"
#endif

[Setup]
AppId={#AppId}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
UninstallDisplayIcon={app}\{#AppExeName}
OutputDir=dist
OutputBaseFilename=Offspring-Setup-{#AppVersion}
Compression=lzma2/ultra
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible
; Require admin so: (1) every install lands in the same scope (no more
; stacked per-user + admin copies), and (2) the shell-extension cert
; trust step below can populate LocalMachine\TrustedPeople, which is
; what Add-AppxPackage validates the MSIX signature against. Users
; without admin can't install — acceptable for this app's audience.
PrivilegesRequired=admin
WizardStyle=modern
DisableProgramGroupPage=yes
DisableDirPage=auto

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional options:"; Flags: unchecked

[Files]
Source: "{#BinDir}\{#AppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\DESIGN.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "scripts\download_ffmpeg.ps1"; DestDir: "{app}\scripts"; Flags: ignoreversion
Source: "scripts\trust_cert.ps1"; DestDir: "{app}\scripts"; Flags: ignoreversion
; Shell-ext DLL + signed sparse MSIX + public cert, consumed by the
; modern-menu Settings toggle (Add-AppxPackage runs against them from
; the app at toggle-on time). The CI workflow copies the DLL from
; shell-ext/target/release/ into {#BinDir} before iscc runs.
Source: "{#BinDir}\offspring_shell_ext.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "msix\dist\OffspringShellExt.msix"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "msix\dist\OffspringShellExt.cer"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Run]
; After install: download FFmpeg if missing (UI is provided by the PowerShell script)
Filename: "powershell.exe"; \
    Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\scripts\download_ffmpeg.ps1"""; \
    StatusMsg: "Checking FFmpeg..."; \
    Flags: runhidden waituntilterminated
; Trust the shell-extension signing cert machine-wide so the modern-menu
; toggle's Add-AppxPackage call doesn't fail with 0x800B0109 (untrusted
; root). Installer is admin-only, so no privilege check needed.
; Delegated to a dedicated script to avoid Inno-Setup's `}}` escaping
; gotcha (only `{{` escapes — a stray `}` slips through and breaks the
; embedded PowerShell). The script logs to %ProgramData% so silent
; failures at install time can be diagnosed later.
Filename: "powershell.exe"; \
    Parameters: "-NoProfile -ExecutionPolicy Bypass -File ""{app}\scripts\trust_cert.ps1"" -CerPath ""{app}\OffspringShellExt.cer"""; \
    StatusMsg: "Trusting shell-extension certificate..."; \
    Flags: runhidden waituntilterminated
; Seed default presets + populate SendTo shortcuts
Filename: "{app}\{#AppExeName}"; \
    Parameters: "first-run"; \
    Flags: runhidden waituntilterminated
; Optional: launch the app at the end
Filename: "{app}\{#AppExeName}"; \
    Description: "Launch {#AppName}"; \
    Flags: postinstall skipifsilent nowait

[UninstallRun]
; Remove SendTo shortcuts before files are deleted
Filename: "{app}\{#AppExeName}"; \
    Parameters: "cleanup"; \
    RunOnceId: "OffspringSendToCleanup"; \
    Flags: runhidden waituntilterminated
; Remove the shell-extension signing cert from LocalMachine\TrustedPeople.
; Best-effort: silently continues if the cert isn't present (per-user
; install that never trusted it) or if the process lacks admin rights.
Filename: "powershell.exe"; \
    Parameters: "-NoProfile -ExecutionPolicy Bypass -Command ""Get-ChildItem Cert:\LocalMachine\TrustedPeople -ErrorAction SilentlyContinue | Where-Object {{ $_.Subject -eq 'CN=Second March' }} | Remove-Item -ErrorAction SilentlyContinue"""; \
    RunOnceId: "OffspringCertCleanup"; \
    Flags: runhidden waituntilterminated

[UninstallDelete]
; Leave %APPDATA%\Offspring and %LOCALAPPDATA%\Offspring alone by default.
; Users can delete manually if they want to wipe presets / FFmpeg.
