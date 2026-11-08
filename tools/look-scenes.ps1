# Look scenes: frame sequences of the battle view for comparing a look change against the build before it.
#   .\tools\look-scenes.ps1 -Exe <battle_demo.exe> -Out <folder>                 every scene
#   .\tools\look-scenes.ps1 -Exe <battle_demo.exe> -Out <folder> -Scene Death    one scene (or a list: -Scene Missiles,Death)
# The exe is the battle_demo example (cargo build --release -p battle-view --example battle_demo). Every scene renders offscreen (no
# window, no audio) into PNG frames <Out>\<scene>_t<ms>.png and writes its console output to <Out>\<scene>.log. The battle is
# deterministic, so two builds give the same moments; frames that differ between them show what the change did. Builds whose battle
# simulation differs drift apart after a few seconds of battle: compare the render code on top of one simulation.
# Each scene is also a function (Invoke-LookMissiles, ...) with the same -Exe and -Out, for use after dot-sourcing this file.
# Needs the game and the Remake mod installed. Saves and settings of the runs stay under <Out>, nothing goes into the repo.
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\target\release\examples\battle_demo.exe'),
    [string]$Out = (Join-Path $env:TEMP 'eaw-remake-look'),
    [string[]]$Scene = @('Missiles', 'Death', 'OrbitExecutor', 'OrbitStarDestroyer', 'Reticles', 'Squadrons', 'SquadronsSelected', 'Hunt', 'SelectedExecutor'),
    [string]$Size = '1280x720',
    [int]$TimeoutSeconds = 600
)
$ErrorActionPreference = 'Stop'

# Comma-separated capture times from $From to $To in steps of $Step seconds.
function Get-LookTimes([double]$From, [double]$To, [double]$Step) {
    $t = @()
    for ($x = $From; $x -le $To + 1e-6; $x += $Step) { $t += $x.ToString('0.###', [Globalization.CultureInfo]::InvariantCulture) }
    $t -join ','
}

# Runs one scene: the SP_DEMO_* switches in $Env, the demo arguments in $Arguments.
function Invoke-LookScene([string]$Exe, [string]$Out, [string]$Name, [hashtable]$Env, [string[]]$Arguments, [int]$TimeoutSeconds = 600) {
    if (-not (Test-Path $Exe)) { throw "battle_demo not found: $Exe" }
    New-Item -ItemType Directory -Force $Out | Out-Null
    $saved = @{}
    $vars = @{ APPDATA = (Join-Path $Out 'appdata'); SP_SAVES_DIR = (Join-Path $Out 'saves') } + $Env
    New-Item -ItemType Directory -Force $vars.APPDATA, $vars.SP_SAVES_DIR | Out-Null
    foreach ($k in $vars.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k); [Environment]::SetEnvironmentVariable($k, [string]$vars[$k]) }
    try {
        Get-ChildItem $Out -Filter "${Name}_t*.png" -ErrorAction SilentlyContinue | Remove-Item
        $log = Join-Path $Out "$Name.log"
        $all = @('--offscreen', ('"{0}"' -f (Join-Path $Out "$Name.png")), '--size', $Size) + $Arguments
        $p = Start-Process -FilePath $Exe -ArgumentList $all -NoNewWindow -PassThru -RedirectStandardOutput $log -RedirectStandardError "$log.err"
        if (-not $p.WaitForExit($TimeoutSeconds * 1000)) { Stop-Process -Id $p.Id -Force; Write-Host "$Name : timed out after $TimeoutSeconds s" -ForegroundColor Red; return }
        $n = @(Get-ChildItem $Out -Filter "${Name}_t*.png").Count
        Write-Host ("{0,-20} {1,3} frames  {2}" -f $Name, $n, $Out)
    } finally {
        foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k]) }
    }
}

# An Executor's missile salvo in flight (41 to 43 s of battle, every 0.1 s), the camera on the first missile, low against open space.
function Invoke-LookMissiles([string]$Exe, [string]$Out) {
    Invoke-LookScene $Exe $Out 'missiles' @{ SP_DEMO_EXTRA = 'Executor_Star_Dreadnought:1'; SP_DEMO_PROJ = 'missile'; SP_DEMO_DIST = '930'; SP_DEMO_PITCH = '-15' } `
        @('--shadows', '--times', (Get-LookTimes 41 43 0.1)) $TimeoutSeconds
}

# The first capital ship that dies (the default demo: an MC-80 at about 162 s) and its explosions, from its death to 8 s after it, every 0.25 s.
function Invoke-LookDeath([string]$Exe, [string]$Out) {
    Invoke-LookScene $Exe $Out 'death' @{ SP_DEMO_DEATH = '150' } @('--shadows', '--cam', '30,35,900', '--times', (Get-LookTimes 0 8 0.25)) $TimeoutSeconds
}

# The camera turning round an Executor at 45 degrees per second (8 to 9 s of battle, every 0.05 s): hull, turrets and engine glows.
function Invoke-LookOrbitExecutor([string]$Exe, [string]$Out) {
    Invoke-LookScene $Exe $Out 'orbit_executor' @{ SP_DEMO_EXTRA = 'Executor_Star_Dreadnought:1'; SP_DEMO_FOCUS = 'executor'; SP_DEMO_ORBIT = '45' } `
        @('--shadows', '--cam', '0,40,4000', '--times', (Get-LookTimes 8 9 0.05)) $TimeoutSeconds
}

