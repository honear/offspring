; Offspring â€” Inno Setup script
; Builds a single-file installer for the Tauri-compiled offspring.exe,
; prompts the user to download FFmpeg on first install if not already
; present, and registers first-run/cleanup hooks for SendTo shortcuts.
;
; Expected layout before compiling this script (relative to this file):
;   app/src-tauri/target/release/offspring.exe
;   app/installer/msix/dist/OffspringShellExt*.msix + .cer (if modern menu)
;
; Compile with the Inno Setup compiler (iscc.exe or the Inno Setup IDE).

#define AppName      "Offspring"
#define AppVersion   "0.5.0-b0002"
#define AppVersionMsix "0.5.0.2"
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
; Fill in the version-info LegalCopyright field — leaving it empty
; reads as "unattributed binary" to security-conscious users and to
; some AV heuristics. Year stays static so the file-version resource
; doesn't churn every January 1st.
AppCopyright=Copyright (C) 2026 Second March
VersionInfoCompany={#AppPublisher}
VersionInfoCopyright=Copyright (C) 2026 Second March
VersionInfoDescription=Offspring Installer
VersionInfoProductName={#AppName}
; Per-user install. Lands under %LocalAppData%\Programs\Offspring,
; same scope VS Code / Discord / Slack use. No UAC prompt at install,
; no LocalMachine cert injection, no admin rights required. The
; trade-off: each Windows user on a shared PC has to install their
; own copy.
DefaultDirName={userpf}\{#AppName}
DefaultGroupName={#AppName}
UninstallDisplayIcon={app}\{#AppExeName}
OutputDir=dist
OutputBaseFilename=Offspring-Setup-{#AppVersion}
; Compression=none keeps Inno's payload as plain concatenated files
; instead of LZMA-compressed blobs. Yields a larger .exe (~3x) but
; produces a "clean" PE without the unusual .itext / .didata section
; entropy that triggers `packer_unknown_pe_section_name` heuristics
; in CAPE / VT sandboxes. User accepted the size trade-off explicitly.
Compression=none
SolidCompression=no
ArchitecturesInstallIn64BitMode=x64compatible
ArchitecturesAllowed=x64compatible
; Run unelevated so the user picks where it lands and so the modern-menu
; cert + MSIX get installed into the invoking user's per-user store
; (Cert:\CurrentUser\TrustedPeople + Add-AppxPackage user scope). Older
; per-machine installs are detected in InitializeSetup and migrated to
; per-user before proceeding â€” see the migration handler in [Code].
PrivilegesRequired=lowest
WizardStyle=modern
DisableProgramGroupPage=yes
DisableDirPage=auto
; If offspring.exe is running when the installer fires, Windows
; can't overwrite the locked exe. CloseApplications=yes tells Inno
; to detect running instances (registered via Restart Manager or
; matched by the file paths it's about to write) and politely ask
; them to close. RestartApplications=yes re-launches them after the
; install finishes - irrelevant for us since our [Run] section has
; its own /LAUNCHAFTER hook, but it's the standard pairing.
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

; No [Types] / [Components] sections — the modern right-click menu
; is the heart of the app, so making it an opt-in checkbox just
; created a coherence gap (the in-app "Reinstall modern menu" button
; can do the same thing later anyway, which means the checkbox isn't
; gating any real security boundary — it was UX theater). A separate
; "Offspring Studio" build is the right shape for users who want a
; cert-free / capability-capped variant; see SECURITY.md.

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional options:"; Flags: unchecked

[Files]
Source: "{#BinDir}\{#AppExeName}"; DestDir: "{app}"; Flags: ignoreversion
; Microsoft Edge WebView2 Evergreen Bootstrapper. Tauri renders its
; UI through WebView2, which is pre-installed on Windows 11 and on
; mid-2021+ Windows 10 builds (via Edge updates) but missing on
; fresh-out-of-box snapshots / Windows Sandbox containers. The
; bootstrapper is a ~1.7 MB Microsoft-signed binary that:
;   - silently no-ops if WebView2 is already present
;   - downloads + installs the runtime if not
; We only [Run] it when our IsWebView2Installed [Code] check returns
; False, so users with a normal Windows install never see it touch
; their machine. Extracted to {tmp} and auto-deleted after install.
;
; Microsoft permits redistribution under their "Distribution of the
; WebView2 Runtime" terms (https://developer.microsoft.com/en-us/microsoft-edge/webview2/).
; The file is gitignored - tools/build-release.ps1 ensures it's
; present locally before iscc runs.
Source: "MicrosoftEdgeWebView2Setup.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall
; Shell-ext DLL + three signed sparse MSIX packages + public cert.
; ALWAYS copied to disk regardless of the modernmenu component
; checkbox — the in-app "Set up Windows 11 modern menu..." button
; in Settings needs them to be present on disk so a user who
; unchecked the component at install time can opt in later
; without re-running the installer. Only the auto-trust + register
; [Run] step in this script is component-gated; the files
; themselves are universal. Cumulative on-disk cost is ~150 KB.
;
; Three MSIX packages share one DLL on disk:
;   * OffspringShellExt.msix         - "Offspring" (unified mode)
;   * OffspringShellExt.Presets.msix - "Offspring Presets" (split mode)
;   * OffspringShellExt.Tools.msix   - "Offspring Tools" (split mode)
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
; Bootstrap Microsoft WebView2 runtime if it isn't already present.
; Gated by IsWebView2Installed in [Code] so the bootstrapper is only
; launched on machines that actually need it - typical Windows 11
; installs ship WebView2 and skip this entirely. The bootstrapper
; itself is Microsoft-signed and handles its own UAC if it needs to
; install machine-wide (it doesn't on per-user-only systems).
;
; Runs BEFORE the offspring.exe first-run below because first-run
; spawns offspring.exe which would fail to launch its WebView if the
; runtime wasn't in place yet.
Filename: "{tmp}\MicrosoftEdgeWebView2Setup.exe"; \
    Parameters: "/silent /install"; \
    StatusMsg: "Installing Microsoft WebView2 runtime..."; \
    Check: NeedsWebView2; \
    Flags: waituntilterminated

; FFmpeg is intentionally NOT fetched at install time. The app surfaces
; a "Download FFmpeg" button in Settings on first launch when it
; detects FFmpeg is missing, so the user opts in to that network call
; rather than having it happen silently during install. Aligns with
; the "no automatic outbound" promise in SECURITY.md and removes one
; PowerShell-script-drop signature from sandbox scanners.

; Trust the shell-extension signing cert in the invoking user's
; CurrentUser\TrustedPeople store so the modern-menu toggle's
; Add-AppxPackage call doesn't fail with 0x800B0109 (untrusted root).
; Per-user scope means no admin required, no machine-wide change.
;
; certutil.exe is a Microsoft-signed system tool that ships with
; Windows in System32 - using it instead of PowerShell's
; Import-Certificate avoids dropping a .ps1 to disk, the
; `-ExecutionPolicy Bypass` string, and the "scripting utility was
; executed" sandbox flag. Same functional result, much smaller
; surface for AV heuristics to grab onto.
Filename: "certutil.exe"; \
    Parameters: "-user -addstore TrustedPeople ""{app}\OffspringShellExt.cer"""; \
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
; once [Run] reaches this entry. The installer already runs as the user
; (no admin) so no scope-handoff is needed. Gated on the flag so normal
; silent installs (e.g. deployment scripts) aren't surprised by a window
; popping up.
Filename: "{app}\{#AppExeName}"; \
    Flags: nowait; \
    Check: ShouldLaunchAfter

[UninstallRun]
; Remove SendTo shortcuts before files are deleted
Filename: "{app}\{#AppExeName}"; \
    Parameters: "cleanup"; \
    RunOnceId: "OffspringSendToCleanup"; \
    Flags: runhidden waituntilterminated
; Remove the shell-extension signing cert from CurrentUser\TrustedPeople.
; Best-effort: silently continues if the cert isn't present (user opted
; out of the modern-menu component, or already uninstalled).
;
; Match by FriendlyName ('Offspring Shell Ext Dev Cert') *and*
; CN=Second March, so we only remove certificates we provisioned â€”
; never an unrelated cert that happens to share the CN. The FriendlyName
; is set at provisioning time in build-msix.ps1.
Filename: "powershell.exe"; \
    Parameters: "-NoProfile -NonInteractive -Command ""Get-ChildItem Cert:\CurrentUser\TrustedPeople -ErrorAction SilentlyContinue | Where-Object {{ $_.Subject -eq 'CN=Second March' -and $_.FriendlyName -eq 'Offspring Shell Ext Dev Cert' }} | Remove-Item -ErrorAction SilentlyContinue"""; \
    RunOnceId: "OffspringCertCleanup"; \
    Flags: runhidden waituntilterminated

[InstallDelete]
; Pre-install cleanup: remove files that earlier builds shipped but
; this one no longer does. Inno's default reinstall behavior just
; overwrites what's in [Files]; orphan files from previous versions
; (like the {app}\scripts\*.ps1 we dropped in 0.4.4) stay around
; forever otherwise. Listed explicitly so we don't accidentally
; delete anything we still ship.
Type: filesandordirs; Name: "{app}\scripts"

[UninstallDelete]
; Leave %APPDATA%\Offspring and %LOCALAPPDATA%\Offspring alone by default.
; Users can delete manually if they want to wipe presets / FFmpeg.

[Code]
// Win32 APIs imported from user32.dll. We use these to force the wizard
// window to the foreground after it's created â€” by default Inno's setup
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
// ExitProcess from kernel32. Used by the silent-update migration
// fallback to force exit code 0 — Inno's own `Result := False`
// returns a non-zero status that v0.4.3's in-app updater treats as
// "installer exited early; update not applied" and surfaces a
// confusing error. We want the silent run to look like a clean exit
// so the old app shuts down quietly while the freshly-spawned
// interactive copy of THIS installer handles the actual migration.
procedure ExitProcess(uExitCode: Cardinal);
  external 'ExitProcess@kernel32.dll stdcall';
// SW_SHOW is pre-declared by Inno's Pascal scripting library, but
// SW_RESTORE isn't â€” declare the missing one ourselves. Re-declaring
// SW_SHOW would throw "Duplicate identifier".
const
  SW_RESTORE = 9;
  // HKLM uninstall key Inno writes when installing per-machine. We
  // probe both the 64-bit and 32-bit views (older installer builds
  // may have ended up in WoW6432Node).
  HKLM_UNINSTALL_KEY = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#AppId}_is1';

// WebView2 Evergreen runtime presence check. Microsoft documents two
// registry locations to detect the installed runtime, depending on
// install scope:
//   per-machine: HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{CLSID}
//   per-user:    HKCU\Software\Microsoft\EdgeUpdate\Clients\{CLSID}
// The CLSID below is the documented Evergreen client ID. We consider
// the runtime "installed" if either key exists AND has a non-empty
// `pv` (product version) value.
//
// Used by the [Run] entry that fires MicrosoftEdgeWebView2Setup.exe -
// the bootstrapper itself is idempotent (no-ops if WebView2 is
// already installed), but skipping the run entirely on machines
// that already have it saves a few seconds of install time and
// keeps the UAC-prompt count down on per-machine WebView2 setups.
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

// Inverse of IsWebView2Installed, used as the [Run] Check predicate
// (Inno's Check returns "should this entry execute"). Pulled out as
// a named function so the [Run] line reads as "Check: NeedsWebView2"
// rather than a less-obvious inline negation.
function NeedsWebView2: Boolean;
begin
  Result := not IsWebView2Installed;
end;

// Look up the QuietUninstallString of an existing per-machine install,
// preferring 64-bit view but falling back to 32-bit. Returns '' if no
// per-machine install is registered.
function GetPerMachineUninstallString: String;
var
  S: String;
begin
  Result := '';
  if RegQueryStringValue(HKLM, HKLM_UNINSTALL_KEY, 'QuietUninstallString', S) then
  begin
    Result := S;
    Exit;
  end;
  if RegQueryStringValue(HKLM, HKLM_UNINSTALL_KEY, 'UninstallString', S) then
  begin
    // Inno-generated UninstallString is shell-quoted already. Append
    // /VERYSILENT /SUPPRESSMSGBOXES so we can run it non-interactively.
    Result := S + ' /VERYSILENT /SUPPRESSMSGBOXES /NORESTART';
    Exit;
  end;
end;

// Split a quoted command line into the executable path and its
// remaining argument string. Handles the Inno-style format:
//     "C:\Path with spaces\unins000.exe" /VERYSILENT
// Falls back to splitting on the first space if there are no quotes.
procedure SplitCommandLine(Cmd: String; var ExePath, Args: String);
var
  P: Integer;
begin
  ExePath := '';
  Args := '';
  Cmd := Trim(Cmd);
  if Cmd = '' then Exit;
  if Copy(Cmd, 1, 1) = '"' then
  begin
    P := Pos('"', Copy(Cmd, 2, Length(Cmd)));
    if P > 0 then
    begin
      ExePath := Copy(Cmd, 2, P - 1);
      Args := Trim(Copy(Cmd, P + 2, Length(Cmd)));
      Exit;
    end;
  end;
  P := Pos(' ', Cmd);
  if P > 0 then
  begin
    ExePath := Copy(Cmd, 1, P - 1);
    Args := Trim(Copy(Cmd, P + 1, Length(Cmd)));
  end
  else
    ExePath := Cmd;
end;

// InitializeSetup runs before the wizard is shown. Returning False
// aborts the install. We use it to detect existing per-machine
// installs (from <=0.4.3 admin-scope installers) and offer to migrate
// them to per-user before continuing.
//
// Why: leaving the per-machine install in place would create:
//   - Two Offspring folders (Program Files + LocalAppData\Programs)
//   - Two Add/Remove Programs entries
//   - The user wouldn't know which one their right-click menu points to
//   - The HKLM-scope MSIX registration would shadow the per-user one
//
// Behavior:
//   - Interactive run: show a MsgBox explaining the situation; if the
//     user accepts, ShellExec the old uninstaller (UAC prompt for
//     elevation, since the old uninstaller needs admin) and wait for
//     it to finish, then continue. If the user declines, abort the
//     install - we don't want to leave them with duplicates.
//   - Silent run (in-app updater from v0.4.3): we can't safely elevate
//     mid-silent-update without surprising the user. But silently
//     bailing out makes the old app surface a confusing "installer
//     exited early" error. Compromise: re-spawn THIS installer
//     interactively (without /VERYSILENT) so the migration prompt
//     appears, then ExitProcess(0) so v0.4.3's install_update sees a
//     clean exit and shuts down. The user sees one window - the new
//     interactive installer - pick up where things left off.
function InitializeSetup(): Boolean;
var
  UninstallCmd, ExePath, Args: String;
  ResultCode: Integer;
  UserChoice: Integer;
  SelfPath: String;
begin
  Result := True;
  UninstallCmd := GetPerMachineUninstallString;
  if UninstallCmd = '' then Exit; // no migration needed

  if WizardSilent then
  begin
    // Silent path (in-app updater from v0.4.3). Re-launch self
    // interactively so the user sees the migration MsgBox and the
    // UAC prompt for the old uninstaller is contextualised. Pass
    // /LAUNCHAFTER through so the freshly-installed binary still
    // gets auto-launched at the end of the wizard (matches what
    // v0.4.3's install_update originally asked for).
    SelfPath := ExpandConstant('{srcexe}');
    Log('Offspring: per-machine install detected during silent run; ' +
        're-spawning interactively for migration prompt: ' + SelfPath);
    if not ShellExec('open', SelfPath, '/LAUNCHAFTER', '', SW_SHOW,
                     ewNoWait, ResultCode) then
    begin
      Log('Offspring: failed to re-spawn interactive installer; aborting.');
      Result := False;
      Exit;
    end;
    // Force exit code 0 so v0.4.3's install_update sees a clean
    // exit (its 500ms try_wait then treats Ok(Some(0)) as "did our
    // part - quit so user can relaunch manually") rather than the
    // misleading "installer exited early" error. The interactive
    // copy is now running and will handle the actual migration.
    ExitProcess(0);
  end;

  UserChoice := MsgBox(
    'An older version of Offspring is installed for all users on this PC.' #13#10 #13#10 +
    'Offspring now installs per-user, which is more secure (no admin rights, no machine-wide certificate trust).' #13#10 #13#10 +
    'Click Yes to uninstall the old version first, then continue with the per-user install.' #13#10 +
    'Click No to cancel this installer (the old version will be left alone).' #13#10 #13#10 +
    'Your presets and FFmpeg cache will be preserved either way â€” they live in %AppData% and %LocalAppData%.',
    mbConfirmation, MB_YESNO);
  if UserChoice <> IDYES then
  begin
    Result := False;
    Exit;
  end;

  SplitCommandLine(UninstallCmd, ExePath, Args);
  if ExePath = '' then
  begin
    MsgBox('Could not locate the existing uninstaller. Please uninstall the old Offspring manually via Settings > Apps, then re-run this installer.',
      mbError, MB_OK);
    Result := False;
    Exit;
  end;

  // ShellExec with verb 'runas' triggers a UAC prompt â€” the old
  // installer ran under admin so its uninstaller needs admin too.
  // SW_SHOW so the user sees what's happening (uninstaller has its
  // own brief progress window).
  if not ShellExec('runas', ExePath, Args, '', SW_SHOW, ewWaitUntilTerminated, ResultCode) then
  begin
    MsgBox('Could not launch the old uninstaller (the elevation prompt may have been declined). Aborting.',
      mbError, MB_OK);
    Result := False;
    Exit;
  end;

  if ResultCode <> 0 then
  begin
    MsgBox('The old uninstaller exited with code ' + IntToStr(ResultCode) + '. Please uninstall the old Offspring manually via Settings > Apps, then re-run this installer.',
      mbError, MB_OK);
    Result := False;
    Exit;
  end;

  // Migration succeeded. Continue with per-user install.
end;

// Inno fires InitializeWizard right after WizardForm is constructed but
// before the first page is rendered. That's the cleanest place to nail
// the foreground state â€” we want the user to see the wizard on top the
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
// command-line scan is robust enough â€” the switch is ours, not Inno's, so
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
