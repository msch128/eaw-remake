# Soak test: long hidden runs (invisible window, no audio) that guard against leaks, log spam and slow frames.
#   .\tools\soak.ps1                          battle 5 min + galaxy 10 min + offscreen frame-time check
#   .\tools\soak.ps1 -BattleMinutes 1 -GalaxyMinutes 2 -Only galaxy     quick trial
# Samples process memory every 10 s (PrivateMemorySize64, WorkingSet64) and reads the app's SP_ASSET_LOG output
# (images / meshes / materials ...). Output goes to $Out (never into the repo). Exit code 1 on any FAIL.
# Thresholds: RSS > 8 GB; after a warm-up of 25% of the run (min 60 s) memory growth > 5%/min, or asset counts that
# rise in more than 80% of the later samples and end > 10% above the warm-up value (linear growth); any ERROR,
# "Out of Memory", "failed validation", panic; >20 identical WARN lines; offscreen galaxy p99 frame time >= 33 ms.
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\space-peace.exe'),
    [string]$Out = (Join-Path $env:TEMP 'space-peace-soak'),
    [int]$BattleMinutes = 5,
    [int]$GalaxyMinutes = 10,
    [string]$Only = '',
    [double]$MaxRssGB = 8,
    [double]$MaxGrowthPctPerMin = 5,
    [double]$MaxP99Ms = 33
)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path $Exe)) { Write-Host "exe not found: $Exe (run .\build.ps1 first, or pass -Exe)" -ForegroundColor Red; exit 2 }
New-Item -ItemType Directory -Force $Out | Out-Null
$env:APPDATA = Join-Path $Out 'appdata'
New-Item -ItemType Directory -Force $env:APPDATA | Out-Null
$env:SP_ASSET_LOG = '1'
Remove-Item Env:SP_INPUT_TEST -ErrorAction SilentlyContinue
$camp = 'GC_Full_Web_Way_Rebel'; $yavin = 'The Yavin System'
$ansi = [regex]'\x1B\[[0-9;]*m'

function Quote($a) { $a | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } } }

# Linear regression slope of y over x (x in minutes).
function Slope($xs, $ys) {
    $n = $xs.Count; if ($n -lt 3) { return 0 }
    $mx = ($xs | Measure-Object -Average).Average; $my = ($ys | Measure-Object -Average).Average
    $num = 0; $den = 0
    for ($i = 0; $i -lt $n; $i++) { $num += ($xs[$i] - $mx) * ($ys[$i] - $my); $den += ($xs[$i] - $mx) * ($xs[$i] - $mx) }
    if ($den -eq 0) { 0 } else { $num / $den }
}

