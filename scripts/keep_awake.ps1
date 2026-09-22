<#
.SYNOPSIS
    Holds the system awake for the trading session, then releases it.

.DESCRIPTION
    THE BUG THIS FIXES. On 2026-09-14 the 3:55pm skim did not run although
    the backend was alive at 16:10, and the cause could not be found: the
    Windows System log showed no sleep event. It showed no CLASSIC sleep
    event — this laptop uses Modern Standby, which logs Kernel-Power 506
    (enter) and 507 (exit), not the 42 everyone greps for.

        2026-09-14  15:51:00 enter -> 16:05:13 exit   (skim window)
        2026-09-21  15:47:00 enter -> 16:08:25 exit   (skim window)

    It happened 2-5 times on every trading day of the past fortnight. While
    the system is in standby the container is frozen: no ticks arrive, no
    exit rule is evaluated, no stop can fire, and a position rides whatever
    the market does until the machine wakes. The missed skims were the
    visible symptom of a much larger hole.

    Why it happens: "Sleep after" is 0 (never) on AC but 300 seconds on
    battery. On 2026-09-21 at 15:41 the power source changed to battery and
    the machine suspended five minutes later.

    THE FIX. SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED)
    tells Windows the system must stay in the working state — the same
    request a media player holds while playing. Deliberately NOT
    ES_DISPLAY_REQUIRED: the screen may switch off, only the CPU must keep
    running.

    This is preferred over editing the power plan's DC timeout because the
    request DIES WITH THE PROCESS. If this script is killed, or the machine
    reboots, Windows returns to normal sleep behaviour on its own — there is
    no persistent setting left behind to drain the battery on a Sunday.

    Lifetime: exits at UntilHhmm (default 16:20, after the 16:10 stop and
    the 16:15 analysis), or immediately if the market is shut. A named mutex
    keeps one instance: auto_start.bat runs every 15 minutes and would
    otherwise pile up twenty-five of them a day.

.PARAMETER UntilHhmm
    Local time to release at, HHmm. Default 1620.

.PARAMETER Force
    Skip the market-hours guard (for testing).

.NOTES
    `powercfg /requests` lists held requests but needs an elevated prompt.
    Without elevation, the proof is the non-zero return value of
    SetThreadExecutionState (the previous state), which this logs, plus the
    absence of Kernel-Power 506 while it holds.
#>

param(
    [string]$UntilHhmm = '1620',
    [switch]$Force
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$log  = Join-Path $root ("logs\auto_{0}.log" -f (Get-Date -Format 'yyyyMMdd'))

function Say([string]$msg) {
    $line = "[{0}] KEEP_AWAKE {1}" -f (Get-Date -Format 'dd-MM-yyyy HH:mm:ss.ff'), $msg
    try { Add-Content -Path $log -Value $line -Encoding ascii } catch { }
    Write-Host $line
}

# -- One instance ----------------------------------------------------------
$created = $false
$mutex = New-Object System.Threading.Mutex($true, 'Global\StockMarketAI_KeepAwake', [ref]$created)
if (-not $created) { exit 0 }

try {
    # -- Guards: weekday, and inside the session -----------------------------
    $now = Get-Date
    if (-not $Force) {
        if ($now.DayOfWeek -in 'Saturday', 'Sunday') { Say "weekend - not holding"; exit 0 }
        $mins = $now.Hour * 60 + $now.Minute
        if ($mins -lt 540 -or $mins -gt 930) { Say "outside 09:00-15:30 - not holding"; exit 0 }
    }

    $until = [datetime]::ParseExact($UntilHhmm, 'HHmm', $null)
    $until = $now.Date.AddHours($until.Hour).AddMinutes($until.Minute)
    if ($until -le $now) { Say "release time $UntilHhmm already passed - not holding"; exit 0 }

    # -- Hold ----------------------------------------------------------------
    $sig = @'
[DllImport("kernel32.dll", SetLastError = true)]
public static extern uint SetThreadExecutionState(uint esFlags);
'@
    $api = Add-Type -MemberDefinition $sig -Name 'PowerApi' -Namespace 'StockAI' -PassThru

    # DECIMAL, not 0x80000000. Windows PowerShell 5.1 parses a hex literal as
    # Int32, so [uint32]0x80000000 is a cast of -2147483648 and throws
    # "Value was either too large or too small for a UInt32" — which killed
    # this script silently on its first run, holding nothing.
    $ES_CONTINUOUS      = [uint32]2147483648   # 0x80000000
    $ES_SYSTEM_REQUIRED = [uint32]1            # 0x00000001
    $flags = $ES_CONTINUOUS -bor $ES_SYSTEM_REQUIRED

    # Returns the PREVIOUS state, or 0 on failure. `powercfg /requests` would
    # show the request but needs elevation, so this return value is the
    # non-elevated proof that the call took.
    $prev = $api::SetThreadExecutionState($flags)
    if ($prev -eq 0) {
        Say "SetThreadExecutionState FAILED - the machine may still sleep mid-session"
        exit 1
    }
    Say ("holding the system awake until {0:HH:mm} (previous state 0x{1:X})" -f $until, $prev)

    # The request lives on THIS thread, so the thread has to stay alive. The
    # loop also re-asserts it every 5 minutes: a request can be lost across
    # some power transitions, and re-asserting is free.
    while ((Get-Date) -lt $until) {
        Start-Sleep -Seconds 300
        [void]$api::SetThreadExecutionState($ES_CONTINUOUS -bor $ES_SYSTEM_REQUIRED)
    }
    Say "released - normal sleep behaviour resumes"
}
finally {
    # Clear the request explicitly. Windows would clear it when the process
    # exits anyway; doing it here means a crash in the loop above still ends
    # with the machine free to sleep.
    try { [void]$api::SetThreadExecutionState([uint32]2147483648) } catch { }
    $mutex.ReleaseMutex()
    $mutex.Dispose()
}
