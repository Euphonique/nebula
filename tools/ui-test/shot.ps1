# Screenshot of the Nebula window. Usage: shot.ps1 <file.png>
param([Parameter(Mandatory)][string]$Out)
. "$PSScriptRoot\win.ps1"
Add-Type -AssemblyName System.Drawing
$h = Get-NebulaWindow
if ($h -eq [IntPtr]::Zero) { throw "Kein Nebula-Fenster." }
$r = Get-WindowRect $h
$bmp = New-Object System.Drawing.Bitmap ($r.Right - $r.Left), ($r.Bottom - $r.Top)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.Left, $r.Top, 0, 0, $bmp.Size)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
"$($bmp.Width)x$($bmp.Height)"
