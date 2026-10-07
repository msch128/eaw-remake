# Space Peace: Not at War - Build-Skript
# Richtet bei Bedarf Rust und die MSVC Build Tools ein und baut das Spiel lokal.
#   .\build.ps1            -> Release-Build
#   .\build.ps1 -Debug     -> Debug-Build
param([switch]$Debug)

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

function Have($cmd) { [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }

# 1. Rust
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (Have cargo) -and (Test-Path "$cargoBin\cargo.exe")) { $env:PATH = "$cargoBin;$env:PATH" }
if (-not (Have cargo)) {
    Write-Host 'Rust fehlt - installiere rustup ...' -ForegroundColor Yellow
    winget install --id Rustlang.Rustup -e --silent --accept-package-agreements --accept-source-agreements
    $env:PATH = "$cargoBin;$env:PATH"
    rustup default stable
}

# 2. MSVC Linker (Visual Studio Build Tools mit C++-Workload)
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$hasMsvc = (Test-Path $vswhere) -and (& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
if (-not $hasMsvc) {
    Write-Host 'MSVC Build Tools fehlen - installiere (Admin-Bestaetigung noetig) ...' -ForegroundColor Yellow
    winget install --id Microsoft.VisualStudio.BuildTools -e --silent --accept-package-agreements --accept-source-agreements `
        --override '--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
}

# 3. Build
$cargoArgs = @('build')
if (-not $Debug) { $cargoArgs += '--release' }
cargo @cargoArgs
if ($LASTEXITCODE -ne 0) { Write-Host 'Build fehlgeschlagen.' -ForegroundColor Red; exit $LASTEXITCODE }

$profileDir = if ($Debug) { 'debug' } else { 'release' }
Write-Host "Fertig. Ausgabe: $PSScriptRoot\target\$profileDir" -ForegroundColor Green
