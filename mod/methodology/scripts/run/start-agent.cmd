@echo off
REM Start (or replace) the coop remote agent on THIS machine.
REM
REM -Force matters: a wedged earlier agent holds a named mutex forever, and a plain start then exits
REM silently. That is why several restarts in a row appeared to do nothing. -Force kills the previous
REM holder and takes over, and the window stays open so any boot failure is readable.
setlocal
set HERE=%~dp0
echo Starting coop agent from "%HERE%" ...
powershell -NoProfile -ExecutionPolicy Bypass -File "%HERE%start-agent.ps1" -Force
echo.
echo Agent exited (code %ERRORLEVEL%). If that was not Ctrl-C, the line above says why.
pause
