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
#define AppVersion   "0.4.1"
#define AppVersionMsix "0.4.1.0"
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
; VersionInfoVersion has to be MAJOR.MINOR.BUILD.REVISION four-numeric
; (Win32 file-version resource format). When AppVersion carries a
; pre-release tag like "0.3.41-0007", the bumper writes the
; four-numeric form into AppVersionMsix and we use that here so the
; installer .exe still has a valid version resource.
VersionInfoVersion={#AppVersionMsix}
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
Source: "scripts\download_ffmpeg.ps1"; DestDir: "{app}\scripts"; Flags: ignoreversion
Source: "scripts\trust_cert.ps1"; DestDir: "{app}\scripts"; Flags: ignoreversion
; Shell-ext DLL + three signed sparse MSIX packages + public cert,
; consumed by the modern-menu Settings toggle (Add-AppxPackage runs
; against them from the app). The CI workflow copies the DLL from
; shell-ext/target/release/ into {#BinDir} before iscc runs.
;
; Three MSIX packages share one DLL on disk:
;   * OffspringShellExt.msix         — "Offspring" (unified mode)
;   * OffspringShellExt.Presets.msix — "Offspring Presets" (split mode)
;   * OffspringShellExt.Tools.msix   — "Offspring Tools" (split mode)
; Each has a distinct package identity so Win11 doesn't auto-group
; them under one parent. The app dynamically registers either
; {Unified} or {Presets, Tools} based on the user's split-layout
; toggle.
Source: "{#BinDir}\offspring_shell_ext.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "msix\dist\OffspringShellExt.msix"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "msix\dist\OffspringShellExt.Presets.msix"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
Source: "msix\dist\OffspringShellExt.Tools.msix"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
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
; In-app updater relaunch: when the app spawns this installer with
; /LAUNCHAFTER (silent-update flow), launch the freshly-installed binary
; once [Run] reaches this entry. runasoriginaluser drops admin so the app
; runs in the invoking user's context — not the elevated installer's —
; which matters for per-user AppData/registry writes. Gated on the flag
; so normal silent installs (e.g. deployment scripts) aren't surprised by
; a window popping up.
Filename: "{app}\{#AppExeName}"; \
    Flags: nowait runasoriginaluser; \
    Check: ShouldLaunchAfter

[UninstallRun]
; Remove SendTo shortcuts before files are deleted
Filename: "{app}\{#AppExeName}"; \
    Parameters: "cleanup"; \
    RunOnceId: "OffspringSendToCleanup"; \
    Flags: runhidden waituntilterminated
; Remove the shell-extension signing cert from LocalMachine\TrustedPeople.
; Best-effort: silently continues if the cert isn't present (per-user
; install that never trusted it) or if the process lacks admin rights.
;
; Match by FriendlyName ('Offspring Shell Ext Dev Cert') *and*
; CN=Second March, so we only remove certificates we provisioned —
; never an unrelated cert that happens to share the CN. This is
; defense-in-depth against an admin who's installed multiple things
; under the same Subject; the FriendlyName is set at provisioning time
; in build-msix.ps1.
Filename: "powershell.exe"; \
    Parameters: "-NoProfile -ExecutionPolicy Bypass -Command ""Get-ChildItem Cert:\LocalMachine\TrustedPeople -ErrorAction SilentlyContinue | Where-Object {{ $_.Subject -eq 'CN=Second March' -and $_.FriendlyName -eq 'Offspring Shell Ext Dev Cert' }} | Remove-Item -ErrorAction SilentlyContinue"""; \
    RunOnceId: "OffspringCertCleanup"; \
    Flags: runhidden waituntilterminated

[UninstallDelete]
; Leave %APPDATA%\Offspring and %LOCALAPPDATA%\Offspring alone by default.
; Users can delete manually if they want to wipe presets / FFmpeg.

[Code]
// Win32 APIs imported from user32.dll. We use these to force the wizard
// window to the foreground after it's created — by default Inno's setup
// window can come up BEHIND whatever was last active, because Windows'
// foreground-lock rules don't always grant the new process focus
// (especially after a UAC elevation handoff). Calling SetForegroundWindow
// + a SW_SHOW reassertion at the right moment side-steps that.
function SetForegroundWindow(hWnd: Longword): Boolean;
  external 'SetForegroundWindow@user32.dll stdcall';
function ShowWindow(hWnd: Longword; nCmdShow: Integer): Boolean;
  external 'ShowWindow@user32.dll stdcall';
function BringWindowToTop(hWnd: Longword): Boolean;
  external 'BringWindowToTop@user32.dll stdcall';
function SwitchToThisWindow(hWnd: Longword; fAltTab: Boolean): Boolean;
  external 'SwitchToThisWindow@user32.dll stdcall';
// SW_SHOW is pre-declared by Inno's Pascal scripting library, but
// SW_RESTORE isn't — declare the missing one ourselves. Re-declaring
// SW_SHOW would throw "Duplicate identifier".
const
  SW_RESTORE = 9;

// Inno fires InitializeWizard right after WizardForm is constructed but
// before the first page is rendered. That's the cleanest place to nail
// the foreground state — we want the user to see the wizard on top the
// moment it appears.
//
// We belt-and-suspender three Win32 calls because SetForegroundWindow
// alone can be silently no-op'd by Windows when the process doesn't
// hold the foreground "right" (it returns FALSE but doesn't error). The
// combo of ShowWindow(SW_RESTORE/SW_SHOW) + BringWindowToTop +
// SwitchToThisWindow is the standard installer-side workaround used by
// e.g. Inno Setup community templates.
procedure InitializeWizard;
var
  H: Longword;
begin
  H := WizardForm.Handle;
  ShowWindow(H, SW_RESTORE);
  ShowWindow(H, SW_SHOW);
  BringWindowToTop(H);
  SetForegroundWindow(H);
  SwitchToThisWindow(H, True);
end;

// ShouldLaunchAfter returns True iff the installer was invoked with the
// custom /LAUNCHAFTER switch. The in-app updater passes this when it wants
// the freshly-installed binary to relaunch itself on completion. A plain
// command-line scan is robust enough — the switch is ours, not Inno's, so
// Inno won't strip it from GetCmdTail.
function ShouldLaunchAfter: Boolean;
var
  I: Integer;
begin
  Result := False;
  for I := 1 to ParamCount do
  begin
    if CompareText(ParamStr(I), '/LAUNCHAFTER') = 0 then
    begin
      Result := True;
      Exit;
    end;
  end;
end;
