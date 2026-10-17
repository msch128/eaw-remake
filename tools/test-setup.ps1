# Silent test of the setup made by tools\package.ps1. Nothing opens a window and nothing needs admin rights:
#   a. installs it with /VERYSILENT into a scratch folder, also over an existing install (update)
#   b. checks the files, the Start menu entry and the uninstall entry
#   c. runs the installed game offscreen (--campaign ... --frames 30 --offscreen) and checks the PNG
#   d. runs the installed game with --check-install (exit code 0) and --check-install --game-dir <empty folder> (exit code 2)
#   e. uninstalls silently and checks what is left
#   f. installs with the game, then with the mod made invisible to the setup (EAW_GAME_DIR / EAW_MOD_DIR): the install still
#      completes and the setup log has the --check-install exit code 2 and 3
#   .\tools\test-setup.ps1 [-Setup <setup.exe>] [-Scratch <folder>]
# Needs the game and the Remake mod (steps c, d). The game runs with a scratch APPDATA, SP_SAVES_DIR and --volume 0, so the
# player's settings, saves and logs are never touched. Steps a, e and f do write the real Start menu folder and the real uninstall
# key (HKCU) of the user, and remove them again. The message boxes of the wizard never show in a silent setup, so they are not tested.
param(
    [string]$Setup = (Get-ChildItem (Join-Path $PSScriptRoot '..\dist\EAW-Remake-Setup-*.exe') -ErrorAction SilentlyContinue | Sort-Object LastWriteTime | Select-Object -Last 1 -ExpandProperty FullName),
    [string]$Scratch = (Join-Path $env:TEMP 'eaw-setup-test')
)

$ErrorActionPreference = 'Stop'
if (-not $Setup -or -not (Test-Path $Setup)) { throw 'setup not found: build it with .\tools\package.ps1 or pass -Setup' }
$Setup = (Resolve-Path $Setup).Path
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$script = Get-Content -Raw (Join-Path $repo 'tools\installer\eaw-remake.iss')
$appId = [regex]::Match($script, '(?m)^AppId=\{\{([0-9A-Fa-f-]+)\}').Groups[1].Value
$uninstallRoot = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall'
$uninstallKey = "$uninstallRoot\{$appId}_is1"
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\EAW Remake'
$programs = Split-Path $startMenu

Remove-Item $Scratch -Recurse -Force -ErrorAction SilentlyContinue
$app = Join-Path $Scratch 'app'
$appdata = Join-Path $Scratch 'appdata'
$empty = Join-Path $Scratch 'empty-game-dir'
New-Item -ItemType Directory -Force $Scratch, $appdata, (Join-Path $Scratch 'saves'), $empty | Out-Null
$gameEnv = @{ APPDATA = $appdata; SP_SAVES_DIR = (Join-Path $Scratch 'saves') }

$rows = New-Object System.Collections.Generic.List[object]
function Check($name, [bool]$ok, $detail = '') {
    $rows.Add([pscustomobject]@{ Check = $name; Ok = $ok })
    Write-Host ("{0,-4} {1}  {2}" -f $(if ($ok) { 'PASS' } else { 'FAIL' }), $name, $detail)
}

# Starts a program without a window, waits for it (kills only this process on timeout), returns exit code and captured output.
function Run($file, [string[]]$argList, [int]$seconds, [string]$tag, [hashtable]$vars = @{}) {
    $quoted = $argList | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }
    $out = Join-Path $Scratch "$tag.out"
    $err = Join-Path $Scratch "$tag.err"
    $saved = @{}
    foreach ($k in $vars.Keys) { $saved[$k] = [Environment]::GetEnvironmentVariable($k); [Environment]::SetEnvironmentVariable($k, $vars[$k]) }
    try {
        $p = Start-Process -FilePath $file -ArgumentList $quoted -PassThru -WindowStyle Hidden -RedirectStandardOutput $out -RedirectStandardError $err
        $null = $p.Handle   # keeps the handle so ExitCode is readable after WaitForExit
        $timedOut = -not $p.WaitForExit($seconds * 1000)
        if ($timedOut) { try { $p.Kill() } catch {} }
    } finally {
        foreach ($k in $vars.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k]) }
    }
    [pscustomobject]@{ Code = $(if ($timedOut) { -1 } else { $p.ExitCode }); TimedOut = $timedOut; Out = (Get-Content -Raw $out -ErrorAction SilentlyContinue); Err = (Get-Content -Raw $err -ErrorAction SilentlyContinue) }
}

