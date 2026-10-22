# Starts the debug build with an isolated configuration and sizes the window.
# Usage: launch.ps1 -Config <dir> [-Args ...]
param([Parameter(Mandatory)][string]$Config, [string[]]$AppArgs = @())
. "$PSScriptRoot\win.ps1"
$root = Resolve-Path "$PSScriptRoot\..\.."
Get-Process nebula -ErrorAction SilentlyContinue | Where-Object { $_.Path -like '*target\debug*' } | Stop-Process -Force
Remove-Item -Recurse -Force "$Config\backup" -ErrorAction SilentlyContinue
$env:NEBULA_CONFIG_DIR = $Config
if ($AppArgs.Count) { Start-Process "$root\target\debug\nebula.exe" -ArgumentList $AppArgs } else { Start-Process "$root\target\debug\nebula.exe" }
$h = [IntPtr]::Zero
for ($i = 0; $i -lt 60 -and $h -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 500; $h = Get-NebulaWindow }
if ($h -eq [IntPtr]::Zero) { throw "Kein Nebula-Fenster gefunden." }
Start-Sleep -Seconds 2
[W]::keybd_event(0x12, 0, 0, [IntPtr]::Zero); [W]::keybd_event(0x12, 0, 2, [IntPtr]::Zero)  # Alt, so SetForegroundWindow works
[void][W]::SetForegroundWindow($h)
[void][W]::MoveWindow($h, 40, 40, 1800, 1100, $true)
"ok $h"
