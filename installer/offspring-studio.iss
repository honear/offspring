; Offspring Studio - Inno Setup script
; Cert-free, no-outbound-network variant of Offspring for users /
; admins who want the strictest behaviour profile. The Rust side is
; compiled with --features studio: the binary literally cannot fetch
; FFmpeg from gyan.dev, cannot reach github.com for updates, and
; cannot import certificates. Classic right-click menu only.
;
; Expected layout before compiling:
;   target-studio/release/offspring.exe   (renamed to offspring-studio.exe on install)
;
; Compile with the Inno Setup compiler (iscc.exe). Driven by
; tools/build-release.ps1 which sets CARGO_TARGET_DIR=target-studio
; for the studio cargo build pass.

#define AppName      "Offspring Studio"
#define AppVersion   "0.5.0-b0002"
#define AppVersionMsix "0.5.0.2"
#define AppPublisher "Second March"
#define AppExeName   "offspring-studio.exe"
; Distinct AppId from the standard build so the two installs don't
; recognise each other - users can have both side-by-side and the
; per-machine migration handler in offspring.iss won't trigger on
; this one.
#define AppId        "{{F1A4D7C2-9E3B-4A82-B5E9-1C7D5F2A8B14}"

; Build artifact location. build-release.ps1 invokes cargo with
; CARGO_TARGET_DIR=<repo>/target-studio for the studio pass; this
; default mirrors that so manual iscc runs work too.
#define StudioBinDir GetEnv("OFFSPRING_STUDIO_BIN_DIR")
#if StudioBinDir == ""
  #define StudioBinDir "..\target-studio\release"
#endif

[Setup]
AppId={#AppId}
AppName={#AppName}
AppVersion={#AppVersion}
VersionInfoVersion={#AppVersionMsix}
AppPublisher={#AppPublisher}
AppCopyright=Copyright (C) 2026 Second March
VersionInfoCompany={#AppPublisher}
VersionInfoCopyright=Copyright (C) 2026 Second March
VersionInfoDescription=Offspring Studio Installer
VersionInfoProductName={#AppName}
; Per-user install, separate folder so it coexists with Offspring
; standard if both are present.
DefaultDirName={userpf}\Offspring Studio
DefaultGroupName={#AppName}
UninstallDisplayIcon={app}\{#AppExeName}
OutputDir=dist
OutputBaseFilename=Offspring-Studio-Setup-{#AppVersion}
Compression=none
SolidCompression=no
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible
PrivilegesRequired=lowest
WizardStyle=modern
DisableProgramGroupPage=yes
DisableDirPage=auto
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional options:"; Flags: unchecked

[Files]
; Cargo builds the binary as `offspring.exe` inside the studio
; target-dir; DestName renames it on install so it's distinguishable
; in Task Manager / Process Explorer and so it doesn't collide if a
; user has the standard build installed in the same Programs dir.
Source: "{#StudioBinDir}\offspring.exe"; DestDir: "{app}"; DestName: "{#AppExeName}"; Flags: ignoreversion

; NO shell-extension DLL, NO MSIX packages, NO .cer file.
; That's the entire point of the studio variant.

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"
Name: "{group}\Uninstall {#AppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Run]
; No certutil call. No FFmpeg download. Studio installs do nothing
; beyond extracting offspring-studio.exe and registering uninstall.
; First-run seeds default presets + classic right-click menu.
Filename: "{app}\{#AppExeName}"; \
    Parameters: "first-run"; \
    Flags: runhidden waituntilterminated
Filename: "{app}\{#AppExeName}"; \
    Description: "Launch {#AppName}"; \
    Flags: postinstall skipifsilent nowait

[UninstallRun]
Filename: "{app}\{#AppExeName}"; \
    Parameters: "cleanup"; \
    RunOnceId: "OffspringStudioSendToCleanup"; \
    Flags: runhidden waituntilterminated

[UninstallDelete]
; Leave %APPDATA%\Offspring Studio and %LOCALAPPDATA%\Offspring Studio
; alone - users can wipe their preset/FFmpeg-path settings manually.

[Code]
function SetForegroundWindow(hWnd: Longword): Boolean;
  external 'SetForegroundWindow@user32.dll stdcall';
function ShowWindow(hWnd: Longword; nCmdShow: Integer): Boolean;
  external 'ShowWindow@user32.dll stdcall';
function BringWindowToTop(hWnd: Longword): Boolean;
  external 'BringWindowToTop@user32.dll stdcall';
function SwitchToThisWindow(hWnd: Longword; fAltTab: Boolean): Boolean;
  external 'SwitchToThisWindow@user32.dll stdcall';

const
  SW_RESTORE = 9;

// WebView2 Evergreen runtime presence check. Mirrors the function in
// offspring.iss - same CLSID, same dual-scope (HKLM 64-bit + HKCU)
// probe. Studio deliberately doesn't bundle the bootstrapper, so this
// is purely a detect-and-inform path.
function IsWebView2Installed: Boolean;
var
  Pv: String;
  ClientsKey: String;
begin
  Result := False;
  ClientsKey := 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\' +
                '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  if RegQueryStringValue(HKLM, ClientsKey, 'pv', Pv) then
  begin
    if (Pv <> '') and (Pv <> '0.0.0.0') then
    begin
      Result := True;
      Exit;
    end;
  end;
  ClientsKey := 'Software\Microsoft\EdgeUpdate\Clients\' +
                '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  if RegQueryStringValue(HKCU, ClientsKey, 'pv', Pv) then
  begin
    if (Pv <> '') and (Pv <> '0.0.0.0') then
      Result := True;
  end;
end;

// Studio's "no automatic outbound network" promise means we won't
// silently fetch the WebView2 runtime - that would defeat the
// entire point of the variant. Instead, if WebView2 is absent we
// stop the install with a clear MsgBox explaining what to do and
// linking the user to the Microsoft download page.
//
// Returning False from InitializeSetup aborts the install. The
// user has three options after that:
//   1. Install WebView2 from the Microsoft URL we surface, then
//      re-run this installer. The detection on second-run finds the
//      runtime and proceeds normally.
//   2. Install the standard Offspring variant instead, which bundles
//      the bootstrapper.
//   3. Skip Offspring entirely. Either way no network call ever
//      originated from this binary.
function InitializeSetup(): Boolean;
var
  Choice: Integer;
begin
  Result := True;
  if IsWebView2Installed then Exit;

  Choice := MsgBox(
    'Offspring Studio requires the Microsoft WebView2 Runtime to display its UI.' #13#10 #13#10 +
    'Studio installers never download anything from the network themselves, so we cannot fetch it for you. Two options:' #13#10 #13#10 +
    '  1. Click YES to open the Microsoft WebView2 download page. Install the "Evergreen Bootstrapper" or "Evergreen Standalone Installer" from there, then re-run this Offspring Studio installer.' #13#10 #13#10 +
    '  2. Click NO to cancel and instead install the standard Offspring variant - it bundles the WebView2 bootstrapper and handles this automatically.' #13#10 #13#10 +
    'Most Windows 11 machines already have WebView2 - this prompt only appears on fresh / minimal Windows installs.',
    mbInformation, MB_YESNO);
  if Choice = IDYES then
  begin
    ShellExec('open',
      'https://developer.microsoft.com/en-us/microsoft-edge/webview2/',
      '', '', SW_SHOW, ewNoWait, Choice);
  end;
  Result := False;
end;

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
