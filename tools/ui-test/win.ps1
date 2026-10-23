# Shared helpers for the UI test scripts (dot-source this file).
Add-Type -ReferencedAssemblies System.Drawing @"
using System;
using System.Runtime.InteropServices;
public static class W {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr h, int x, int y, int w, int hh, bool repaint);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, int x, int y, int d, IntPtr e);
    [DllImport("user32.dll")] public static extern void keybd_event(byte k, byte s, uint f, IntPtr e);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
}
"@
[void][W]::SetProcessDPIAware()

# Window of the debug build (never touches the user's release instance). With
# several debug windows: the one in the foreground, else the first.
function Get-NebulaWindow {
    $debug = Get-Process nebula -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -like '*target\debug*' -and $_.MainWindowHandle -ne 0 }
    $fg = [W]::GetForegroundWindow()
    $fgPid = 0
    [void][W]::GetWindowThreadProcessId($fg, [ref]$fgPid)
    $front = $debug | Where-Object { $_.Id -eq $fgPid } | Select-Object -First 1
    if ($front) { return $fg }
    $p = $debug | Select-Object -First 1
    if ($p) { $p.MainWindowHandle } else { [IntPtr]::Zero }
}

function Assert-Foreground($h) {
    if ([W]::GetForegroundWindow() -ne $h) { throw "Nebula ist nicht im Vordergrund – Abbruch." }
}

function Get-WindowRect($h) {
    $r = New-Object W+RECT
    [void][W]::GetWindowRect($h, [ref]$r)
    $r
}
