# Launch nodi.exe, measure time-to-window + RAM, capture a screenshot of the window.
# Usage: pwsh scripts\smoke.ps1 [-Exe path] [-Shot path] [-KeepOpen]
param(
    [string]$Exe = "$PSScriptRoot\..\target\release\nodi.exe",
    [string]$Shot = "$PSScriptRoot\..\.agent-logs\smoke.png",
    [switch]$KeepOpen
)
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System; using System.Runtime.InteropServices;
public static class W {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
[W]::SetProcessDPIAware() | Out-Null
$sw = [Diagnostics.Stopwatch]::StartNew()
$p = Start-Process $Exe -PassThru
while ($sw.ElapsedMilliseconds -lt 10000) {
    $p.Refresh()
    if ($p.MainWindowHandle -ne 0) { break }
    Start-Sleep -Milliseconds 10
}
$t = $sw.ElapsedMilliseconds
$p.WaitForInputIdle(5000) | Out-Null
$tIdle = $sw.ElapsedMilliseconds
Start-Sleep -Milliseconds 800
$p.Refresh()
"title=$($p.MainWindowTitle)"
"responding=$($p.Responding)"
"time_to_window_ms=$t time_to_input_idle_ms=$tIdle"
"working_set_MB=$([math]::Round($p.WorkingSet64/1MB,1)) private_MB=$([math]::Round($p.PrivateMemorySize64/1MB,1))"
"exe_KB=$([math]::Round((Get-Item $Exe).Length/1KB))"
[W]::SetForegroundWindow($p.MainWindowHandle) | Out-Null
Start-Sleep -Milliseconds 300
$r = New-Object W+RECT
[W]::GetWindowRect($p.MainWindowHandle, [ref]$r) | Out-Null
$w = $r.R - $r.L; $h = $r.B - $r.T
if ($w -gt 0 -and $h -gt 0) {
    $bmp = New-Object Drawing.Bitmap $w, $h
    $g = [Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($r.L, $r.T, 0, 0, $bmp.Size)
    $bmp.Save($Shot, [Drawing.Imaging.ImageFormat]::Png)
    "screenshot=$Shot ($w x $h)"
}
if (-not $KeepOpen) { $p.CloseMainWindow() | Out-Null; if (-not $p.WaitForExit(5000)) { "WARN: did not exit in 5s"; $p.Kill() } else { "exit_code=$($p.ExitCode)" } }
