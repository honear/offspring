@echo off
setlocal EnableDelayedExpansion

REM =====================================================================
REM Offspring context-menu cleanup
REM =====================================================================
REM Removes every Offspring-related registry key that could contribute a
REM right-click submenu entry, then restarts Explorer so the shell drops
REM its cached handlers. Fixes the "duplicate Offspring entries" symptom
REM that happens when an older install left stale keys behind.
REM
REM After running this, reinstall Offspring from:
REM   https://github.com/honear/offspring/releases/latest
REM
REM Double-click to run. It will self-elevate for HKLM cleanup (old
REM admin-installs). If you cancel the UAC prompt it still cleans the
REM per-user keys, which covers the common case.
REM =====================================================================

title Offspring context-menu cleanup

echo.
echo  ======================================================
echo   Offspring - context menu cleanup
echo  ======================================================
echo.
echo  This removes ALL Offspring entries from Windows's
echo  right-click menu (classic + Windows 11 modern).
echo.
echo  After it finishes, reinstall Offspring from:
echo    https://github.com/honear/offspring/releases/latest
echo.
pause

REM --- Self-elevate so we can also touch HKLM. If the user declines, we
REM     fall through and at least do the HKCU pass.
net session >nul 2>&1
if %errorlevel% neq 0 (
    echo.
    echo  Requesting administrator privileges for HKLM cleanup...
    powershell -NoProfile -Command "Start-Process -FilePath '%~f0' -Verb RunAs" >nul 2>&1
    if !errorlevel! equ 0 exit /b
    echo  Continuing without admin - HKLM keys will be skipped.
    echo.
)

set "CLSID={4A8F1E2B-6C9D-4E1F-8A2B-3C4D5E6F7A8B}"

echo.
echo  [1/3] Removing HKCU (per-user) keys...

reg delete "HKCU\Software\Classes\*\shell\Offspring"                           /f >nul 2>&1
reg delete "HKCU\Software\Classes\Offspring.SubCommands"                       /f >nul 2>&1
reg delete "HKCU\Software\Classes\Directory\shell\Offspring"                   /f >nul 2>&1
reg delete "HKCU\Software\Classes\Directory\Background\shell\Offspring"        /f >nul 2>&1
reg delete "HKCU\Software\Classes\Drive\shell\Offspring"                       /f >nul 2>&1
reg delete "HKCU\Software\Classes\AllFilesystemObjects\shell\Offspring"        /f >nul 2>&1
reg delete "HKCU\Software\Classes\CLSID\%CLSID%"                               /f >nul 2>&1
reg delete "HKCU\Software\Offspring"                                           /f >nul 2>&1

echo  [2/3] Removing HKLM (machine-wide) keys...

reg delete "HKLM\Software\Classes\*\shell\Offspring"                           /f >nul 2>&1
reg delete "HKLM\Software\Classes\Offspring.SubCommands"                       /f >nul 2>&1
reg delete "HKLM\Software\Classes\Directory\shell\Offspring"                   /f >nul 2>&1
reg delete "HKLM\Software\Classes\Directory\Background\shell\Offspring"        /f >nul 2>&1
reg delete "HKLM\Software\Classes\Drive\shell\Offspring"                       /f >nul 2>&1
reg delete "HKLM\Software\Classes\AllFilesystemObjects\shell\Offspring"        /f >nul 2>&1
reg delete "HKLM\Software\Classes\CLSID\%CLSID%"                               /f >nul 2>&1
reg delete "HKLM\Software\Offspring"                                           /f >nul 2>&1

echo  [3/3] Restarting Explorer to refresh the shell...

taskkill /f /im explorer.exe >nul 2>&1
timeout /t 1 /nobreak >nul
start "" explorer.exe

echo.
echo  ======================================================
echo   Done.
echo  ======================================================
echo.
echo  Next step: reinstall Offspring from
echo    https://github.com/honear/offspring/releases/latest
echo.
echo  The fresh install will recreate its menu entries from
echo  scratch, with no duplicates.
echo.
pause
endlocal
