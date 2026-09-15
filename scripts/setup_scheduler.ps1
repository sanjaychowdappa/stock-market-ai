<#
.SYNOPSIS
    Registers the Windows Task Scheduler tasks that run Stock Market AI
    unattended. Running it again re-registers all four exactly as below,
    so this file IS the definition of the schedule.

    StockMarketAI_Start        Mon-Fri 09:25 ET, then every 15 min for 6 h,
                               and 90 s after every logon.
                               scripts\auto_start.bat (idempotent; it has its
                               own weekend and market-hours guards).
    StockMarketAI_Stop         Mon-Fri 16:10 ET.   scripts\auto_stop.bat
    StockMarketAI_WeekendStop  Sat 00:05.          scripts\auto_stop.bat
    StockAI Daily Analysis     Mon-Fri 16:15 ET.   analyze.ps1

.DESCRIPTION
    Rewritten 2026-09-15. The previous version registered only Start and
    Stop, with a plain 09:25 trigger and a visible "cmd.exe /c" console, and
    had drifted from what was actually installed by hand over the following
    months. Re-running it would have silently removed the 15-minute
    repetition and the logon trigger that get the stack started after the
    laptop wakes late. Everything the live schedule needs is now here:

    * Every action runs through scripts\run_hidden.vbs. The tasks must run in
      the interactive session (auto_start.bat launches Docker Desktop, which
      has to land on the user's desktop), and an interactive "cmd.exe /c"
      opens a console window for the life of the script. With a 15-minute
      repetition that was a black window flashing four times an hour, all
      day. wscript.exe owns no console and starts the child hidden.

    * StartWhenAvailable and AllowStartIfOnBatteries on every task. The
      WeekendStop and Daily Analysis tasks were created with the cmdlet
      defaults, which are the opposite: WeekendStop was refused on 2026-09-12
      (result 0x800710E0, "start only on AC power") and the analysis never
      ran on 2026-09-10 because the laptop was asleep at 16:10 and a task
      that cannot catch up simply skips the day.

    * The analysis runs at 16:15, after the 16:10 stop has fetched the final
      EOD report, rather than racing it.

    The 15-minute repetition on a weekly trigger cannot be expressed with
    New-ScheduledTaskTrigger's parameters; the Repetition block is built on a
    -Once trigger and copied across, which the scheduler accepts.

.NOTES
    Does not need elevation: the tasks run as the current user at the
    Limited run level (the user is in docker-users; nothing here needs admin).
    To remove all four:  .\setup_scheduler.ps1 -Remove
#>

param(
    [switch]$Remove
)

$ErrorActionPreference = 'Stop'

$ProjectDir  = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$Launcher    = Join-Path $ProjectDir 'scripts\run_hidden.vbs'
$StartScript = Join-Path $ProjectDir 'scripts\auto_start.bat'
$StopScript  = Join-Path $ProjectDir 'scripts\auto_stop.bat'
$Analyze     = Join-Path $ProjectDir 'analyze.ps1'
$Username    = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name

$TaskNames = @('StockMarketAI_Start', 'StockMarketAI_Stop', 'StockMarketAI_WeekendStop', 'StockAI Daily Analysis')

function Remove-TaskIfPresent([string]$Name) {
    try {
        Unregister-ScheduledTask -TaskName $Name -Confirm:$false -ErrorAction Stop
        Write-Host "  Removed $Name" -ForegroundColor Green
    } catch {
        Write-Host "  $Name not present" -ForegroundColor Gray
    }
}

# -- Remove mode -----------------------------------------------------------
if ($Remove) {
    Write-Host "Removing Stock Market AI scheduled tasks..." -ForegroundColor Yellow
    foreach ($n in $TaskNames) { Remove-TaskIfPresent $n }
    Write-Host "`nDone." -ForegroundColor Green
    exit 0
}

# -- Create mode -----------------------------------------------------------
Write-Host ""
Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Stock Market AI - Scheduler Setup" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan
Write-Host ""
Write-Host "  Project:  $ProjectDir"
Write-Host "  User:     $Username"
Write-Host ""

foreach ($f in @($Launcher, $StartScript, $StopScript, $Analyze)) {
    if (-not (Test-Path $f)) { Write-Error "Required file not found: $f"; exit 1 }
}
$LogDir = Join-Path $ProjectDir 'logs'
if (-not (Test-Path $LogDir)) { New-Item -ItemType Directory -Path $LogDir -Force | Out-Null }

# Every action: wscript.exe //B //Nologo "<launcher>" <program> [args...]
# Arguments containing spaces are quoted by the launcher itself.
function New-HiddenAction([string]$ArgLine) {
    New-ScheduledTaskAction -Execute 'wscript.exe' `
        -Argument ('//B //Nologo "{0}" {1}' -f $Launcher, $ArgLine) `
        -WorkingDirectory $ProjectDir
}

# Settings shared by every task: run on battery, keep running if the power
# is pulled, and CATCH UP if the trigger was missed (machine asleep or off).
function New-Settings([timespan]$TimeLimit, [int]$RestartCount, [switch]$WakeToRun) {
    $p = @{
        AllowStartIfOnBatteries   = $true
        DontStopIfGoingOnBatteries = $true
        StartWhenAvailable        = $true
        ExecutionTimeLimit        = $TimeLimit
        MultipleInstances         = 'IgnoreNew'
    }
    if ($RestartCount -gt 0) {
        $p.RestartCount    = $RestartCount
        $p.RestartInterval = (New-TimeSpan -Minutes 1)
    }
    if ($WakeToRun) { $p.WakeToRun = $true }
    New-ScheduledTaskSettingsSet @p
}

function Register-Task([string]$Name, $Action, $Triggers, $Settings, [string]$Description) {
    Remove-TaskIfPresent $Name | Out-Null
    Register-ScheduledTask -TaskName $Name -Action $Action -Trigger $Triggers `
        -Settings $Settings -Description $Description -User $Username | Out-Null
    Write-Host "  Registered $Name" -ForegroundColor Green
}

$Weekdays = @('Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday')

# -- StockMarketAI_Start ---------------------------------------------------
Write-Host "Creating StockMarketAI_Start..." -ForegroundColor Yellow
$startWeekly = New-ScheduledTaskTrigger -Weekly -DaysOfWeek $Weekdays -At '09:25'
$repeater    = New-ScheduledTaskTrigger -Once -At '09:25' `
                   -RepetitionInterval (New-TimeSpan -Minutes 15) `
                   -RepetitionDuration (New-TimeSpan -Hours 6)
$startWeekly.Repetition = $repeater.Repetition
$startLogon = New-ScheduledTaskTrigger -AtLogOn -User $Username
$startLogon.Delay = 'PT1M30S'
Register-Task 'StockMarketAI_Start' `
    (New-HiddenAction ('cmd.exe /c "{0}"' -f $StartScript)) `
    @($startWeekly, $startLogon) `
    (New-Settings (New-TimeSpan -Minutes 10) 2 -WakeToRun) `
    'Starts Stock Market AI before the open; repeats every 15 min until 15:25 and at logon so a late wake still starts it. The script itself refuses on weekends and outside 09:00-15:30.'

# -- StockMarketAI_Stop ----------------------------------------------------
Write-Host "Creating StockMarketAI_Stop..." -ForegroundColor Yellow
Register-Task 'StockMarketAI_Stop' `
    (New-HiddenAction ('cmd.exe /c "{0}"' -f $StopScript)) `
    (New-ScheduledTaskTrigger -Weekly -DaysOfWeek $Weekdays -At '16:10') `
    (New-Settings (New-TimeSpan -Minutes 5) 1) `
    'Stops Stock Market AI after market close: saves the backend log, fetches the EOD report, brings the containers down.'

# -- StockMarketAI_WeekendStop ---------------------------------------------
Write-Host "Creating StockMarketAI_WeekendStop..." -ForegroundColor Yellow
Register-Task 'StockMarketAI_WeekendStop' `
    (New-HiddenAction ('cmd.exe /c "{0}"' -f $StopScript)) `
    (New-ScheduledTaskTrigger -Weekly -DaysOfWeek 'Saturday' -At '00:05') `
    (New-Settings (New-TimeSpan -Minutes 5) 0) `
    'Belt-and-braces: bring the stack down at the start of every weekend regardless of how it was started.'

# -- StockAI Daily Analysis ------------------------------------------------
Write-Host "Creating StockAI Daily Analysis..." -ForegroundColor Yellow
Register-Task 'StockAI Daily Analysis' `
    (New-HiddenAction ('powershell.exe -NoProfile -ExecutionPolicy Bypass -File "{0}"' -f $Analyze)) `
    (New-ScheduledTaskTrigger -Weekly -DaysOfWeek $Weekdays -At '16:15') `
    (New-Settings (New-TimeSpan -Minutes 30) 0) `
    'Standalone daily/weekly analysis from the prediction log, after the 16:10 stop has fetched the final EOD report.'

# -- Verify ----------------------------------------------------------------
Write-Host ""
Write-Host "Verifying..." -ForegroundColor Yellow
foreach ($n in $TaskNames) {
    $t = Get-ScheduledTask -TaskName $n
    $i = $t | Get-ScheduledTaskInfo
    $s = $t.Settings
    Write-Host ("  {0,-26} next={1}  catchup={2} battery={3}" -f $n, $i.NextRunTime, $s.StartWhenAvailable, (-not $s.DisallowStartIfOnBatteries))
    foreach ($tr in $t.Triggers) {
        $desc = $tr.CimClass.CimClassName.Replace('MSFT_Task', '')
        if ($tr.StartBoundary) { $desc += ' ' + $tr.StartBoundary.Substring(11, 5) }
        if ($tr.Repetition.Interval) { $desc += ' every ' + $tr.Repetition.Interval + ' for ' + $tr.Repetition.Duration }
        if ($tr.Delay) { $desc += ' delay ' + $tr.Delay }
        Write-Host ("    {0}" -f $desc) -ForegroundColor Gray
    }
}

Write-Host ""
Write-Host "  Daily schedule (Mon-Fri, ET):"
Write-Host "    09:25  stack starts (retries every 15 min until 15:25, and at logon)"
Write-Host "    09:30  market opens, paper trading begins"
Write-Host "    15:55  daily skim banks the day and flattens"
Write-Host "    16:10  backend log saved, EOD report fetched, containers down"
Write-Host "    16:15  daily analysis"
Write-Host "    Sat 00:05  weekend backstop stop"
Write-Host ""
Write-Host "  Logs:    $LogDir"
Write-Host "  Remove:  .\setup_scheduler.ps1 -Remove" -ForegroundColor Gray
Write-Host ""