function Invoke-Soak([string]$name, [string[]]$appArgs, [int]$minutes) {
    $log = Join-Path $Out "$name.log"
    Remove-Item $log, "$log.out", "$log.err" -ErrorAction SilentlyContinue
    $p = Start-Process -FilePath $Exe -ArgumentList (Quote $appArgs) -NoNewWindow -PassThru -RedirectStandardOutput "$log.out" -RedirectStandardError "$log.err"
    $null = $p.Handle
    $samples = @(); $sw = [Diagnostics.Stopwatch]::StartNew(); $limit = $minutes * 60
    while (-not $p.HasExited -and $sw.Elapsed.TotalSeconds -lt $limit) {
        Start-Sleep -Seconds 10
        try { $p.Refresh(); if (-not $p.HasExited) { $samples += [pscustomobject]@{ T = $sw.Elapsed.TotalMinutes; Priv = $p.PrivateMemorySize64 / 1MB; WS = $p.WorkingSet64 / 1MB } } } catch {}
    }
    $exited = $p.HasExited
    if (-not $exited) { try { $p.Kill() } catch {}; $p.WaitForExit(5000) | Out-Null }
    $code = if ($exited) { $p.ExitCode } else { 0 }
    $text = ''
    foreach ($f in "$log.out", "$log.err") { if (Test-Path $f) { $text += Get-Content $f -Raw -ErrorAction SilentlyContinue } }
    $text = $ansi.Replace($text, '')
    Set-Content -Path $log -Value $text
    Remove-Item "$log.out", "$log.err" -ErrorAction SilentlyContinue
    $lines = $text -split "`r?`n"
    $fails = @(); $notes = @()

    if ($exited -and $code -ne 0) { $fails += "exited early with code $code after $([math]::Round($sw.Elapsed.TotalSeconds)) s" }
    elseif ($exited) { $notes += "app exited by itself after $([math]::Round($sw.Elapsed.TotalSeconds)) s" }

    # Log hygiene.
    if ($text -match 'panicked at') { $fails += 'panic' }
    if ($text -cmatch '\bERROR\b') { $fails += ('ERROR x' + ([regex]::Matches($text, '\bERROR\b').Count)) }
    if ($text -match '(?i)out of memory') { $fails += 'out of memory' }
    if ($text -match 'failed validation') { $fails += 'failed validation' }
    $warnGroups = $lines | Where-Object { $_ -match '\bWARN\b' } | ForEach-Object { ($_ -replace '^\S+\s+', '') -replace '\d+(\.\d+)?', '#' } | Group-Object | Where-Object Count -gt 20
    foreach ($g in $warnGroups) { $fails += ("WARN spam x{0}: {1}" -f $g.Count, $g.Name.Substring(0, [math]::Min(70, $g.Name.Length))) }

    # Memory.
    $peakGB = if ($samples) { ($samples | Measure-Object Priv -Maximum).Maximum / 1024 } else { 0 }
    if ($peakGB -gt $MaxRssGB) { $fails += ("memory peak {0:N1} GB > {1} GB" -f $peakGB, $MaxRssGB) }
    $warm = [math]::Max(1.0, $minutes * 0.25)
    $late = @($samples | Where-Object T -ge $warm)
    $growth = 0.0
    if ($late.Count -ge 6) {
        $slope = Slope ($late | ForEach-Object T) ($late | ForEach-Object Priv)     # MB per minute
        $base = ($late | Select-Object -First 3 | Measure-Object Priv -Average).Average
        $growth = if ($base -gt 0) { 100.0 * $slope / $base } else { 0 }
        if ($growth -gt $MaxGrowthPctPerMin) { $fails += ("private memory grows {0:N1}%/min after warm-up" -f $growth) }
    } elseif ($minutes -ge 3) { $notes += 'too few samples after warm-up for the growth check' }

    # Asset counts from SP_ASSET_LOG ("assetlog frame=... images=N ... meshes=N std_mats=N particle_mats=N shader_buffers=N").
    $assets = @()
    foreach ($l in $lines) {
        if ($l -match 'assetlog frame=(\d+) images=(\d+).*?meshes=(\d+) std_mats=(\d+) particle_mats=(\d+) shader_buffers=(\d+)') {
            $m = $Matches
            $ts = if ($l -match '^(\d{4}-\d\d-\d\dT[\d:.]+)Z') { [datetime]::Parse($Matches[1]).ToUniversalTime() } else { $null }
            $assets += [pscustomobject]@{ Frame = [int]$m[1]; Time = $ts; images = [int]$m[2]; meshes = [int]$m[3]; std_mats = [int]$m[4]; particle_mats = [int]$m[5]; shader_buffers = [int]$m[6] }
        }
    }
    $assetSummary = @()
    if ($assets.Count -ge 6) {
        $skip = [int][math]::Ceiling($assets.Count * 0.25)
        $tail = @($assets | Select-Object -Skip $skip)
        foreach ($k in 'images', 'meshes', 'std_mats', 'particle_mats', 'shader_buffers') {
            $vals = @($tail | ForEach-Object { $_.$k })
            $first = $vals[0]; $last = $vals[-1]
            $ups = 0; for ($i = 1; $i -lt $vals.Count; $i++) { if ($vals[$i] -gt $vals[$i - 1]) { $ups++ } }
            $frac = $ups / [math]::Max(1, $vals.Count - 1)
            $assetSummary += "$k $first->$last"
            if ($frac -gt 0.8 -and $last -gt $first * 1.1 -and ($last - $first) -gt 20) { $fails += "$k grows linearly ($first -> $last, rising in $([int](100*$frac))% of samples)" }
        }
        # Mean frame time from the asset-log cadence (300 frames per entry); the hidden run has no per-frame stats.
        $times = @($tail | Where-Object { $_.Time })
        if ($times.Count -ge 3) {
            $ms = for ($i = 1; $i -lt $times.Count; $i++) { ($times[$i].Time - $times[$i - 1].Time).TotalMilliseconds / ($times[$i].Frame - $times[$i - 1].Frame) }
            $sorted = @($ms | Sort-Object)
            $notes += ("frame time (300-frame means): mean {0:N1} ms, worst {1:N1} ms" -f ($ms | Measure-Object -Average).Average, $sorted[-1])
        }
        # Rolling p99 per 300-frame window, logged by the app on the "assetlog rss=" line; gate on the median window.
        $p99s = @(); foreach ($l in $lines) { if ($l -match 'assetlog rss=.*frame_ms mean=([\d.]+) p99=([\d.]+) max=([\d.]+)') { $p99s += [double]$Matches[2] } }
        $p99s = @($p99s | Select-Object -Skip ([int][math]::Ceiling($p99s.Count * 0.25)))
        if ($p99s.Count -ge 3) {
            $ps = @($p99s | Sort-Object); $med = $ps[[int]($ps.Count / 2)]
            $notes += ("p99 per 300 frames: median {0:N1} ms, worst {1:N1} ms" -f $med, $ps[-1])
            if ($med -ge $MaxP99Ms) { $fails += ("p99 {0:N1} ms >= {1} ms" -f $med, $MaxP99Ms) }
        } else { $notes += 'no p99 entries in the asset log (older exe?)' }
    } else { $fails += "no assetlog output ($($assets.Count) entries): SP_ASSET_LOG not honoured or run too short" }

    [pscustomobject]@{
        Scenario = $name; Result = $(if ($fails) { 'FAIL' } else { 'ok' }); Minutes = [math]::Round($sw.Elapsed.TotalMinutes, 1)
        PeakGB = [math]::Round($peakGB, 2); 'Growth%/min' = [math]::Round($growth, 1)
        Assets = ($assetSummary -join '; '); Notes = ((@($fails) + @($notes)) -join ' | ')
    }
}

