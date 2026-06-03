@echo off
title Stock Market AI Launcher
cd /d "%~dp0"
py launcher.py
if %errorlevel% neq 0 (
    echo.
    echo Failed to start. Make sure Python is installed.
    pause
)
