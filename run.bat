@echo off
cd /d "%~dp0"
taskkill /F /IM claude-code-usage-monitor.exe /T >nul 2>&1
timeout /t 1 /nobreak >nul
cargo build --release
if %errorlevel% neq 0 (pause & exit /b 1)
start "" "%~dp0target\release\claude-code-usage-monitor.exe"
