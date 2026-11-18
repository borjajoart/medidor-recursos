@echo off
cd /d "%~dp0"
"%~dp0medidor-recursos.exe" detener
timeout /t 5
