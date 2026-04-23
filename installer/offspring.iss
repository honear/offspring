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
#define AppVersion   "0.2.0"
#define AppPublisher "Rolando Barry"
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
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
WizardStyle=modern
DisableProgramGroupPage=yes
DisableDirPage=auto
; Start Menu + uninstaller automatically under LOCALAPPDATA when non-admin

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional options:"; Flags: unchecked

[Files]
Source: "{#BinDir}\{#AppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\DESIGN.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "scripts\download_ffmpeg.ps1"; DestDir: "{app}\scripts"; Flags: ignoreversion

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

[UninstallDelete]
; Leave %APPDATA%\Offspring and %LOCALAPPDATA%\Offspring alone by default.
; Users can delete manually if they want to wipe presets / FFmpeg.
