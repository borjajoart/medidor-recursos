@echo off
title Medidor de recursos
cd /d "%~dp0"
echo Deja esta ventana abierta mientras juegas.
echo Para terminar y ver el informe pulsa Ctrl+C en esta ventana.
echo.
"%~dp0medidor-recursos.exe"
pause
