# Sound audit without listening: runs the galaxy and the battle smoke offscreen against an empty audio device with the
# sound ledger on, then checks every sound event the run fired:
#   - the event exists in the data, has samples and the files exist in the game file system (or the gap is listed in
#     crates\alamo-audio\tests\known_audio_gaps.txt: blank / missing data of the Remake itself);
#   - the expected sound categories were heard (music scene, weapon fire, impacts, deaths, unit responses, advisor line).
#   .\tools\audio-audit.ps1 [-Exe <eaw-remake.exe>] [-Out <dir>]
# Safe while the user is here: offscreen, --volume 0, scratch APPDATA, no window, no audio device.
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\eaw-remake.exe'),
    [string]$Out = (Join-Path $env:TEMP 'space-peace-audio-audit'),
    [int]$TimeoutSeconds = 420
)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path $Exe)) { Write-Host "exe not found: $Exe" -ForegroundColor Red; exit 2 }
New-Item -ItemType Directory -Force $Out | Out-Null
$env:APPDATA = Join-Path $Out 'appdata'
New-Item -ItemType Directory -Force $env:APPDATA | Out-Null
$camp = 'GC_Full_Web_Way_Rebel'
$yavin = 'The Yavin System'
$known = @{}
Get-Content (Join-Path $PSScriptRoot '..\crates\alamo-audio\tests\known_audio_gaps.txt') -ErrorAction SilentlyContinue | ForEach-Object {
    $known[($_ -split ': ', 2)[0].ToLowerInvariant()] = $true
}

# name, args, regexes that must match some "<Kind> <event>" line of the ledger
$runs = @(
    @('galaxy', @('--campaign', $camp, '--select', $yavin, '--zoom', '0.15', '--build', 'Squadron', '--move', 'Hoth', '--weeks', '4', '--frames', '120'),
        @('^Music Galactic_Map_\w+_Music_Event$', '^Sfx Unit_Select_', '^Sfx \wHD_Battle_Pending$', '^Sfx Galactic_Fleet_Hyperspace$')),
    @('battle', @('--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'fight', '--battle-orders', '--battle-speed', '30', '--frames', '5'),
        @('^Music Space_Map_\w+_Battle_Music_Event$', '^Music \w+_Win_Tactical_Event$', '^Sfx \w+_Fire_', '^Sfx SFX_\w+_Detonation$', '^Sfx \w+_Death_SFX$', '^Sfx Unit_Select_', '^Sfx \wHD_Battle_End$'))
)
$failed = 0
foreach ($r in $runs) {
    $name = $r[0]
    $ledger = Join-Path $Out "$name.tsv"
    Remove-Item $ledger -ErrorAction SilentlyContinue
    $env:SP_AUDIO_LEDGER = $ledger
    $args = @('--offscreen', (Join-Path $Out "$name.png"), '--volume', '0') + $r[1] | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }
    $p = Start-Process -FilePath $Exe -ArgumentList $args -NoNewWindow -PassThru -RedirectStandardOutput "$Out\$name.out" -RedirectStandardError "$Out\$name.err"
    $null = $p.Handle
    if (-not $p.WaitForExit($TimeoutSeconds * 1000)) { try { $p.Kill() } catch {}; Write-Host "${name}: timeout" -ForegroundColor Red; $failed++; continue }
    if ($p.ExitCode -ne 0) { Write-Host "${name}: exit code $($p.ExitCode)" -ForegroundColor Red; $failed++; continue }
    if (-not (Test-Path $ledger)) { Write-Host "${name}: no ledger written" -ForegroundColor Red; $failed++; continue }
    $rows = Import-Csv $ledger -Delimiter "`t"
    $played = $rows | Where-Object { $_.outcome -eq 'Played' } | Group-Object { "$($_.kind) $($_.event)" }
    $problems = $rows | Where-Object { $_.outcome -notin 'Played','Dropped' } | Group-Object { "$($_.outcome) $($_.kind) $($_.event) $($_.sample)" }
    $bad = @($problems | Where-Object { $_.Group[0].outcome -eq 'UnknownEvent' -or -not $known.ContainsKey($_.Group[0].event.ToLowerInvariant()) })
    $missing = @($r[2] | Where-Object { $rx = $_; -not ($played | Where-Object { $_.Name -match $rx }) })
    Write-Host ("{0}: {1} requests, {2} distinct sounds played, {3} problem kinds ({4} unexpected), {5} expected categories missing" -f $name, $rows.Count, $played.Count, @($problems).Count, $bad.Count, $missing.Count)
    foreach ($b in $bad) { Write-Host "    UNEXPECTED $($b.Name) x$($b.Count)" -ForegroundColor Red }
    foreach ($m in $missing) { Write-Host "    NOT HEARD  $m" -ForegroundColor Red }
    $played | Sort-Object Count -Descending | Select-Object -First 8 | ForEach-Object { Write-Host ("    {0,5}  {1}" -f $_.Count, $_.Name) }
    if ($bad.Count -gt 0 -or $missing.Count -gt 0) { $failed++ }
}
if ($failed) { Write-Host "audio audit: $failed run(s) FAILED" -ForegroundColor Red; exit 1 }
Write-Host 'audio audit: PASS' -ForegroundColor Green
