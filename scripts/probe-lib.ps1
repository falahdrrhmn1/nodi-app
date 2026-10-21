# Helpers for ad-hoc probes and screenshots: dot-source after scripts\e2e.ps1.
#   . .\scripts\e2e.ps1 -Only none; . .\scripts\probe-lib.ps1
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class NodiShot {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
[NodiShot]::SetProcessDPIAware() | Out-Null

# Saves a PNG of the window (works even when other windows cover it).
function Save-Shot($nodi, [string]$path) {
    $rect = New-Object NodiShot+RECT
    [NodiShot]::GetWindowRect($nodi.Hwnd, [ref]$rect) | Out-Null
    $width = $rect.R - $rect.L; $height = $rect.B - $rect.T
    $bitmap = New-Object Drawing.Bitmap $width, $height
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $hdc = $graphics.GetHdc()
    [NodiShot]::PrintWindow($nodi.Hwnd, $hdc, 2) | Out-Null
    $graphics.ReleaseHdc($hdc)
    $bitmap.Save($path, [Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
    $path
}

# Writes a small test PNG (gradient + text) and returns its path.
function New-TestPng([string]$path, [int]$width = 240, [int]$height = 120) {
    $bitmap = New-Object Drawing.Bitmap $width, $height
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $brush = New-Object Drawing.Drawing2D.LinearGradientBrush (New-Object Drawing.Rectangle 0, 0, $width, $height), ([Drawing.Color]::SteelBlue), ([Drawing.Color]::Orange), 0.0
    $graphics.FillRectangle($brush, 0, 0, $width, $height)
    $graphics.DrawString('Nodi', (New-Object Drawing.Font 'Segoe UI', 20), [Drawing.Brushes]::White, 10, 10)
    $bitmap.Save($path, [Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
    $path
}
