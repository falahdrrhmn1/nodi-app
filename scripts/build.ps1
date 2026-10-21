# Builds the release exe and copies it into the portable folder dist\Nodi.
# Notes live next to the exe in dist\Nodi\data (created on first start).
#
# Usage: powershell -ExecutionPolicy Bypass -File scripts\build.ps1

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path "$PSScriptRoot\..").Path

$env:CARGO_HOME = 'E:\Software\Rust\cargo'
$env:RUSTUP_HOME = 'E:\Software\Rust\rustup'
$env:PATH = "E:\Software\Rust\cargo\bin;$env:PATH"
# Keeps peak RAM low on an 8 GB laptop; raise it if you have memory to spare.
if (-not $env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS = '2' }

Push-Location $root
try {
    cargo build --release -p nodi-windows
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }
} finally {
    Pop-Location
}

$dist = Join-Path $root 'dist\Nodi'
New-Item -ItemType Directory -Force $dist | Out-Null
$running = Get-Process nodi -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq "$dist\nodi.exe" }
if ($running) { throw 'Close Nodi (dist\Nodi\nodi.exe) before rebuilding.' }
Copy-Item (Join-Path $root 'target\release\nodi.exe') $dist -Force
"Built: $dist\nodi.exe ($([math]::Round((Get-Item "$dist\nodi.exe").Length / 1KB)) KB)"
"Data:  $dist\data"
