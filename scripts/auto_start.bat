@echo off
:: Stock Market AI — Auto Start Script
:: Scheduled to run at 9:25 AM ET (before market open at 9:30)
:: Starts Docker containers and begins paper trading

set LOGFILE=%~dp0..\logs\auto_%date:~-4%%date:~4,2%%date:~7,2%.log
if not exist "%~dp0..\logs" mkdir "%~dp0..\logs"

echo [%date% %time%] === AUTO START === >> "%LOGFILE%"

:: Check if Docker Desktop is running, start if not
tasklist /FI "IMAGENAME eq Docker Desktop.exe" 2>NUL | find /I "Docker Desktop.exe" >NUL
if %ERRORLEVEL% NEQ 0 (
    echo [%date% %time%] Starting Docker Desktop... >> "%LOGFILE%"
    start "" "C:\Program Files\Docker\Docker\Docker Desktop.exe"
    :: Wait up to 90 seconds for Docker to be ready
    set /a count=0
    :wait_docker
    timeout /t 5 /nobreak >NUL
    docker info >NUL 2>&1
    if %ERRORLEVEL% EQU 0 goto docker_ready
    set /a count+=1
    if %count% LSS 18 goto wait_docker
    echo [%date% %time%] ERROR: Docker failed to start after 90s >> "%LOGFILE%"
    exit /b 1
)
:docker_ready
echo [%date% %time%] Docker is ready >> "%LOGFILE%"

:: Start trading system
cd /d "%~dp0.."
echo [%date% %time%] Starting containers... >> "%LOGFILE%"
docker compose up -d backend frontend redis >> "%LOGFILE%" 2>&1

:: Wait for backend health
set /a count=0
:wait_backend
timeout /t 3 /nobreak >NUL
curl -s http://localhost:8000/api/health >NUL 2>&1
if %ERRORLEVEL% EQU 0 goto backend_ready
set /a count+=1
if %count% LSS 20 goto wait_backend
echo [%date% %time%] WARNING: Backend not responding after 60s >> "%LOGFILE%"
goto done
:backend_ready
echo [%date% %time%] Backend healthy — paper trading active >> "%LOGFILE%"
echo [%date% %time%] Market opens at 9:30 AM ET. Trading will begin automatically. >> "%LOGFILE%"

:done
echo [%date% %time%] Auto start complete >> "%LOGFILE%"
