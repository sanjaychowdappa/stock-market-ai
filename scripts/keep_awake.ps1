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

    THE FIX, SECOND ATTEMPT (2026-10-09). The first version asked for
    ES_SYSTEM_REQUIRED only, on the reasoning that the screen may switch off
    so long as the CPU keeps running. MEASURED OVER THREE WEEKS, THAT DID
    NOTHING:

        before (09-08..09-21)  33 spans, 296 min asleep mid-session
        after  (09-22..10-09)  24 spans, 298 min asleep mid-session
        worst span after the "fix": 106 minutes, on 2026-10-07

    On a Modern Standby platform a System power request does not prevent
    standby entry — standby follows the screen going off, and only a DISPLAY
    request holds that open. ES_SYSTEM_REQUIRED is the right call on an S3
    machine and a no-op on this one, which is why it logged success every
    morning while changing nothing.

    So the request is now ES_CONTINUOUS | ES_SYSTEM_REQUIRED |
    ES_DISPLAY_REQUIRED. The cost is explicit and was accepted by the owner:
    the screen stays lit from 09:25 to 16:20 on a trading day, and the
    battery drains faster for it.

    DO NOT "optimise" the display flag away. It is the only flag here that
    does the job, and dropping it reproduces three weeks of blind sessions
    that look exactly like success in the log.

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
    $ES_CONTINUOUS       = [uint32]2147483648  # 0x80000000
    $ES_SYSTEM_REQUIRED  = [uint32]1           # 0x00000001
    $ES_DISPLAY_REQUIRED = [uint32]2           # 0x00000002 - the one that works here
    $flags = $ES_CONTINUOUS -bor $ES_SYSTEM_REQUIRED -bor $ES_DISPLAY_REQUIRED

    # Returns the PREVIOUS state, or 0 on failure. `powercfg /requests` would
    # show the request but needs elevation, so this return value is the
    # non-elevated proof that the call took.
    $prev = $api::SetThreadExecutionState($flags)
    if ($prev -eq 0) {
        Say "SetThreadExecutionState FAILED - the machine may still sleep mid-session"
        exit 1
    }
    Say ("holding the system awake until {0:HH:mm} (system+display, previous state 0x{1:X})" -f $until, $prev)

    # Report the PREVIOUS session's standby holes into today's log. The first
    # version of this script logged "holding the system awake" every morning
    # for three weeks while the machine slept for five hours a session, and
    # nothing in the project measured the thing the script existed to stop.
    # Now it reports itself, and a non-zero figure here is the signal that
    # the display request is not doing its job either.
    try {
        $since = (Get-Date).Date.AddDays(-4)
        $ev = Get-WinEvent -FilterHashtable @{LogName='System'; Id=506,507; StartTime=$since} -ErrorAction Stop |
              Sort-Object TimeCreated
        $open = $null; $spans = @()
        foreach ($e in $ev) {
            if ($e.Id -eq 506) { if (-not $open) { $open = $e.TimeCreated } }
            elseif ($open) { $spans += [pscustomobject]@{ Start = $open; Secs = [int]($e.TimeCreated - $open).TotalSeconds }; $open = $null }
        }
        $sess = $spans | Where-Object {
            $_.Start.DayOfWeek -notin 'Saturday', 'Sunday' -and
            (($_.Start.Hour -eq 9 -and $_.Start.Minute -ge 30) -or ($_.Start.Hour -ge 10 -and $_.Start.Hour -lt 16))
        }
        foreach ($g in ($sess | Group-Object { $_.Start.ToString('yyyy-MM-dd') } | Sort-Object Name)) {
            $mins = [int]((($g.Group | Measure-Object Secs -Sum).Sum) / 60)
            Say ("standby holes {0}: {1} span(s), {2} min of the session blind" -f $g.Name, $g.Count, $mins)
        }
        if (-not $sess) { Say "standby holes: none in the last 4 days of sessions" }
    } catch {
        Say "could not read standby history: $($_.Exception.Message)"
    }

    # The request lives on THIS thread, so the thread has to stay alive. The
    # loop also re-asserts it every 5 minutes: a request can be lost across
    # some power transitions, and re-asserting is free.
    while ((Get-Date) -lt $until) {
        Start-Sleep -Seconds 300
        [void]$api::SetThreadExecutionState($flags)
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
