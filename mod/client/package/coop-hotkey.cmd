@echo off
REM Starts the co-op hotkey listener. Run this on the CLIENT machine and leave it minimised.
REM Ctrl+Alt+J then joins the host session (falls back to Ctrl+Alt+Shift+J if taken).
setlocal
cd /d "%~dp0"
echo co-op hotkey listener
echo   Ctrl+Alt+J  -  join the host session
echo.
echo Progress and errors are written to coop_link.log in this folder.
echo Close this window to stop listening.
echo.
coop-keys.exe
pause >nul
