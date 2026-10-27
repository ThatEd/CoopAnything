@echo off
REM Join the host's co-op session. Run this on the CLIENT machine.
REM It receives the host's save into this game's SaveGames and keeps receiving race state.
setlocal
cd /d "%~dp0"

REM The host address is NOT committed: it differs per network. Put it in coop_supervisor.txt
REM (one line, host-ip:45990) next to this script. The HOST/PORT pair below is only a fallback
REM used when that file is missing.
set HOST=127.0.0.1
set PORT=45990

if not exist coop_supervisor.txt goto noconfig
for /f "usebackq delims=" %%a in ("coop_supervisor.txt") do set TARGET=%%a
if "%TARGET%"=="" goto noconfig

echo Connecting to host supervisor at %TARGET%
echo.
echo The host save will be written into this game's SaveGames folder.
echo Leave this window open while you play.
echo.
coop-link.exe --role client --supervisor %TARGET%
echo.
echo Link ended. Press any key.
pause >nul
exit /b 0

:noconfig
echo coop_supervisor.txt is missing or empty.
echo It should contain one line: host-ip:port
pause >nul
exit /b 1