# The camera turning round an Imperial Star Destroyer at 60 degrees per second, close (20 to 21 s, every 0.05 s).
function Invoke-LookOrbitStarDestroyer([string]$Exe, [string]$Out) {
    Invoke-LookScene $Exe $Out 'orbit_star_destroyer' @{ SP_DEMO_FOCUS = 'star_destroyer'; SP_DEMO_ORBIT = '60' } `
        @('--shadows', '--cam', '0,35,1400', '--times', (Get-LookTimes 20 21 0.05)) $TimeoutSeconds
}

# The EaW interface on a hovered enemy capital ship while the camera turns round it at 60 degrees per second: brackets, bars and
# hardpoint reticles (20 to 20.5 s, every frame). The log has, per frame, how far the reticles sit from their hardpoints (marker lines).
function Invoke-LookReticles([string]$Exe, [string]$Out) {
    Invoke-LookScene $Exe $Out 'reticles' @{ SP_DEMO_NOFOG = '1'; SP_DEMO_FOCUS_TARGET = '2500'; SP_DEMO_ORBIT = '60'; SP_DEMO_MARKER_LOG = '1' } `
        @('--ui', '--overlay', '--shadows', '--cam', '0,45,2500', '--times', (Get-LookTimes 20 20.5 0.016)) $TimeoutSeconds
}

# The EaW interface over the TIE and X-wing squadrons of the default demo next to a Star Destroyer, nothing selected or hovered: at play zoom
# (camera 4000, 20 to 22 s, every 0.5 s), far out (8000, 23 s) and close (1500, 24 s).
function Invoke-LookSquadrons([string]$Exe, [string]$Out) {
    Invoke-LookScene $Exe $Out 'squadrons' @{ SP_DEMO_FOCUS = 'star_destroyer' } `
        @('--ui', '--shadows', '--cam', '0,45,4000', '--cam', '0,45,4000', '--cam', '0,45,4000', '--cam', '0,45,4000', '--cam', '0,45,4000', '--cam', '0,45,8000', '--cam', '0,45,1500', '--times', '20,20.5,21,21.5,22,23,24') $TimeoutSeconds
}

# The same with every own TIE squadron selected (20 to 22 s, every 0.5 s, camera 4000).
function Invoke-LookSquadronsSelected([string]$Exe, [string]$Out) {
    Invoke-LookScene $Exe $Out 'squadrons_selected' @{ SP_DEMO_FOCUS = 'star_destroyer'; SP_DEMO_SELECT = 'tie_fighter' } `
        @('--ui', '--shadows', '--cam', '0,45,4000', '--times', (Get-LookTimes 20 22 0.5)) $TimeoutSeconds
}

# The player's TIE squadrons switch HUNT on at 3 s with the Rebels still in the fog; the camera follows the first TIE fighter (3 to 63 s, every 4 s,
# camera 6000): the squadrons search, then fly to the enemies once they are revealed.
function Invoke-LookHunt([string]$Exe, [string]$Out) {
    Invoke-LookScene $Exe $Out 'hunt' @{ SP_DEMO_ABILITY = 'HUNT@3'; SP_DEMO_FOCUS = 'tie_fighter' } `
        @('--ui', '--cam', '0,50,6000', '--times', (Get-LookTimes 3 63 4)) $TimeoutSeconds
}

# An own Executor, selected alone and not hovered (8 to 9 s, every 0.25 s, camera 6000).
function Invoke-LookSelectedExecutor([string]$Exe, [string]$Out) {
    Invoke-LookScene $Exe $Out 'selected_executor' @{ SP_DEMO_EXTRA = 'Executor_Star_Dreadnought:1'; SP_DEMO_SELECT = 'executor'; SP_DEMO_FOCUS = 'executor' } `
        @('--ui', '--shadows', '--cam', '30,40,6000', '--times', (Get-LookTimes 8 9 0.25)) $TimeoutSeconds
}

if ($MyInvocation.InvocationName -ne '.') {
    $Exe = (Resolve-Path $Exe).Path
    foreach ($s in $Scene) {
        switch ($s) {
            'Missiles' { Invoke-LookMissiles $Exe $Out }
            'Death' { Invoke-LookDeath $Exe $Out }
            'OrbitExecutor' { Invoke-LookOrbitExecutor $Exe $Out }
            'OrbitStarDestroyer' { Invoke-LookOrbitStarDestroyer $Exe $Out }
            'Reticles' { Invoke-LookReticles $Exe $Out }
            'Squadrons' { Invoke-LookSquadrons $Exe $Out }
            'SquadronsSelected' { Invoke-LookSquadronsSelected $Exe $Out }
            'Hunt' { Invoke-LookHunt $Exe $Out }
            'SelectedExecutor' { Invoke-LookSelectedExecutor $Exe $Out }
            default { Write-Host "unknown scene $s (Missiles, Death, OrbitExecutor, OrbitStarDestroyer, Reticles, Squadrons, SquadronsSelected, Hunt, SelectedExecutor)" -ForegroundColor Red }
        }
    }
}
