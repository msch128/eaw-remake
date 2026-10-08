# Real-input playtest helpers for Space Peace (Windows PowerShell 5.1).
# Dot-source it:  . .\tools\playtest\driver.ps1
# Usage:
#   Start-Game -Exe 'target\playfix\release\space-peace.exe'   # windowed, --volume 0 (muted; the flag overrides the stored setting), scratch APPDATA
#   Save-Shot 'menu'                                           # PNG of OUR window only; returns the path to read
#   Click-At 0.5 0.4                  # left click at 50% x, 40% y of the window; -Button R for right click
#   Drag-Box 0.2 0.3 0.6 0.7          # box select; Press-Key 'S'; Press-Key $VK.Esc; Press-Key '0' -Mods $VK.Ctrl
#   Show-LogTail; Get-LogProblems; Stop-Game
# Nothing here runs a game by itself. Start-Game launches a WINDOWED app and the mouse/key functions move the real OS
# cursor, so only use it when the machine is free. Coordinates are fractions (0..1) of the game window's client area.
# Captures cover only the game window (never the full desktop) and go to a scratch folder, not the repo.

Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System; using System.Runtime.InteropServices; using System.Drawing;
public static class Win {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, int data, UIntPtr extra);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
  public const uint LD = 2, LU = 4, RD = 8, RU = 16, MD = 32, MU = 64, WHEEL = 0x800, KEYUP = 2;
  public static Bitmap Capture(IntPtr h) {
    RECT r; GetClientRect(h, out r); POINT p = new POINT(); ClientToScreen(h, ref p);
    int w = r.R - r.L, ht = r.B - r.T; Bitmap bmp = new Bitmap(w, ht);
    using (Graphics g = Graphics.FromImage(bmp)) g.CopyFromScreen(p.X, p.Y, 0, 0, new Size(w, ht));
    return bmp;
  }
}
'@
[void][Win]::SetProcessDPIAware()
Add-Type -AssemblyName System.Drawing

$script:Proc = $null
$script:Scratch = Join-Path $env:TEMP 'space-peace-qa'
$script:Shot = 0

function Start-Game {
    param([string]$Exe = 'target\play\release\space-peace.exe', [string[]]$ExtraArgs = @())
    New-Item -ItemType Directory -Force "$script:Scratch\appdata", "$script:Scratch\shots" | Out-Null
    $env:APPDATA = "$script:Scratch\appdata"   # scratch saves/settings/logs; the user's real ones stay untouched
    $script:Proc = Start-Process -FilePath $Exe -ArgumentList (@('--volume', '0') + $ExtraArgs) -PassThru
    for ($i = 0; $i -lt 120 -and $script:Proc.MainWindowHandle -eq 0; $i++) { Start-Sleep -Milliseconds 500; $script:Proc.Refresh() }
    if ($script:Proc.MainWindowHandle -eq 0) { throw 'no window appeared' }
    [void][Win]::SetForegroundWindow($script:Proc.MainWindowHandle)
}
function Stop-Game {
    if ($script:Proc -and -not $script:Proc.HasExited) {
        [void]$script:Proc.CloseMainWindow()
        if (-not $script:Proc.WaitForExit(5000)) { $script:Proc.Kill() }
    }
}
function Test-GameAlive { [bool]($script:Proc -and -not $script:Proc.HasExited) }
function Get-ClientSize { $r = New-Object Win+RECT; [void][Win]::GetClientRect($script:Proc.MainWindowHandle, [ref]$r); @($r.R, $r.B) }

function Move-Mouse([int]$x, [int]$y) {
    $p = New-Object Win+POINT; $p.X = $x; $p.Y = $y
    [void][Win]::ClientToScreen($script:Proc.MainWindowHandle, [ref]$p)
    [void][Win]::SetCursorPos($p.X, $p.Y); Start-Sleep -Milliseconds 60
}
function Get-Px([double]$fx, [double]$fy) { $s = Get-ClientSize; @([int]($fx * $s[0]), [int]($fy * $s[1])) }
function Get-ButtonFlags($Button) {
    switch ($Button) { 'R' { @([Win]::RD, [Win]::RU) } 'M' { @([Win]::MD, [Win]::MU) } default { @([Win]::LD, [Win]::LU) } }
}
# Click-At 0.5 0.9 [-Button L|R|M] [-Double]
function Click-At([double]$fx, [double]$fy, $Button = 'L', [switch]$Double) {
    $p = Get-Px $fx $fy; Move-Mouse $p[0] $p[1]; $f = Get-ButtonFlags $Button
    $times = if ($Double) { 2 } else { 1 }
    for ($n = 0; $n -lt $times; $n++) {
        [Win]::mouse_event($f[0], 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 70
        [Win]::mouse_event($f[1], 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 90
    }
}
function Drag-Box([double]$x1, [double]$y1, [double]$x2, [double]$y2, $Button = 'L') {
    $a = Get-Px $x1 $y1; $b = Get-Px $x2 $y2; Move-Mouse $a[0] $a[1]; $f = Get-ButtonFlags $Button
    [Win]::mouse_event($f[0], 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 80
    foreach ($t in 1..8) { Move-Mouse ([int]($a[0] + ($b[0] - $a[0]) * $t / 8)) ([int]($a[1] + ($b[1] - $a[1]) * $t / 8)) }
    [Win]::mouse_event($f[1], 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 100
}
function Scroll-Wheel([int]$notches) { [Win]::mouse_event([Win]::WHEEL, 0, 0, 120 * $notches, [UIntPtr]::Zero); Start-Sleep -Milliseconds 100 }

# Press-Key 'S' | Press-Key $VK.Esc | Press-Key '0' -Mods $VK.Ctrl
$VK = @{ Esc = 0x1B; Space = 0x20; Enter = 0x0D; F3 = 0x72; F10 = 0x79; F11 = 0x7A; Shift = 0x10; Ctrl = 0x11 }
function Press-Key($Key, [int[]]$Mods = @(), [int]$HoldMs = 60) {
    $vk = if ($Key -is [string]) { [byte][char]$Key.ToUpper() } else { [byte]$Key }
    foreach ($m in $Mods) { [Win]::keybd_event([byte]$m, 0, 0, [UIntPtr]::Zero) }
    [Win]::keybd_event($vk, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds $HoldMs
    [Win]::keybd_event($vk, 0, [Win]::KEYUP, [UIntPtr]::Zero)
    foreach ($m in $Mods) { [Win]::keybd_event([byte]$m, 0, [Win]::KEYUP, [UIntPtr]::Zero) }
    Start-Sleep -Milliseconds 80
}

# Window-only screenshot (client area) into the scratch folder; returns the path to read.
function Save-Shot([string]$Name = 'shot') {
    $script:Shot++; $path = '{0}\shots\{1:D3}-{2}.png' -f $script:Scratch, $script:Shot, $Name
    $bmp = [Win]::Capture($script:Proc.MainWindowHandle)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose(); $path
}
function Get-LatestLog { Get-ChildItem "$script:Scratch\appdata\SpacePeace\logs" -Filter *.log -ErrorAction SilentlyContinue | Sort-Object LastWriteTime | Select-Object -Last 1 }
function Show-LogTail([int]$n = 40) { $l = Get-LatestLog; if ($l) { Get-Content $l.FullName -Tail $n } }
function Get-LogProblems { $l = Get-LatestLog; if ($l) { Select-String -Path $l.FullName -Pattern 'ERROR|WARN|panick' | Select-Object -Last 30 | ForEach-Object Line } }
