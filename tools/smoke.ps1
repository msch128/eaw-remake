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
    @('battle-auto-resolve', @('--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'auto', '--frames', '200'), 'battle result dialog shown: .* at .* units survived [(]lost [0-9]+[)]'),
    @('battle-fight-orders', @('--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'fight', '--battle-orders', '--battle-speed', '30', '--frames', '5'), 'battle: over'),
    # The real windowed plugin set (audio, window systems) with an invisible window and zero volume; no picture is made.
    @('windowed-menu',       @('--hidden', '--volume', '0', '--frames', '400'), 'install'),
    @('windowed-galaxy',     @('--hidden', '--volume', '0', '--campaign', $camp, '--frames', '700'), 'human faction'),
    @('windowed-battle',     @('--hidden', '--no-vsync', '--exit-after', '9000', '--volume', '0', '--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'fight', '--battle-auto', '--battle-speed', '30'), 'battle: over'),
    # Real input path of the tactical screen: injected window events (select, order, bar button, deselect); see battle-view/src/input_test.rs.
    @('windowed-battle-input', @('--hidden', '--no-vsync', '--exit-after', '9000', '--volume', '0', '--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'fight', '--battle-reserve', '3', '--battle-speed', '1'), 'input-test: [0-9]+ checks PASS'),
    # Real pointer path of the galaxy screens: battle banner, ESC menu (save, slots, options, load, main menu); see space-peace/src/input_test.rs.
    @('windowed-ui-pointer', @('--hidden', '--no-vsync', '--exit-after', '8000', '--volume', '0', '--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth'), 'ui-test: [0-9]+ checks PASS'),
    # F10 equivalent (--battle-abort-after): the half-fought battle is abandoned and the galaxy keeps the losses so far (some side lost units).
    @('windowed-battle-f10-abandon', @('--hidden', '--no-vsync', '--exit-after', '6000', '--volume', '0', '--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'fight', '--battle-auto', '--battle-speed', '30', '--battle-abort-after', '90'), 'abandoned at t=\d+s, units lost per side \[[0-9, ]*[1-9]'),
    # Autocast proof: an X4 (Sensor_Jamming with autofire) joins the fleet; input_test right-clicks its ability button.
    @('windowed-battle-input-autocast', @('--hidden', '--no-vsync', '--exit-after', '9000', '--volume', '0', '--campaign', $camp, '--select', $yavin, '--grant-unit', 'X4', '--at', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'fight', '--battle-reserve', '3', '--battle-speed', '1'), 'input-test: [0-9]+ checks PASS'),
    # Audio feel (SP_AUDIO_LOG event log, volume 0): a move order right after the select line cuts it and plays the move line; battle music follows the fight and the advisor reports losses.
    @('windowed-battle-input-audio-orders', @('--hidden', '--no-vsync', '--exit-after', '9000', '--volume', '0', '--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'fight', '--battle-reserve', '3', '--battle-speed', '1'), '(?s)(?=.*audio sfx CUT unit_select_\w+ for Unit_Move_\w+)(?=.*audio advisor ReinforcementsAvailable)'),
    @('windowed-battle-audio-music-advisor', @('--hidden', '--no-vsync', '--exit-after', '5000', '--volume', '0', '--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'fight', '--battle-speed', '30'), '(?s)(?=.*Space_Map_\w+_Ambient_Music_Event)(?=.*Space_Map_\w+_Battle_Music_Event)(?=.*audio advisor UnitDestroyed)'),
    # Audio pass 2 (SP_AUDIO_LOG, volume 0). Positional battle SFX: spatialized with a distance / attenuation, engine loops from the unit XML; music: loading loop -> ambient -> battle -> win stinger.
    @('windowed-battle-audio-positional', @('--hidden', '--no-vsync', '--exit-after', '9000', '--volume', '0', '--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'fight', '--battle-speed', '30'), '(?s)(?=.*audio sfx PLAY \w+ \(unit_response=false spatial=true dist=\d+ att=0\.\d+)(?=.*audio emitter \S+ -> Unit_\w+_Idle_Engine_Loop)(?=.*audio emitter \S+ -> Unit_\w+_Moving_Engine_Loop)(?=.*BattleLoading -> event Some\("Tactical_Battle_Loading_Loop_Event"\))(?=.*Space_Map_\w+_Ambient_Music_Event)(?=.*Space_Map_\w+_Battle_Music_Event)(?=.*Result \{[^}]*win: true[^}]*\} -> event Some\("\w+_Win_Tactical_Event"\))'),
    # Galaxy UI sounds: tab / filter dials, button clicks, build started + complete of the ordered object, fleet select + move lines.
    @('windowed-ui-pointer-audio', @('--hidden', '--no-vsync', '--exit-after', '8000', '--volume', '0', '--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth'), '(?s)(?=.*ui-test: [0-9]+ checks PASS)(?=.*audio sfx GUI_Dial_Switch_SFX)(?=.*audio sfx GUI_Button_Press_2_SFX)(?=.*audio sfx Unit_Select_\w+)(?=.*audio sfx Unit_(Retreat|Move|Fleet_Move)_\w+)'),
    @('windowed-galaxy-audio-build', @('--hidden', '--no-vsync', '--exit-after', '15000', '--volume', '0', '--campaign', $camp, '--select', $yavin, '--build', 'a', '--weeks', '12'), '(?s)(?=.*audio sfx [RUE]HD_Build_\w+)(?=.*audio sfx [RUE]HD_\w*(UPGRADED|Complete)\w*)'),
    # Esc in the real app path (no SP_INPUT_TEST): the battle menu opens above the HUD and pauses the battle clock.
    @('windowed-battle-esc-menu', @('--hidden', '--no-vsync', '--exit-after', '2500', '--volume', '0', '--campaign', $camp, '--select', $yavin, '--weeks', '8', '--move', 'Hoth', '--battle-choice', 'fight', '--battle-speed', '10', '--battle-esc-after', '5'), 'battle menu: open, clock paused=true'),
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
    $windowed = $name -like 'windowed-*'
    $base = if ($windowed) { @() } else { @('--offscreen', $png) }
    $args = ($base + $s[1]) | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }
    if ($name -like '*audio*') { $env:SP_AUDIO_LOG = '1' } else { Remove-Item Env:SP_AUDIO_LOG -ErrorAction SilentlyContinue }
    if ($name -like '*input*') { $env:SP_INPUT_TEST = 'exit' } else { Remove-Item Env:SP_INPUT_TEST -ErrorAction SilentlyContinue }
    if ($name -like '*ui-pointer*') { $env:SP_UI_TEST = 'exit'; $env:SP_SAVES_DIR = Join-Path $Out 'ui-test-saves' } else { Remove-Item Env:SP_UI_TEST, Env:SP_SAVES_DIR -ErrorAction SilentlyContinue }
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
    if ($text -match 'failed validation') { $problems += 'system parameter validation failed' }
    # Battle scenarios save their mid-battle shot as <name>_battle.png; every other scenario must write its picture.
    $shot = if ($name -like 'battle-fight*' -or $name -like 'battle-reinforce*') { Join-Path $Out "${name}_battle.png" } else { $png }
    if (-not $windowed -and -not (Test-Path $shot)) { $problems += 'no picture' }
    # A texture that exists but cannot be decoded is our bug (absent files are data gaps, logged at INFO and replaced by a neutral fallback).
    if ($text -match 'texture [^\r\n]*: undecodable') { $problems += 'undecodable texture (see log)' }
    if (-not $windowed -and (Test-Path $shot)) {
        Add-Type -AssemblyName System.Drawing
        $bmp = [System.Drawing.Bitmap]::FromFile($shot)
        $hits = 0; $n = 0
        for ($y = 0; $y -lt $bmp.Height; $y += [math]::Max(1, [int]($bmp.Height / 80))) {
            for ($x = 0; $x -lt $bmp.Width; $x += [math]::Max(1, [int]($bmp.Width / 140))) {
                $c = $bmp.GetPixel($x, $y); $n++
                if ($c.R -gt 235 -and $c.G -lt 30 -and $c.B -gt 235) { $hits++ }
            }
        }
        $bmp.Dispose()
        if ($hits -gt $n * 0.01) { $problems += "magenta picture ($hits of $n samples)" }
    }
    # A self-test that ends silently (no verdict) has to say why: the frame cap cut it off, or a click landed on Quit.
    if ($windowed -and $name -like '*input*' -or $name -like '*ui-pointer*') {
        if ($text -match 'exit-after: \d+ frames reached') { $problems += 'test cut off by the --exit-after frame cap' }
        if ($text -match 'menu: Quit clicked') { $problems += 'a click hit the Quit button' }
    }
    # Lookahead patterns over a multi-MB log are quadratic when they fail: a match timeout turns a hang into a FAIL.
    if ($s[2]) {
        try {
            $rx = [regex]::new($s[2], [Text.RegularExpressions.RegexOptions]::IgnoreCase, [TimeSpan]::FromSeconds(20))
            if (-not $rx.IsMatch($text)) { $problems += "missing '$($s[2])'" }
        } catch [System.Text.RegularExpressions.RegexMatchTimeoutException] { $problems += "check regex timed out (log $($text.Length) chars): '$($s[2])'" }
    }
    $rows += [pscustomobject]@{ Scenario = $name; Result = $(if ($problems) { 'FAIL' } else { 'ok' }); Seconds = [math]::Round($sw.Elapsed.TotalSeconds); Notes = ($problems -join ', ') }
}
$rows | Format-Table -AutoSize
$failed = @($rows | Where-Object Result -eq 'FAIL').Count
Write-Host ("{0} scenarios, {1} failed. Pictures and logs: {2}" -f $rows.Count, $failed, $Out)
exit $(if ($failed) { 1 } else { 0 })
