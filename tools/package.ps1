# Builds the player installer, EAW-Remake-Setup-<version>.exe (Inno Setup, per user, no admin rights):
#   1. cargo build --release -p space-peace --features dist, with the C runtime linked in, so a fresh Windows needs no Visual C++ runtime
#   2. checks the exe: a windowed app that imports only DLLs of Windows 10/11
#   3. compiles tools\installer\eaw-remake.iss with ISCC
#   .\tools\package.ps1                              build into target\dist, write dist\EAW-Remake-Setup-<version>.exe
#   .\tools\package.ps1 -SkipBuild                   reuse the exe of the last build (checks and setup only)
#   .\tools\package.ps1 -TargetDir F:\t -OutDir C:\o  other cargo target folder / output folder
# Needs Rust and the MSVC build tools (build.ps1 installs them) and Inno Setup 6:
#   winget install --id JRSoftware.InnoSetup -e --silent --scope user
# The target folder is its own one: the C runtime flag would otherwise force a rebuild of everything in target\release.
param(
    [string]$TargetDir = (Join-Path $PSScriptRoot '..\target\dist'),
    [string]$OutDir = (Join-Path $PSScriptRoot '..\dist'),
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$TargetDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($TargetDir)
$OutDir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutDir)

# The version of the setup is the workspace version.
$manifest = Get-Content -Raw (Join-Path $repo 'Cargo.toml')
if ($manifest -notmatch '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') { throw 'No version under [workspace.package] in Cargo.toml' }
$version = $Matches[1]
$exe = Join-Path $TargetDir 'release\eaw-remake.exe'

# 1. Build
if (-not $SkipBuild) {
    $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue) -and (Test-Path "$cargoBin\cargo.exe")) { $env:PATH = "$cargoBin;$env:PATH" }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'cargo not found: run .\build.ps1 once, it installs Rust and the MSVC build tools' }
    $oldFlags = $env:RUSTFLAGS
    $oldTarget = $env:CARGO_TARGET_DIR
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    $env:CARGO_TARGET_DIR = $TargetDir
    Push-Location $repo
    try {
        # cargo reports progress on stderr; with 'Stop' Windows PowerShell 5.1 turns that into a terminating error when the output is
        # captured, so only the exit code decides here (as in build.ps1).
        $ErrorActionPreference = 'Continue'
        cargo build --release -p space-peace --features dist
        $buildCode = $LASTEXITCODE
        $ErrorActionPreference = 'Stop'
    } finally {
        Pop-Location
        $env:RUSTFLAGS = $oldFlags
        $env:CARGO_TARGET_DIR = $oldTarget
    }
    if ($buildCode -ne 0) { throw "cargo build failed (exit code $buildCode)" }
}
if (-not (Test-Path $exe)) { throw "exe not found: $exe" }

# 2. Check the exe
function Find-Dumpbin {
    $onPath = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path $vswhere) {
        $hits = & $vswhere -products '*' -find 'VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe'
        if ($hits) { return @($hits)[-1] }
    }
    throw 'dumpbin.exe not found: install the MSVC build tools (build.ps1 does)'
}
$dumpbin = Find-Dumpbin

# Windowed app: the installed game opens no console window.
$headers = & $dumpbin /nologo /headers $exe
if (-not ($headers -match 'subsystem \(Windows GUI\)')) { throw 'the exe is not a windowed app: was it built with --features dist?' }

# Only DLLs that come with Windows 10/11. The C runtime (vcruntime, msvcp, ucrt) must be inside the exe.
$deps = & $dumpbin /nologo /dependents $exe | ForEach-Object { if ($_ -match '^\s+(\S+\.dll)\s*$') { $Matches[1].ToLowerInvariant() } } | Sort-Object -Unique
if (-not $deps) { throw 'dumpbin /dependents listed no DLLs' }
$runtime = '^(vcruntime|msvcp|msvcr|vcomp|concrt|vccorlib|ucrtbase|api-ms-win-crt-)'
$system32 = Join-Path $env:SystemRoot 'System32'
$bad = @()
foreach ($dll in $deps) {
    if ($dll -match $runtime) {
        $bad += "$dll (C runtime: it must be linked in, is RUSTFLAGS=-C target-feature=+crt-static set?)"
    } elseif ($dll -like 'api-ms-win-*') {
        continue   # an API set: Windows maps it to its own DLLs
    } elseif (-not (Test-Path (Join-Path $system32 $dll))) {
        $bad += "$dll (not in System32: not a DLL of Windows 10/11)"
    } elseif ((Get-Item (Join-Path $system32 $dll)).VersionInfo.CompanyName -notlike 'Microsoft*') {
        $bad += "$dll (in System32, but not from Microsoft)"
    }
}
Write-Host ("exe imports {0} DLLs: {1}" -f $deps.Count, ($deps -join ', '))
if ($bad) { $bad | ForEach-Object { Write-Host "  not allowed: $_" -ForegroundColor Red }; throw 'the exe imports DLLs that a fresh Windows may not have' }

# 3. Setup
$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { $found = Get-Command ISCC.exe -ErrorAction SilentlyContinue; if ($found) { $iscc = $found.Source } }
if (-not $iscc) { throw 'Inno Setup 6 not found: winget install --id JRSoftware.InnoSetup -e --silent --scope user' }
New-Item -ItemType Directory -Force $OutDir | Out-Null
& $iscc "/DAppVersion=$version" "/DExePath=$exe" "/O$OutDir" (Join-Path $repo 'tools\installer\eaw-remake.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC failed (exit code $LASTEXITCODE)" }

$setup = Join-Path $OutDir "EAW-Remake-Setup-$version.exe"
if (-not (Test-Path $setup)) { throw "setup not written: $setup" }
Write-Host ("Done. {0}  {1:N1} MB (game exe {2:N1} MB)" -f $setup, ((Get-Item $setup).Length / 1MB), ((Get-Item $exe).Length / 1MB)) -ForegroundColor Green
Write-Host ("SHA-256 {0}" -f (Get-FileHash $setup -Algorithm SHA256).Hash)
