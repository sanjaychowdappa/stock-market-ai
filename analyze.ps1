<#
  analyze.ps1 - Standalone swing-trading analysis for stock-market-ai.

  Runs WITHOUT Claude. Reads reports/prediction_accuracy.jsonl and produces
  the daily + weekly report: win rate, expectancy after fees, per-symbol
  breakdown, exit reasons, and the all-important random-baseline (Model C)
  comparison that tells you whether the signals actually have an edge.

  Every run also saves a dated copy to reports/analysis/analysis_<date>.txt
  so you build a browsable history.

  Usage:
    .\analyze.ps1                 # today + last 7 days
    .\analyze.ps1 -Days 14        # today + last 14 days
    .\analyze.ps1 -Date 2026-06-22

  Schedule it (optional): Windows Task Scheduler -> daily at 4:10pm ET ->
    Program:  powershell.exe
    Args:     -ExecutionPolicy Bypass -File "C:\Users\sanja\stock-market-ai\analyze.ps1"
#>
param(
    [int]$Days = 7,
    [string]$Date = (Get-Date -Format 'yyyy-MM-dd')
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$jsonl = Join-Path $root 'reports\prediction_accuracy.jsonl'

# Output buffer: everything goes here, then we print + save it.
$report = New-Object System.Collections.Generic.List[string]
function A([string]$s = '') { $report.Add($s) | Out-Null }

if (-not (Test-Path $jsonl)) {
    Write-Host "No log file found at $jsonl - has the system traded yet?" -ForegroundColor Yellow
    exit 0
}

# ---- Load and classify log lines -------------------------------------------
$lines = Get-Content $jsonl -Encoding utf8 | Where-Object { $_.Trim() -ne '' }
$objs = foreach ($l in $lines) { try { $l | ConvertFrom-Json } catch { } }

$realTrades = $objs | Where-Object { $null -ne $_.pnl -and $_.exit_reason -and -not $_.type }
$shadowSells = $objs | Where-Object { $_.type -eq 'shadow_trade' -and $_.action -eq 'SELL' }

function Summarize($trades, $label) {
    A ''
    A "=== $label ==="
    if (-not $trades -or $trades.Count -eq 0) { A "  (no closed trades)"; return }

    $n = $trades.Count
    $wins = @($trades | Where-Object { $_.pnl -gt 0 })
    $losses = @($trades | Where-Object { $_.pnl -le 0 })
    $gross = ($trades | Measure-Object -Property pnl -Sum).Sum
    $winRate = if ($n) { [math]::Round($wins.Count / $n * 100, 1) } else { 0 }
    $avgWin = if ($wins.Count) { ($wins | Measure-Object -Property pnl -Average).Average } else { 0 }
    $avgLoss = if ($losses.Count) { ($losses | Measure-Object -Property pnl -Average).Average } else { 0 }
    $expectancy = if ($n) { $gross / $n } else { 0 }
    $estFeePerTrade = 0.0015 * 75   # ~0.15% of a ~$75 position; adjust if sizing changes
    $netExpectancy = $expectancy - $estFeePerTrade

    A ("  Trades:       {0}   (W {1} / L {2})" -f $n, $wins.Count, $losses.Count)
    A ("  Win rate:     {0}%" -f $winRate)
    A ('  Gross P/L:    ${0}' -f [math]::Round($gross, 4))
    A (('  Avg win:      ${0}' -f [math]::Round($avgWin,4)) + ('    Avg loss: ${0}' -f [math]::Round($avgLoss,4)))
    A (('  Expectancy:   ${0}/trade gross  ->  ~' -f [math]::Round($expectancy,4)) + ('${0}/trade after est. fees' -f [math]::Round($netExpectancy,4)))

    A "  Per symbol:"
    $trades | Group-Object symbol | Sort-Object { ($_.Group | Measure-Object pnl -Sum).Sum } -Descending | ForEach-Object {
        $p = ($_.Group | Measure-Object -Property pnl -Sum).Sum
        $w = @($_.Group | Where-Object { $_.pnl -gt 0 }).Count
        A (("    {0,-6} {1,2} trades  W{2,-2}  " -f $_.Name, $_.Count, $w) + ('${0}' -f [math]::Round($p,4)))
    }

    A "  Exit reasons:"
    $trades | ForEach-Object { ($_.exit_reason -split '\(')[0] } | Group-Object | Sort-Object Count -Descending | ForEach-Object {
        A ("    {0,-18} {1}" -f $_.Name, $_.Count)
    }
}

# ---- Date filters ----------------------------------------------------------
$cutoff = (Get-Date $Date).AddDays(-([math]::Max(0,$Days-1))).Date
function InWindow($t) {
    if (-not $t.timestamp) { return $false }
    try { return ([datetime]$t.timestamp).Date -ge $cutoff } catch { return $false }
}
function OnDate($t, $d) {
    if (-not $t.timestamp) { return $false }
    try { return ([datetime]$t.timestamp).ToString('yyyy-MM-dd') -eq $d } catch { return $false }
}

A "############################################################"
A "  STOCK-MARKET-AI  -  SWING ANALYSIS"
A ("  Generated: {0}   |   Report date: {1}   |   Window: last {2} days" -f (Get-Date -Format 'yyyy-MM-dd HH:mm'), $Date, $Days)
A "############################################################"

$todayReal = @($realTrades | Where-Object { OnDate $_ $Date })
Summarize $todayReal "REAL TRADER - TODAY ($Date)"

$winReal = @($realTrades | Where-Object { InWindow $_ })
Summarize $winReal "REAL TRADER - LAST $Days DAYS"

# ---- The key test: real vs random baseline (Model C) -----------------------
A ''
A "=== EDGE TEST: real signals vs random baseline (last $Days days) ==="
$winShadow = @($shadowSells | Where-Object { InWindow $_ })
$byModel = $winShadow | Group-Object model_id
$realExp = if ($winReal.Count) { ($winReal | Measure-Object pnl -Sum).Sum / $winReal.Count } else { 0 }
A (("  {0,-24} expectancy " -f 'REAL signals') + ('${0}' -f [math]::Round($realExp,4)) + ("/trade  ({0} trades)" -f $winReal.Count))
foreach ($m in $byModel) {
    $exp = ($m.Group | Measure-Object pnl -Sum).Sum / $m.Count
    $tag = if ($m.Name -match 'random') { '  <-- the bar to beat' } else { '' }
    A (("  {0,-24} expectancy " -f $m.Name) + ('${0}' -f [math]::Round($exp,4)) + ("/trade  ({0} trades){1}" -f $m.Count, $tag))
}
$randModel = $byModel | Where-Object { $_.Name -match 'random' }
if ($randModel -and $winReal.Count) {
    $randExp = ($randModel.Group | Measure-Object pnl -Sum).Sum / $randModel.Count
    if ($realExp -gt $randExp) {
        A ('  VERDICT: real beats random by ${0}/trade - signals show edge (need bigger sample to confirm)' -f [math]::Round($realExp-$randExp,4))
    } else {
        A "  VERDICT: real does NOT beat random - no demonstrated edge yet"
    }
} else {
    A "  (not enough random-baseline trades yet to judge)"
}

# ---- Live portfolio (only if the system is running) ------------------------
A ''
try {
    $st = Invoke-RestMethod -Uri 'http://localhost:8000/api/daily/status' -TimeoutSec 3
    A (('LIVE: portfolio ${0}' -f $st.portfolio.current_value) + ('  (P/L ${0})' -f $st.portfolio.pnl) + ("  running={0}" -f $st.running))
} catch {
    A "LIVE: backend not reachable (Docker stopped?) - analysis above is from logs."
}

# ---- Print to console + save dated archive ---------------------------------
$report | ForEach-Object { Write-Host $_ }

$outDir = Join-Path $root 'reports\analysis'
if (-not (Test-Path $outDir)) { New-Item -ItemType Directory -Path $outDir | Out-Null }
$outFile = Join-Path $outDir ("analysis_{0}.txt" -f $Date)
$report | Set-Content -Path $outFile -Encoding utf8
Write-Host ""
Write-Host "Saved report to $outFile" -ForegroundColor Green
