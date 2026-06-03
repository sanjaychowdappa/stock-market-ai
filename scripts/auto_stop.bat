@echo off
:: Stock Market AI — Auto Stop Script
:: Scheduled to run at 4:10 PM ET (after market close + EOD report)
:: Fetches final EOD report, then shuts down all containers

set LOGFILE=%~dp0..\logs\auto_%date:~-4%%date:~4,2%%date:~7,2%.log
if not exist "%~dp0..\logs" mkdir "%~dp0..\logs"

echo [%date% %time%] === AUTO STOP === >> "%LOGFILE%"

cd /d "%~dp0.."

:: Fetch and save the final EOD report
echo [%date% %time%] Fetching final EOD report... >> "%LOGFILE%"
curl -s http://localhost:8000/api/eod/report > NUL 2>&1
if %ERRORLEVEL% EQU 0 (
    echo [%date% %time%] EOD report saved to reports/ >> "%LOGFILE%"
) else (
    echo [%date% %time%] WARNING: Could not fetch EOD report >> "%LOGFILE%"
)

:: Log final portfolio state
echo [%date% %time%] Final daily status: >> "%LOGFILE%"
curl -s http://localhost:8000/api/daily/status >> "%LOGFILE%" 2>&1
echo. >> "%LOGFILE%"

:: Stop all containers
echo [%date% %time%] Stopping containers... >> "%LOGFILE%"
docker compose down >> "%LOGFILE%" 2>&1

echo [%date% %time%] All containers stopped >> "%LOGFILE%"
echo [%date% %time%] === SESSION COMPLETE === >> "%LOGFILE%"
echo. >> "%LOGFILE%"
