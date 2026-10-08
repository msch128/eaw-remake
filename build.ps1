# Space Peace: Not at War - build script
# Installs Rust and the MSVC Build Tools if missing, then builds the game locally.
#   .\build.ps1            -> release build
#   .\build.ps1 -Debug     -> debug build
param([switch]$Debug)

$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

function Have($cmd) { [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }

# 1. Rust
$cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
if (-not (Have cargo) -and (Test-Path "$cargoBin\cargo.exe")) { $env:PATH = "$cargoBin;$env:PATH" }
if (-not (Have cargo)) {
    Write-Host 'Rust not found - installing rustup ...' -ForegroundColor Yellow
    winget install --id Rustlang.Rustup -e --silent --accept-package-agreements --accept-source-agreements
    $env:PATH = "$cargoBin;$env:PATH"
    rustup default stable
}

# 2. MSVC linker (Visual Studio Build Tools with the C++ workload)
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$hasMsvc = (Test-Path $vswhere) -and (& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
if (-not $hasMsvc) {
    Write-Host 'MSVC Build Tools not found - installing (admin confirmation required) ...' -ForegroundColor Yellow
    winget install --id Microsoft.VisualStudio.BuildTools -e --silent --accept-package-agreements --accept-source-agreements `
        --override '--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
}

# 3. Build
$cargoArgs = @('build')
if (-not $Debug) { $cargoArgs += '--release' }
# cargo reports progress on stderr; with 'Stop' Windows PowerShell 5.1 turns that into a terminating error
# when the output is captured, so only the exit code decides here.
$ErrorActionPreference = 'Continue'
cargo @cargoArgs
if ($LASTEXITCODE -ne 0) { Write-Host 'Build failed.' -ForegroundColor Red; exit $LASTEXITCODE }

$profileDir = if ($Debug) { 'debug' } else { 'release' }
Write-Host "Done. Output: $PSScriptRoot\target\$profileDir" -ForegroundColor Green
