# Live progress watcher for the grid run (run_grid.py).
#
# Counts finished games across the batch PGNs and shows percent, an
# INSTANTANEOUS rate (from the last two samples, so it tracks the real
# current speed as the run moves from fast low-depth configs to slow
# high-depth ones), and an ETA based on that current rate.
#
# Run it in PowerShell from anywhere:
#     powershell -ExecutionPolicy Bypass -File calibration\progress.ps1
# Ctrl-C to stop watching (does NOT stop the run).
#
# TOTAL is specific to the current grid (4032 configs, seed-swap: 19
# opponents are the gauntlet seeds, configs are non-seeds and don't play
# each other, batches of 120, 22 games/pair):
#   sum over batches of (C(19,2) + 19*K) * 22.  Auto batch size 93 (44
#   batches, command-length capped) => 1,850,904.
# Re-derive if you change the grid/batch/opponent/games settings.

$ErrorActionPreference = 'SilentlyContinue'
$gridDir = Join-Path $PSScriptRoot 'runs\grid'
$total   = 1850904
$refresh = 10   # seconds between samples

$prev = $null; $prevT = $null
Write-Host "watching $gridDir  (total $total games)  -- Ctrl-C to stop"
while ($true) {
    $done = 0
    foreach ($f in Get-ChildItem (Join-Path $gridDir 'batch_*.pgn')) {
        $done += (Select-String -Path $f.FullName -Pattern '[Result' -SimpleMatch).Count
    }
    $nowT = Get-Date
    $pct  = if ($total) { 100.0 * $done / $total } else { 0 }
    $bar  = ('#' * [int]($pct / 2)).PadRight(50, '-')

    $rateStr = ''
    if ($null -ne $prev) {
        $dt = ($nowT - $prevT).TotalSeconds
        if ($dt -gt 0) {
            $rate = ($done - $prev) / $dt
            if ($rate -gt 0) {
                $etaMin = ($total - $done) / $rate / 60.0
                $rateStr = ('  {0,4:N0} g/s  ETA {1,5:N0} min' -f $rate, $etaMin)
            }
        }
    }
    $line = ('[{0}] {1,5:N1}%  {2,7}/{3}{4}' -f $bar, $pct, $done, $total, $rateStr)
    Write-Host ("`r" + $line.PadRight(100)) -NoNewline

    if ($done -ge $total) { Write-Host "`nrun complete."; break }
    $prev = $done; $prevT = $nowT
    Start-Sleep -Seconds $refresh
}
