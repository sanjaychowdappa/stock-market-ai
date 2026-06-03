#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Registers Windows Task Scheduler tasks for Stock Market AI automation.

    Creates two scheduled tasks:
    1. StockMarketAI_Start — runs at 9:25 AM Mon-Fri (5 min before market open)
    2. StockMarketAI_Stop  — runs at 4:10 PM Mon-Fri (10 min after market close)

.NOTES
    Run this script as Administrator (required for Task Scheduler).
    To remove: .\setup_scheduler.ps1 -Remove
#>

param(
    [switch]$Remove
)

$TaskFolder = "StockMarketAI"
$ProjectDir = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$StartScript = Join-Path $ProjectDir "scripts\auto_start.bat"
$StopScript  = Join-Path $ProjectDir "scripts\auto_stop.bat"
$Username    = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name

# ── Remove mode ────────────────────────────────────────────
if ($Remove) {
    Write-Host "Removing Stock Market AI scheduled tasks..." -ForegroundColor Yellow
    try { Unregister-ScheduledTask -TaskName "StockMarketAI_Start" -Confirm:$false -ErrorAction Stop; Write-Host "  Removed StockMarketAI_Start" -ForegroundColor Green } catch { Write-Host "  StockMarketAI_Start not found" -ForegroundColor Gray }
    try { Unregister-ScheduledTask -TaskName "StockMarketAI_Stop"  -Confirm:$false -ErrorAction Stop; Write-Host "  Removed StockMarketAI_Stop"  -ForegroundColor Green } catch { Write-Host "  StockMarketAI_Stop not found"  -ForegroundColor Gray }
    Write-Host "`nDone. Tasks removed." -ForegroundColor Green
    exit 0
}

# ── Create mode ────────────────────────────────────────────
Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Stock Market AI — Scheduler Setup" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "  Project:     $ProjectDir"
Write-Host "  Start:       9:25 AM Mon-Fri"
Write-Host "  Stop:        4:10 PM Mon-Fri"
Write-Host "  User:        $Username"
Write-Host ""

# Verify scripts exist
if (-not (Test-Path $StartScript)) { Write-Error "Start script not found: $StartScript"; exit 1 }
if (-not (Test-Path $StopScript))  { Write-Error "Stop script not found: $StopScript"; exit 1 }

# Create logs directory
$LogDir = Join-Path $ProjectDir "logs"
if (-not (Test-Path $LogDir)) { New-Item -ItemType Directory -Path $LogDir -Force | Out-Null }

# ── Task 1: Auto Start at 9:25 AM ET, Mon-Fri ─────────────
Write-Host "Creating StockMarketAI_Start task..." -ForegroundColor Yellow

$StartAction  = New-ScheduledTaskAction -Execute "cmd.exe" -Argument "/c `"$StartScript`"" -WorkingDirectory $ProjectDir
$StartTrigger = New-ScheduledTaskTrigger -Weekly -DaysOfWeek Monday,Tuesday,Wednesday,Thursday,Friday -At "09:25AM"
$StartSettings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -StartWhenAvailable `
    -ExecutionTimeLimit (New-TimeSpan -Minutes 10) `
    -RestartCount 2 `
    -RestartInterval (New-TimeSpan -Minutes 1)

# Remove existing if present
try { Unregister-ScheduledTask -TaskName "StockMarketAI_Start" -Confirm:$false -ErrorAction Stop } catch {}

Register-ScheduledTask `
    -TaskName "StockMarketAI_Start" `
    -Action $StartAction `
    -Trigger $StartTrigger `
    -Settings $StartSettings `
    -Description "Starts Stock Market AI trading system 5 minutes before market open (9:30 AM ET)" `
    -RunLevel Highest `
    -User $Username | Out-Null

Write-Host "  Created: 9:25 AM Mon-Fri" -ForegroundColor Green

# ── Task 2: Auto Stop at 4:10 PM ET, Mon-Fri ──────────────
Write-Host "Creating StockMarketAI_Stop task..." -ForegroundColor Yellow

$StopAction  = New-ScheduledTaskAction -Execute "cmd.exe" -Argument "/c `"$StopScript`"" -WorkingDirectory $ProjectDir
$StopTrigger = New-ScheduledTaskTrigger -Weekly -DaysOfWeek Monday,Tuesday,Wednesday,Thursday,Friday -At "04:10PM"
$StopSettings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -StartWhenAvailable `
    -ExecutionTimeLimit (New-TimeSpan -Minutes 5) `
    -RestartCount 1 `
    -RestartInterval (New-TimeSpan -Minutes 1)

try { Unregister-ScheduledTask -TaskName "StockMarketAI_Stop" -Confirm:$false -ErrorAction Stop } catch {}

Register-ScheduledTask `
    -TaskName "StockMarketAI_Stop" `
    -Action $StopAction `
    -Trigger $StopTrigger `
    -Settings $StopSettings `
    -Description "Stops Stock Market AI after market close, saves EOD report" `
    -RunLevel Highest `
    -User $Username | Out-Null

Write-Host "  Created: 4:10 PM Mon-Fri" -ForegroundColor Green

# ── Verify ─────────────────────────────────────────────────
Write-Host ""
Write-Host "Verifying tasks..." -ForegroundColor Yellow
Get-ScheduledTask -TaskName "StockMarketAI_*" | Format-Table TaskName, State, @{N='NextRun';E={($_ | Get-ScheduledTaskInfo).NextRunTime}} -AutoSize

Write-Host ""
Write-Host "========================================" -ForegroundColor Green
Write-Host "  Automation is SET!" -ForegroundColor Green
Write-Host "========================================" -ForegroundColor Green
Write-Host ""
Write-Host "  Daily schedule (Mon-Fri):"
Write-Host "    9:25 AM  Docker starts, engines warm up"
Write-Host "    9:30 AM  Market opens, paper trading begins"
Write-Host "    4:00 PM  Market closes, trading stops"
Write-Host "    4:05 PM  EOD report auto-published"
Write-Host "    4:10 PM  Docker containers shut down"
Write-Host ""
Write-Host "  Logs:    $LogDir"
Write-Host "  Reports: $(Join-Path $ProjectDir 'reports')"
Write-Host ""
Write-Host "  To remove:  .\setup_scheduler.ps1 -Remove" -ForegroundColor Gray
Write-Host ""
