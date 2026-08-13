@echo off
setlocal enabledelayedexpansion
:: Stock Market AI - Auto Start
:: Scheduled 9:25 AM ET, before the 9:30 open.
::
:: Rewritten 2026-08-13. The previous version exited 255 the moment Docker
:: Desktop was not already running, which is every morning the machine has
:: slept. It logged "=== AUTO START ===" and died on the next line, so the
:: stack was 16 minutes late to the open on 08-12 and 44 minutes on 08-13.
::
:: Defects fixed:
::   1. Labels (:wait_docker) and goto inside a parenthesised if-block. cmd
::      cannot parse that; it is what produced the 255. Labels now live at the
::      top level only.
::   2. %count% inside a block expands once at parse time, so the retry loop
::      could never advance. setlocal enabledelayedexpansion + !count! fixes it.
::   3. "docker compose up -d backend frontend redis" named a redis service
::      that no longer exists in docker-compose.yml, and omitted the Kronos
::      sidecar. Bare "up -d" starts exactly what the compose file defines.
::   4. curl against localhost resolves to IPv6 ::1, where a stale wslrelay
::      swallows port 8000. Must be 127.0.0.1.
::
:: THREE RULES FOR EDITING THIS FILE. Each was learned by breaking it.
::
::   PURE ASCII. A rewrite used box-drawing characters in these comments.
::   cmd.exe reads the file in the OEM codepage, the multi-byte sequences broke
::   line parsing, and every comment line was executed as a command. Exit 255.
::
::   CRLF LINE ENDINGS. With LF-only endings cmd cannot resolve labels at all:
::   "The system cannot find the batch label specified - wait_docker".
::
::   NO "timeout" COMMAND. It requires a console. Under Task Scheduler, and any
::   time stdin is redirected, it fails with "Input redirection is not
::   supported, exiting the process immediately" and the wait loop collapses.
::   Use "ping -n <seconds+1> 127.0.0.1" as the sleep instead. The ORIGINAL
::   script used timeout, so this was breaking the scheduled run too.

set ROOT=%~dp0..
if not exist "%ROOT%\logs" mkdir "%ROOT%\logs"
set LOGFILE=%ROOT%\logs\auto_%date:~-4%%date:~4,2%%date:~7,2%.log

echo [%date% %time%] === AUTO START === >> "%LOGFILE%"

:: -- Docker Desktop --
tasklist /FI "IMAGENAME eq Docker Desktop.exe" 2>NUL | find /I "Docker Desktop.exe" >NUL
if not errorlevel 1 goto docker_launched
echo [%date% %time%] Docker Desktop not running - launching >> "%LOGFILE%"
start "" "C:\Program Files\Docker\Docker\Docker Desktop.exe"
:docker_launched

:: -- Wait for the daemon (up to 180s; a cold start can exceed 90) --
set /a count=0
:wait_docker
docker info >NUL 2>&1
if not errorlevel 1 goto docker_ready
ping -n 6 127.0.0.1 >NUL 2>&1
set /a count+=1
if !count! LSS 36 goto wait_docker
echo [%date% %time%] ERROR: Docker daemon not ready after 180s - ABORTING >> "%LOGFILE%"
exit /b 1

:docker_ready
echo [%date% %time%] Docker daemon ready after !count! attempts >> "%LOGFILE%"

:: -- Start the stack --
cd /d "%ROOT%"
echo [%date% %time%] Starting containers... >> "%LOGFILE%"
docker compose up -d >> "%LOGFILE%" 2>&1
if errorlevel 1 (
    echo [%date% %time%] ERROR: docker compose up failed >> "%LOGFILE%"
    exit /b 1
)

:: -- Wait for the backend to answer --
set /a count=0
:wait_backend
curl -s -m 5 http://127.0.0.1:8000/api/health >NUL 2>&1
if not errorlevel 1 goto backend_ready
ping -n 4 127.0.0.1 >NUL 2>&1
set /a count+=1
if !count! LSS 40 goto wait_backend
echo [%date% %time%] WARNING: backend not responding after 120s >> "%LOGFILE%"
goto done

:backend_ready
echo [%date% %time%] Backend healthy - paper trading active >> "%LOGFILE%"
curl -s -m 5 http://127.0.0.1:8000/api/health >> "%LOGFILE%" 2>&1
echo. >> "%LOGFILE%"

:done
echo [%date% %time%] Auto start complete >> "%LOGFILE%"
endlocal