function Install($tag, [hashtable]$vars = @{}) {
    $log = Join-Path $Scratch "$tag.log"
    $r = Run $Setup @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', "/DIR=$app", "/LOG=$log") 300 $tag $vars
    $r | Add-Member -NotePropertyName Log -NotePropertyValue (Get-Content -Raw $log -ErrorAction SilentlyContinue) -PassThru
}

function Uninstall($tag) {
    $r = Run (Join-Path $app 'unins000.exe') @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART') 300 $tag
    for ($i = 0; $i -lt 40 -and (Test-Path $app); $i++) { Start-Sleep -Milliseconds 500 }   # the uninstaller finishes in a copy of itself
    $r
}

function CheckExitCodeInLog($r) { if ($r.Log -match '--check-install exit code (\d+)') { [int]$Matches[1] } else { -1 } }

$programsBefore = (Get-ChildItem $programs -Force | ForEach-Object Name) -join '|'
$uninstallBefore = (Get-ChildItem $uninstallRoot | ForEach-Object PSChildName) -join '|'
$exe = Join-Path $app 'eaw-remake.exe'

# a. install, then install again over it
$r = Install 'setup'
Check 'a. setup /VERYSILENT exits with 0' ($r.Code -eq 0) "exit code $($r.Code)"
$again = Install 'setup-again'
Check 'a. the setup run again over the install (update) exits with 0' ($again.Code -eq 0 -and (Test-Path $exe)) "exit code $($again.Code)"
Check 'a. still one uninstall entry' (@(Get-ChildItem $uninstallRoot | Where-Object PSChildName -like "{$appId}*").Count -eq 1)

# b. files, Start menu, uninstall entry
$expected = 'eaw-remake.exe', 'LICENSE', 'CREDITS.md', 'README.txt', 'unins000.exe'
foreach ($f in $expected) {
    $path = Join-Path $app $f
    Check "b. installed file $f" (Test-Path $path) $(if (Test-Path $path) { '{0:N0} bytes' -f (Get-Item $path).Length })
}
$extra = @(Get-ChildItem $app -Force | ForEach-Object Name | Where-Object { $_ -notin $expected -and $_ -ne 'unins000.dat' })
Check 'b. nothing else in the install folder (no game files)' ($extra.Count -eq 0) ($extra -join ', ')
$lnk = Join-Path $startMenu 'EAW Remake.lnk'
$target = if (Test-Path $lnk) { (New-Object -ComObject WScript.Shell).CreateShortcut($lnk).TargetPath }
Check 'b. Start menu entry points at the installed exe' ($target -eq $exe) "$lnk -> $target"
Check 'b. Start menu uninstall entry' (Test-Path (Join-Path $startMenu 'Uninstall EAW Remake.lnk'))
$key = if (Test-Path $uninstallKey) { Get-ItemProperty $uninstallKey }
Check 'b. uninstall entry in HKCU (Apps and features)' ($null -ne $key -and $key.UninstallString -like '*unins000.exe*') "$($key.DisplayName), $($key.Publisher)"
Check 'b. setup log has the --check-install exit code' ((CheckExitCodeInLog $r) -ge 0) "exit code $(CheckExitCodeInLog $r)"