$rows = @()
if (-not $Only -or 'battle' -like "*$Only*") {
    # Fight at real speed (no --battle-auto), hidden window, vsync off so the loop is not capped.
    $rows += Invoke-Soak 'battle-hidden' @('--hidden', '--no-vsync', '--exit-after', '100000000', '--volume', '0', '--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'fight', '--battle-speed', '1') $BattleMinutes
}
if (-not $Only -or 'galaxy' -like "*$Only*") {
    $rows += Invoke-Soak 'galaxy-hidden' @('--hidden', '--no-vsync', '--exit-after', '100000000', '--volume', '0', '--campaign', $camp) $GalaxyMinutes
}
if (-not $Only -or 'frametime' -like "*$Only*") {
    # Per-frame percentiles exist only in offscreen mode ("offscreen: loop frame time ... p99"), so the p99 gate uses it.
    $png = Join-Path $Out 'frametime.png'; $log = Join-Path $Out 'frametime.log'
    $a = Quote @('--offscreen', $png, '--campaign', $camp, '--select', $yavin, '--zoom', '0.3', '--frames', '1500')
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process -FilePath $Exe -ArgumentList $a -NoNewWindow -PassThru -RedirectStandardOutput "$log.out" -RedirectStandardError "$log.err"
    $null = $p.Handle
    if (-not $p.WaitForExit(600000)) { try { $p.Kill() } catch {} }
    $text = ''; foreach ($f in "$log.out", "$log.err") { if (Test-Path $f) { $text += Get-Content $f -Raw } }
    $text = $ansi.Replace($text, ''); Set-Content $log $text; Remove-Item "$log.out", "$log.err" -ErrorAction SilentlyContinue
    $fails = @(); $note = ''
    if ($text -match 'loop frame time over (\d+) frames: mean ([\d.]+) ms, p50 ([\d.]+), p95 ([\d.]+), p99 ([\d.]+), max ([\d.]+)') {
        $note = "mean $($Matches[2]) p95 $($Matches[4]) p99 $($Matches[5]) max $($Matches[6]) ms"
        if ([double]$Matches[5] -ge $MaxP99Ms) { $fails += "p99 $($Matches[5]) ms >= $MaxP99Ms ms" }
    } else { $fails += 'no frame-time line in log' }
    if ($text -cmatch 'panicked at|\bERROR\b') { $fails += 'panic/ERROR in log' }
    $rows += [pscustomobject]@{ Scenario = 'frametime-offscreen'; Result = $(if ($fails) { 'FAIL' } else { 'ok' }); Minutes = [math]::Round($sw.Elapsed.TotalMinutes, 1); PeakGB = ''; 'Growth%/min' = ''; Assets = ''; Notes = ((@($fails) + $note) -join ' | ') }
}

$rows | Format-Table -AutoSize -Wrap
$failed = @($rows | Where-Object Result -eq 'FAIL').Count
Write-Host ("{0} soak runs, {1} failed. Logs: {2}" -f $rows.Count, $failed, $Out)
exit $(if ($failed) { 1 } else { 0 })
