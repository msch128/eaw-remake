# Opening-frame sweep of every planet space map: renders each one offscreen with battle_demo and flags frames with almost no dark
# pixel (a planet disc filling the view with no limb) or maps that fail to load. One exe run at a time; output goes to $Out, never the repo.
#   .\tools\space-map-sweep.ps1 -Exe target\release\examples\battle_demo.exe [-Maps a,b] [-MinDark 0.03]
param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$Out = (Join-Path $env:TEMP 'space-map-sweep'),
    [string[]]$Maps = @(),
    [double]$MinDark = 0.03,
    [string]$Data = 'F:\SteamLibrary\steamapps\workshop\content\32470\1770851727\Data'
)
Add-Type -AssemblyName System.Drawing
New-Item -ItemType Directory -Force $Out | Out-Null
$env:APPDATA = Join-Path $Out 'appdata'
New-Item -ItemType Directory -Force $env:APPDATA | Out-Null
if (-not $Maps) {
    $Maps = Get-ChildItem (Join-Path $Data 'XML') -Filter 'Planets_*.xml' | ForEach-Object {
        Select-String -Path $_.FullName -Pattern '<Space_Tactical_Map>\s*([^<\s]+?)(\.ted)?\s*<' -AllMatches | ForEach-Object { $_.Matches } | ForEach-Object { $_.Groups[1].Value }
    } | Sort-Object { $_.ToLower() } -Unique
}
$rows = @()
foreach ($m in $Maps) {
    $png = Join-Path $Out "$m.png"
    Remove-Item $png -ErrorAction SilentlyContinue
    $log = & $Exe --offscreen $png --size 480x270 --times 2 --small --msaa 1 --volume 0 --map $m 2>&1 | Out-String
    $dark = $null; $note = ''
    if ($log -match 'ground below the plane: top at ([\d.-]+)') { $note = "top $($Matches[1])" }
    if ($log -match 'view coverage ([\d.]+), slide (\S+)') { $note += " disc cov $($Matches[1]) slide $($Matches[2])" }
    if (Test-Path $png) {
        $bmp = [System.Drawing.Bitmap]::FromFile($png)
        $n = 0; $d = 0
        for ($y = 0; $y -lt $bmp.Height; $y += 3) { for ($x = 0; $x -lt $bmp.Width; $x += 3) {
            $c = $bmp.GetPixel($x, $y); $n++
            if (($c.R * 0.3 + $c.G * 0.59 + $c.B * 0.11) / 255 -lt 0.12) { $d++ }
        } }
        $bmp.Dispose()
        $dark = [math]::Round($d / $n, 2)
    }
    $status = if ($log -match 'cannot read Art/Maps') { 'MISSING' } elseif ($m -match 'SKIES') { 'ground' } elseif ($null -eq $dark) { 'NOFRAME' } elseif ($dark -lt $MinDark) { 'FAIL' } else { 'ok' }
    $rows += [pscustomobject]@{ Map = $m; Dark = $dark; Status = $status; Note = $note }
    Write-Host ('{0,-7} {1,-34} dark {2} {3}' -f $status, $m, $dark, $note)
}
$bad = @($rows | Where-Object { $_.Status -notin 'ok', 'MISSING', 'ground' })
Write-Host "$($rows.Count) maps, $($bad.Count) flagged"
$rows | Export-Csv (Join-Path $Out 'sweep.csv') -NoTypeInformation
if ($bad.Count) { exit 1 }
