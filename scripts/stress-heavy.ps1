# Heavy-note stress test: a big note (thousands of paragraphs, pictures,
# tables, lists, checklists, page breaks) is edited and the process is killed
# at random moments right after Ctrl+S, optionally while another process holds
# most of the free memory. After every kill the note must open again and be
# either the version before the last edit or the one after it — never empty,
# damaged or older.
#
# Usage: pwsh -File scripts\stress-heavy.ps1 [-Kills 10] [-HogMB 0] [-Exe ...]

param(
    [string]$Exe = "$PSScriptRoot\..\target\debug\nodi.exe",
    [int]$Kills = 10,
    [int]$HogMB = 0
)

$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\e2e.ps1" -Exe $Exe -Only none | Out-Null
. "$PSScriptRoot\probe-lib.ps1"

$hog = $null
if ($HogMB -gt 0) {
    # A second process that allocates and touches $HogMB MB and keeps it.
    $script = "`$blocks = New-Object System.Collections.ArrayList; for (`$i = 0; `$i -lt $HogMB; `$i++) { `$b = New-Object byte[] 1048576; for (`$j = 0; `$j -lt 1048576; `$j += 4096) { `$b[`$j] = 1 }; [void]`$blocks.Add(`$b) }; Start-Sleep -Seconds 3600"
    $hog = Start-Process pwsh -ArgumentList '-NoProfile', '-Command', $script -PassThru -WindowStyle Hidden
    Start-Sleep -Seconds 8
    $os = Get-CimInstance Win32_OperatingSystem
    "memory hog of $HogMB MB running; free now {0:N0} MB" -f ($os.FreePhysicalMemory / 1KB)
}

$failures = 0
try {
    $data = New-DataDir 'heavy'
    $png = New-TestPng (Join-Path $Root 'heavy.png') 400 200
    $nodi = Start-Nodi $data
    $watch = [Diagnostics.Stopwatch]::StartNew()
    Invoke-Nodi $nodi 'keys ## Catatan besar' | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    for ($block = 1; $block -le 15; $block++) {
        $lines = (1..200 | ForEach-Object { "blok $block baris $_ dengan beberapa kata isi catatan" }) -join '\n'
        Invoke-Nodi $nodi "type $lines\n" | Out-Null
        Invoke-Nodi $nodi "insert-image $png" | Out-Null
        Invoke-Nodi $nodi 'type \n' | Out-Null
        Invoke-Nodi $nodi 'keys 1. nomor' | Out-Null
        Invoke-Nodi $nodi 'enter' | Out-Null
        Invoke-Nodi $nodi 'tab' | Out-Null
        Invoke-Nodi $nodi "keys anak $block" | Out-Null
        Invoke-Nodi $nodi 'enter' | Out-Null
        Invoke-Nodi $nodi 'enter' | Out-Null
        Invoke-Nodi $nodi 'enter' | Out-Null
        Invoke-Nodi $nodi "keys [ ] cek $block" | Out-Null
        Invoke-Nodi $nodi 'enter' | Out-Null
        Invoke-Nodi $nodi 'enter' | Out-Null
        Invoke-Nodi $nodi 'keys ----' | Out-Null
        Invoke-Nodi $nodi 'enter' | Out-Null
        Invoke-Nodi $nodi 'table 2 3' | Out-Null
        Invoke-Nodi $nodi "type sel $block" | Out-Null
        $length = [int](((Invoke-Nodi $nodi 'sel') -split 'len=')[1])
        Invoke-Nodi $nodi "select $length $length" | Out-Null
        Invoke-Nodi $nodi 'type \n' | Out-Null
    }
    Invoke-Nodi $nodi 'cmd 101' | Out-Null
    $saved = Wait-State $nodi 'Saved' 30000
    $built = $watch.ElapsedMilliseconds
    $size = (Get-Item (Get-ChildItem (Join-Path $data 'notes') -Filter '*.rtf' | Sort-Object Length -Descending | Select-Object -First 1).FullName).Length
    $before = Invoke-Nodi $nodi 'text'
    $pictureCount = Invoke-Nodi $nodi 'pictures'
    "built the note in $built ms (Saved after $saved ms), $([math]::Round($size / 1MB, 1)) MB on disk, $($before.Length) characters, $pictureCount pictures (15 pictures + 15 page breaks)"
    Stop-Nodi $nodi

    for ($kill = 1; $kill -le $Kills; $kill++) {
        $nodi = Start-Nodi $data
        $start = Invoke-Nodi $nodi 'text'
        if ($start -cne $before) {
            $failures++
            "kill ${kill}: the note opened different from the last saved version ($($start.Length) vs $($before.Length) chars)"
        }
        $length = [int](((Invoke-Nodi $nodi 'sel') -split 'len=')[1])
        Invoke-Nodi $nodi "select $length $length" | Out-Null
        Invoke-Nodi $nodi "type \nedit sebelum kill $kill" | Out-Null
        $after = Invoke-Nodi $nodi 'text'
        Invoke-Nodi $nodi 'cmd 101' | Out-Null
        $delay = Get-Random -Minimum 0 -Maximum 40
        Start-Sleep -Milliseconds $delay
        $state = Invoke-Nodi $nodi 'state'
        $nodi.Process.Kill()
        $nodi.Process.WaitForExit(5000) | Out-Null
        $nodi = Start-Nodi $data
        $now = Invoke-Nodi $nodi 'text'
        $pictures = Invoke-Nodi $nodi 'pictures'
        Stop-Nodi $nodi
        $which = if ($now -ceq $after) { 'new version' } elseif ($now -ceq $before) { 'old version' } else { 'DAMAGED' }
        if ($which -eq 'DAMAGED' -or $pictures -ne $pictureCount) {
            $failures++
        }
        "kill ${kill}: killed $delay ms after Ctrl+S (state '$state') -> reopened as $which, $pictures pictures"
        $before = $now
    }
} finally {
    if ($hog) { Stop-Process -Id $hog.Id -Force -ErrorAction SilentlyContinue }
    Get-Process nodi -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe } |
        Stop-Process -Force -ErrorAction SilentlyContinue
}
"$Kills kills, $failures problems"
if ($MyInvocation.InvocationName -ne '.') { exit $failures }
