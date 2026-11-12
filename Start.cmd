@echo off
cd /d "%~dp0"
call npm run build >nul
start "" npx electron .
