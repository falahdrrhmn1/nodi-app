# Performance measurements against the release exe (temp data dir only).
# Usage: pwsh -File scripts\perf.ps1 [-Exe target\release\nodi.exe]
param([string]$Exe = "$PSScriptRoot\..\target\release\nodi.exe")

. "$PSScriptRoot\e2e.ps1" -Exe $Exe -Only none | Out-Null
. "$PSScriptRoot\probe-lib.ps1"

function Memory($nodi) {
    $nodi.Process.Refresh()
    "{0:N1} MB working set, {1:N1} MB private" -f ($nodi.Process.WorkingSet64 / 1MB), ($nodi.Process.PrivateMemorySize64 / 1MB)
}

$results = [ordered]@{}
$exeInfo = Get-Item $Exe
$results['exe size'] = "{0:N0} KB" -f ($exeInfo.Length / 1KB)

# Startup: three runs on an empty data dir, best and average.
$times = @()
for ($i = 0; $i -lt 3; $i++) {
    $data = New-DataDir 'perf-start'
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $nodi = Start-Nodi $data
    $nodi.Process.WaitForInputIdle(5000) | Out-Null
    $times += $watch.ElapsedMilliseconds
    if ($i -eq 0) {
        Start-Sleep -Seconds 2
        $results['RAM idle (1 empty note)'] = Memory $nodi
    }
    Stop-Nodi $nodi
}
$results['startup to idle window'] = "{0} ms best, {1:N0} ms average (3 runs)" -f ($times | Measure-Object -Minimum).Minimum, ($times | Measure-Object -Average).Average

# Ten notes with text, a picture and a formula, all open in tabs.
$png = New-TestPng (Join-Path $Root 'perf.png') 640 360
$data = New-DataDir 'perf-ten'
$nodi = Start-Nodi $data
$paragraph = ('Lorem ipsum dolor sit amet, consectetur adipiscing elit. ' * 8) + '\n'
for ($i = 0; $i -lt 10; $i++) {
    if ($i -gt 0) { Invoke-Nodi $nodi 'cmd 170' | Out-Null }
    Invoke-Nodi $nodi ("type " + ($paragraph * 10)) | Out-Null
    Invoke-Nodi $nodi "insert-image $png" | Out-Null
    Invoke-Nodi $nodi 'formula L = \frac{1}{n} \sum_{i=1}^{n} (\hat{y}_i - y_i)^2' | Out-Null
}
Wait-State $nodi 'Saved' 5000 | Out-Null
Start-Sleep -Seconds 1
$results['RAM with 10 open notes (text + picture + formula each)'] = Memory $nodi
Stop-Nodi $nodi
$watch = [Diagnostics.Stopwatch]::StartNew()
$nodi = Start-Nodi $data
$nodi.Process.WaitForInputIdle(5000) | Out-Null
$results['restart with those 10 tabs'] = "$($watch.ElapsedMilliseconds) ms"
Stop-Nodi $nodi

# One long document (~1 MB of text).
$data = New-DataDir 'perf-long'
$nodi = Start-Nodi $data
$line = 'Baris catatan panjang untuk uji performa editor Nodi Mark III dengan teks biasa.\n'
$chunk = $line * 500
$watch = [Diagnostics.Stopwatch]::StartNew()
for ($i = 0; $i -lt 25; $i++) { Invoke-Nodi $nodi "type $chunk" | Out-Null }
$results['load 12,500 lines (~1 MB) into one note'] = "$($watch.ElapsedMilliseconds) ms"
Wait-State $nodi 'Saved' 10000 | Out-Null
# Typing latency at the end of the long note: one character round trip.
$samples = @()
for ($i = 0; $i -lt 20; $i++) {
    $watch.Restart()
    Invoke-Nodi $nodi 'type x' | Out-Null
    $samples += $watch.Elapsed.TotalMilliseconds
}
$results['keystroke round trip in the 1 MB note'] = "{0:N1} ms average, {1:N1} ms worst (20 keys, includes IPC)" -f ($samples | Measure-Object -Average).Average, ($samples | Measure-Object -Maximum).Maximum
$watch.Restart()
$saved = Wait-State $nodi 'Saved' 10000
$results['autosave of the 1 MB note after typing'] = "Saved $saved ms after the last key (500 ms debounce included)"
$results['RAM with the 1 MB note'] = Memory $nodi
$size = (Get-ChildItem "$data\notes\*.rtf" | Measure-Object Length -Sum).Sum
$results['note file size on disk'] = "{0:N0} KB RTF" -f ($size / 1KB)
Stop-Nodi $nodi
$watch.Restart()
$nodi = Start-Nodi $data
$nodi.Process.WaitForInputIdle(10000) | Out-Null
$results['restart with the 1 MB note open'] = "$($watch.ElapsedMilliseconds) ms"
Stop-Nodi $nodi

foreach ($entry in $results.GetEnumerator()) { "{0,-55} {1}" -f $entry.Key, $entry.Value }
