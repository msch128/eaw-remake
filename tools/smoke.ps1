# Smoke test: runs the release exe offscreen (no window, no audio) through the key scenarios and prints a pass/fail table.
#   .\tools\smoke.ps1                      all scenarios
#   .\tools\smoke.ps1 -Only superweapon    scenarios whose name contains the text
# Needs the game and the Remake mod installed. Output (PNGs, logs, saves) goes to $Out, never into the repo.
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\space-peace.exe'),
    [string]$Out = (Join-Path $env:TEMP 'space-peace-smoke'),
    [string]$Only = '',
    [int]$TimeoutSeconds = 420
)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path $Exe)) { Write-Host "exe not found: $Exe (run .\build.ps1 first, or pass -Exe)" -ForegroundColor Red; exit 2 }
New-Item -ItemType Directory -Force $Out | Out-Null
$env:APPDATA = Join-Path $Out 'appdata'            # saves, settings and logs of the test runs stay here
New-Item -ItemType Directory -Force $env:APPDATA | Out-Null
$empty = Join-Path $Out 'empty-game-dir'
New-Item -ItemType Directory -Force $empty | Out-Null
$camp = 'GC_Full_Web_Way_Rebel'
$yavin = 'The Yavin System'
$save = Join-Path $Out 'smoke-save.json'

# name, argument list, regex that must appear in the log (empty = none)
$scenarios = @(
    @('via-menu-to-galaxy',  @('--via-menu', '--campaign', $camp, '--frames', '300'), 'human faction'),
    @('menu-title-scene',    @('--screen', 'menu', '--frames', '100'), 'title scene'),
    @('campaign-select',     @('--screen', 'campaigns', '--frames', '60'), ''),
    @('options-screen',      @('--screen', 'options', '--frames', '60'), ''),
    @('pause-screen',        @('--campaign', $camp, '--screen', 'pause', '--frames', '80'), ''),
    @('missing-game',        @('--game-dir', $empty, '--screen', 'menu', '--frames', '60'), 'FALLING BACK'),
    @('build-and-move',      @('--campaign', $camp, '--select', $yavin, '--zoom', '0.15', '--build', 'Squadron', '--move', 'Hoth', '--weeks', '4', '--frames', '120'), 'script: done'),
    @('queue-cancel',        @('--campaign', $camp, '--select', $yavin, '--build', 'Squadron', '--weeks', '0', '--click', 'queue00', '--frames', '300'), "clicking .queue00"),
    @('save-slot',           @('--campaign', $camp, '--slots', 'save', '--slot-click', '1', '--frames', '250'), 'saves: wrote'),
    @('save-to-file',        @('--campaign', $camp, '--weeks', '2', '--save-to', $save, '--frames', '60'), 'saves: wrote'),
    @('load-from-file',      @('--campaign', $camp, '--load-from', $save, '--frames', '60'), 'saves: loaded'),
    @('difficulty-classic',  @('--campaign', $camp, '--classic-ai', '--weeks', '3', '--frames', '60'), 'script: done'),
    @('force-end-win',       @('--campaign', $camp, '--force-end', 'win', '--frames', '300'), ''),
    @('force-end-lose-menu', @('--campaign', $camp, '--force-end', 'lose-menu', '--frames', '500'), 'Main Menu'),
    @('superweapon',         @('--campaign', $camp, '--select', $yavin, '--zoom', '0.1', '--grant-unit', 'Death_Star', '--at', 'Arda', '--weeks', '0', '--fire', 'confirm', '--frames', '330'), 'destroyed'),
    @('battle-auto-resolve', @('--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'auto', '--frames', '200'), 'clicking .Auto'),
    @('battle-fight-orders', @('--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'fight', '--battle-orders', '--battle-speed', '30', '--frames', '5'), 'battle: over'),
    @('battle-reinforce',    @('--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'fight', '--battle-reserve', '3', '--battle-click', 'b_reinforcement,r_0000', '--battle-speed', '30', '--frames', '5'), 'simulation accepted')
)

$rows = @()
foreach ($s in $scenarios) {
    $name = $s[0]
    if ($Only -and $name -notlike "*$Only*") { continue }
    $png = Join-Path $Out "$name.png"
    $log = Join-Path $Out "$name.log"
    Remove-Item $png, $log -ErrorAction SilentlyContinue
    # Start-Process joins the list with spaces: quote what has spaces (planet names, paths).
    $args = (@('--offscreen', $png) + $s[1]) | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process -FilePath $Exe -ArgumentList $args -NoNewWindow -PassThru -RedirectStandardOutput "$log.out" -RedirectStandardError "$log.err"
    $null = $p.Handle   # keeps the process handle so ExitCode is readable after WaitForExit
    $timedOut = -not $p.WaitForExit($TimeoutSeconds * 1000)
    if ($timedOut) { try { $p.Kill() } catch {} }
    $sw.Stop()
    $text = ''
    foreach ($f in "$log.out", "$log.err") { if (Test-Path $f) { $text += Get-Content $f -Raw -ErrorAction SilentlyContinue } }
    Set-Content -Path $log -Value $text
    Remove-Item "$log.out", "$log.err" -ErrorAction SilentlyContinue
    $problems = @()
    if ($timedOut) { $problems += "timeout ${TimeoutSeconds}s" }
    elseif ($p.ExitCode -ne 0) { $problems += "exit code $($p.ExitCode)" }
    if ($text -match 'panicked at') { $problems += 'panic' }
    if ($text -match '\bERROR\b') { $problems += 'ERROR in log' }
    # Battle scenarios save their mid-battle shot as <name>_battle.png; every other scenario must write its picture.
    $shot = if ($name -like 'battle-fight*' -or $name -like 'battle-reinforce*') { Join-Path $Out "${name}_battle.png" } else { $png }
    if (-not (Test-Path $shot)) { $problems += 'no picture' }
    if ($s[2] -and $text -notmatch $s[2]) { $problems += "missing '$($s[2])'" }
    $rows += [pscustomobject]@{ Scenario = $name; Result = $(if ($problems) { 'FAIL' } else { 'ok' }); Seconds = [math]::Round($sw.Elapsed.TotalSeconds); Notes = ($problems -join ', ') }
}
$rows | Format-Table -AutoSize
$failed = @($rows | Where-Object Result -eq 'FAIL').Count
Write-Host ("{0} scenarios, {1} failed. Pictures and logs: {2}" -f $rows.Count, $failed, $Out)
exit $(if ($failed) { 1 } else { 0 })