# c. run the installed game offscreen
$png = Join-Path $Scratch 'offscreen.png'
$r = Run $exe @('--campaign', 'GC_Full_Web_Way_Rebel', '--frames', '30', '--offscreen', $png, '--volume', '0') 420 'offscreen' $gameEnv
Check 'c. installed exe offscreen run exits with 0' ($r.Code -eq 0 -and -not $r.TimedOut) "exit code $($r.Code)"
Check 'c. no panic and no ERROR in its log' (($r.Out + $r.Err) -notmatch 'panicked at|\bERROR\b') ('{0:N0} chars of log' -f ($r.Out + $r.Err).Length)
if (Test-Path $png) {
    Add-Type -AssemblyName System.Drawing
    $img = [System.Drawing.Bitmap]::FromFile($png)
    $lit = 0; $n = 0
    for ($y = 0; $y -lt $img.Height; $y += 60) { for ($x = 0; $x -lt $img.Width; $x += 60) { $c = $img.GetPixel($x, $y); $n++; if ($c.R + $c.G + $c.B -gt 30) { $lit++ } } }
    $w = $img.Width; $h = $img.Height
    $img.Dispose()
    Check 'c. PNG is a picture, not black' ($w -ge 1280 -and $lit -gt $n / 10) ("{0}x{1}, {2:N0} bytes, {3} of {4} sample points lit" -f $w, $h, (Get-Item $png).Length, $lit, $n)
} else {
    Check 'c. PNG written' $false $png
}

# d. --check-install
$r = Run $exe @('--check-install') 60 'check-install' $gameEnv
Check 'd. --check-install exits with 0 (game and mod found)' ($r.Code -eq 0) ("exit code $($r.Code): " + (($r.Out -split "`r?`n" | Select-Object -Skip 1 -First 3) -join ' | '))
$r = Run $exe @('--check-install', '--game-dir', $empty) 60 'check-install-empty' $gameEnv
Check 'd. --check-install --game-dir <empty folder> exits with 2' ($r.Code -eq 2) ("exit code $($r.Code): " + (($r.Out -split "`r?`n" | Select-Object -Skip 1 -First 1) -join ''))

# e. uninstall
$r = Uninstall 'uninstall'
Check 'e. uninstaller exits with 0' ($r.Code -eq 0) "exit code $($r.Code)"
Check 'e. install folder is gone' (-not (Test-Path $app)) $(if (Test-Path $app) { (Get-ChildItem $app -Force | ForEach-Object Name) -join ', ' })
Check 'e. Start menu folder is gone' (-not (Test-Path $startMenu))
Check 'e. uninstall entry is gone' (-not (Test-Path $uninstallKey))
Check 'e. no other Start menu folder or uninstall entry changed' (((Get-ChildItem $programs -Force | ForEach-Object Name) -join '|') -eq $programsBefore -and ((Get-ChildItem $uninstallRoot | ForEach-Object PSChildName) -join '|') -eq $uninstallBefore)
# The uninstaller can only remove what the setup installed, unless the script says otherwise: it must not (saves and settings stay).
$code = ($script -split "`r?`n" | Where-Object { $_ -notmatch '^\s*;' }) -join "`n"
Check 'e. the script deletes nothing besides its own files (no [InstallDelete], no [UninstallDelete], no SpacePeace folder)' ($code -notmatch '(?mi)^\[(Install|Uninstall)Delete\]' -and $code -notmatch 'SpacePeace')

# f. game or mod missing: the install still completes
foreach ($case in @(@{ Tag = 'game-missing'; Code = 2; Vars = @{ EAW_GAME_DIR = $empty } }, @{ Tag = 'mod-missing'; Code = 3; Vars = @{ EAW_MOD_DIR = (Join-Path $Scratch 'no-such-mod') } })) {
    $r = Install "setup-$($case.Tag)" $case.Vars
    Check "f. $($case.Tag): setup still exits with 0 and installs the game" ($r.Code -eq 0 -and (Test-Path $exe)) "exit code $($r.Code)"
    Check "f. $($case.Tag): setup log has the --check-install exit code $($case.Code)" ((CheckExitCodeInLog $r) -eq $case.Code) "exit code $(CheckExitCodeInLog $r)"
    $r = Uninstall "uninstall-$($case.Tag)"
    Check "f. $($case.Tag): uninstall leaves nothing" (-not (Test-Path $app) -and -not (Test-Path $startMenu) -and -not (Test-Path $uninstallKey))
}

# sizes
"{0}`nsetup {1:N1} MB" -f $Setup, ((Get-Item $Setup).Length / 1MB)
$fails = @($rows | Where-Object { -not $_.Ok }).Count
if ($fails) { Write-Host "$fails check(s) FAILED" -ForegroundColor Red; exit 1 }
Write-Host "all $($rows.Count) checks passed" -ForegroundColor Green
