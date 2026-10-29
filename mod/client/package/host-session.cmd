@echo off
REM Start hosting. Run this on the HOST machine (the one with the save).
REM   host-session.cmd                 send the latest autosave
REM   host-session.cmd "save168.sav"   send a specific save from SaveGames
setlocal
cd /d "%~dp0"

if "%1"=="" (
  set SAVE=%LOCALAPPDATA%\F1Manager24\Saved\SaveGames\autosave.sav
) else (
  set SAVE=%LOCALAPPDATA%\F1Manager24\Saved\SaveGames\%~1
)

if not exist "%SAVE%" (
  echo Save not found: %SAVE%
  echo Pass a file name that exists in %%LOCALAPPDATA%%\F1Manager24\Saved\SaveGames
  pause >nul
  exit /b 1
)

echo Starting the relay on all interfaces, port 45990.
start "coop-supervisor" coop-supervisor.exe --bind 0.0.0.0
timeout /t 2 >nul

echo.
echo Sending session save: %SAVE%
echo The game pid is detected automatically. Leave this window open.
echo.
coop-link.exe --role host --supervisor 127.0.0.1:45990 --hz 10 --send-save "%SAVE%"
echo.
echo Host link ended. Press any key.
pause >nul
