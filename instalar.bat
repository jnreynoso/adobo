@echo off
title Instalador de Adobo Reader
cls
echo ========================================================
echo   Iniciando Instalador de Adobo Reader...
echo ========================================================
echo.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install.ps1"
echo.
echo Presione cualquier tecla para salir...
pause > null
del null
