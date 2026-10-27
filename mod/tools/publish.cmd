@echo off
REM Publish bridge.js live and (if needed) restart the command server.
REM
REM Why this exists: the pak only bootstraps the bridge. The bridge then re-fetches its own source
REM from /bridge.js whenever /bridge.rev changes, so editing bridge logic needs no repack and no
REM game restart. Without this, every tweak cost a repak plus a restart of two games.
setlocal
set HERE=%~dp0
set SRC=%HERE%..\pak\f1manager24\Content\UIGameface\js\coop\bridge.js

node -e "new Function(require('fs').readFileSync(process.argv[1],'utf8'))" "%SRC%"
if errorlevel 1 (
  echo [publish] bridge.js has a syntax error - NOT publishing
  exit /b 1
)
echo [publish] bridge.js ok, revision is now its mtime

curl -s --max-time 3 "http://127.0.0.1:45999/bridge.rev" >nul 2>&1
if errorlevel 1 (
  echo [publish] command server not answering, starting it
  start "" /min python "%HERE%coop_server.py"
  timeout /t 3 >nul
)
echo [publish] revision: 
curl -s --max-time 3 "http://127.0.0.1:45999/bridge.rev"
echo.
echo [publish] clients pick it up within ~3s. Nothing to restart.
