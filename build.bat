@echo off
rem Space Peace: Not at War - Doppelklick zum Bauen (ruft build.ps1 auf)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0build.ps1" %*
pause
