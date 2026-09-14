@echo off
cd /d "%~dp0"
cargo build --release
if %errorlevel% neq 0 (pause & exit /b 1)
start "" "%~dp0target\release\claude-code-usage-monitor.exe"
