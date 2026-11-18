@echo off
cd /d "%~dp0"
"%~dp0medidor-recursos.exe" --segundo-plano
timeout /t 5
