@echo off
:: Stock Market AI - Auto Stop
:: Scheduled 16:10 ET Mon-Fri, and 00:05 Saturday as a backstop.
:: Saves the backend log, fetches the final EOD report, stops the stack.
::
:: Rewritten 2026-09-15. Three defects, all of which auto_start.bat had
:: already fixed on its side and this file never received:
::
::   1. The log file name was sliced from %date% assuming US order. This
::      machine's %date% is dd-mm-yyyy, so every stop since May logged to
::      "auto_20269-02.log" - not to the day's file - and a search of the
::      day's log for the stop found nothing. The date now comes from
::      PowerShell, the same way auto_start.bat gets it.
::   2. curl went to localhost, which resolves to IPv6 ::1, where a stale
::      wslrelay accepts the connection whether or not the backend is up.
::      On 2026-09-14 that logged "EOD report saved" for a backend whose
::      scheduler had stopped 15 minutes earlier. Must be 127.0.0.1.
::   3. "docker compose down" deletes the containers and, with them, the
::      only copy of the backend's log. On 2026-09-14 the 3:55pm skim did
::      not run; by the time anyone looked the log that would have said why
::      was gone. The log is now saved to logs\backend_<date>.log first.
::
:: The three rules from auto_start.bat apply here too and each has been
:: broken in this repo: PURE ASCII (this file carried an em dash in its
:: title line), CRLF LINE ENDINGS, and NO "timeout" COMMAND.

set ROOT=%~dp0..
if not exist "%ROOT%\logs" mkdir "%ROOT%\logs"
for /f %%d in ('powershell -NoProfile -Command "Get-Date -Format yyyyMMdd"') do set TODAY=%%d
set LOGFILE=%ROOT%\logs\auto_%TODAY%.log

echo [%date% %time%] === AUTO STOP === >> "%LOGFILE%"
cd /d "%ROOT%"

:: -- Preserve the container logs BEFORE anything can remove them --
:: If the stack is not running this writes an empty file, which is fine.
echo [%date% %time%] Saving container logs... >> "%LOGFILE%"
docker compose logs --no-color --timestamps backend > "%ROOT%\logs\backend_%TODAY%.log" 2>&1
docker compose logs --no-color --timestamps finetune-sidecar > "%ROOT%\logs\sidecar_%TODAY%.log" 2>&1
echo [%date% %time%] Container logs saved to logs\backend_%TODAY%.log >> "%LOGFILE%"

:: -- Final EOD report --
echo [%date% %time%] Fetching final EOD report... >> "%LOGFILE%"
curl -s -m 20 http://127.0.0.1:8000/api/eod/report > NUL 2>&1
if errorlevel 1 (
    echo [%date% %time%] WARNING: Could not fetch EOD report - backend not answering on 127.0.0.1:8000 >> "%LOGFILE%"
) else (
    echo [%date% %time%] EOD report saved to reports/ >> "%LOGFILE%"
)

:: -- Final portfolio state --
echo [%date% %time%] Final daily status: >> "%LOGFILE%"
curl -s -m 10 http://127.0.0.1:8000/api/daily/status >> "%LOGFILE%" 2>&1
echo. >> "%LOGFILE%"

:: -- Stop the stack --
echo [%date% %time%] Stopping containers... >> "%LOGFILE%"
docker compose down >> "%LOGFILE%" 2>&1
echo [%date% %time%] All containers stopped >> "%LOGFILE%"
echo [%date% %time%] === SESSION COMPLETE === >> "%LOGFILE%"
echo. >> "%LOGFILE%"
