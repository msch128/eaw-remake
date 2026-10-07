@echo off
rem Space Peace: Not at War - double-click to build (runs build.ps1)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0build.ps1" %*
pause
