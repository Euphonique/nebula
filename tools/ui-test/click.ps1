# Click at window-relative coordinates (pixels of the screenshot).
# Usage: click.ps1 <x> <y> [-Right] [-Double]; aborts unless Nebula is in the foreground.
param([int]$X, [int]$Y, [switch]$Right, [switch]$Double)
. "$PSScriptRoot\win.ps1"
$h = Get-NebulaWindow
Assert-Foreground $h
$r = Get-WindowRect $h
[void][W]::SetCursorPos($r.Left + $X, $r.Top + $Y)
Start-Sleep -Milliseconds 120
$down, $up = if ($Right) { 0x8, 0x10 } else { 0x2, 0x4 }
$n = if ($Double) { 2 } else { 1 }
for ($i = 0; $i -lt $n; $i++) {
    [W]::mouse_event($down, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 40
    [W]::mouse_event($up, 0, 0, 0, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 60
}
Start-Sleep -Milliseconds 400
