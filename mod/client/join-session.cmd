@echo off
REM Attach this machine to the host's race-sim stream. Run once; it logs to coop-link-client.log.
REM The host address is per-network, so it comes from HOST_ADDR or coop_supervisor.txt, never from here.
setlocal
set HERE=%~dp0
cd /d "%HERE%"

if not "%HOST_ADDR%"=="" (
  set TARGET=%HOST_ADDR%
  goto run
)
if not exist coop_supervisor.txt goto nohost
set /p TARGET=<coop_supervisor.txt
if "%TARGET%"=="" goto nohost

:run
echo [%DATE% %TIME%] starting client link to %TARGET% >> "%HERE%coop-link-client.log"
"%HERE%coop-link.exe" --role client --supervisor %TARGET% --hz 20 >> "%HERE%coop-link-client.log" 2>&1
exit /b 0

:nohost
echo No host address. Set HOST_ADDR=host-ip:45990, or write it into coop_supervisor.txt.
pause
exit /b 1
