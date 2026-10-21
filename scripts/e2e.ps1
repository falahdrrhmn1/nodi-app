# End-to-end tests: drive the real nodi.exe and check what lands on disk.
#
# Usage: pwsh -File scripts\e2e.ps1 [-Exe <nodi.exe>] [-Only A,B,...]
#
# Each test uses a throw-away data dir under target\e2e\ (never the real data).
# Nodi is started with NODI_TEST_HOOKS=1 so the script can send commands through
# WM_COPYDATA (see crates/nodi-windows/src/testhook.rs).

param(
    [string]$Exe = "$PSScriptRoot\..\target\debug\nodi.exe",
    [string[]]$Only = @()
)

$ErrorActionPreference = 'Stop'
# `pwsh -File e2e.ps1 -Only A,B` passes one string "A,B".
$Only = @($Only | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
$Exe = (Resolve-Path $Exe).Path
$Root = Join-Path (Resolve-Path "$PSScriptRoot\..").Path 'target\e2e'
New-Item -ItemType Directory -Force $Root | Out-Null

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;

public static class NodiHook {
    [StructLayout(LayoutKind.Sequential)]
    struct CopyData { public IntPtr dwData; public int cbData; public IntPtr lpData; }

    [DllImport("user32.dll")]
    static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint msg, IntPtr wparam, ref CopyData lparam,
        uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll")]
    public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr wparam, IntPtr lparam);

    delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc proc, IntPtr lparam);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern int GetClassNameW(IntPtr hwnd, StringBuilder name, int max);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr dialog, int id);

    /// Clicks button `id` of a dialog (BM_CLICK), like a user would.
    public static bool Click(IntPtr dialog, int id) {
        IntPtr button = GetDlgItem(dialog, id);
        if (button == IntPtr.Zero) return false;
        return PostMessageW(button, 0x00F5, IntPtr.Zero, IntPtr.Zero);
    }

    public static bool Send(IntPtr hwnd, string command) {
        byte[] bytes = Encoding.UTF8.GetBytes(command);
        IntPtr memory = Marshal.AllocHGlobal(Math.Max(bytes.Length, 1));
        try {
            Marshal.Copy(bytes, 0, memory, bytes.Length);
            CopyData data = new CopyData { dwData = (IntPtr)0x4E4F4454, cbData = bytes.Length, lpData = memory };
            IntPtr result;
            IntPtr ok = SendMessageTimeoutW(hwnd, 0x004A, IntPtr.Zero, ref data, 0x0002, 10000, out result);
            return ok != IntPtr.Zero && result != IntPtr.Zero;
        } finally {
            Marshal.FreeHGlobal(memory);
        }
    }

    /// First visible #32770 dialog owned by the process, or zero.
    public static IntPtr FindDialog(int processId) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((hwnd, _) => {
            uint pid;
            GetWindowThreadProcessId(hwnd, out pid);
            if (pid != processId || !IsWindowVisible(hwnd)) return true;
            StringBuilder name = new StringBuilder(64);
            GetClassNameW(hwnd, name, 64);
            if (name.ToString() == "#32770") { found = hwnd; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@

$WM_CLOSE = 0x0010
$WM_COMMAND = 0x0111

function New-DataDir([string]$name) {
    $dir = Join-Path $Root ("$name-" + [DateTime]::Now.ToString('HHmmss-fff'))
    New-Item -ItemType Directory -Force $dir | Out-Null
    return $dir
}

function Start-Nodi([string]$data, [hashtable]$extra = @{}) {
    $reply = "$data.reply.txt"
    $env:NODI_DATA_DIR = $data
    $env:NODI_TEST_HOOKS = '1'
    $env:NODI_TEST_REPLY = $reply
    foreach ($key in $extra.Keys) { Set-Item "env:$key" $extra[$key] }
    try {
        $process = Start-Process $Exe -PassThru
    } finally {
        Remove-Item env:NODI_DATA_DIR, env:NODI_TEST_HOOKS, env:NODI_TEST_REPLY -ErrorAction SilentlyContinue
        foreach ($key in $extra.Keys) { Remove-Item "env:$key" -ErrorAction SilentlyContinue }
    }
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.ElapsedMilliseconds -lt 10000) {
        $process.Refresh()
        if ($process.MainWindowHandle -ne 0) { break }
        Start-Sleep -Milliseconds 20
    }
    if ($process.MainWindowHandle -eq 0) { throw "nodi.exe did not open a window" }
    [pscustomobject]@{ Process = $process; Hwnd = $process.MainWindowHandle; Reply = $reply }
}

function Invoke-Nodi($nodi, [string]$command) {
    Remove-Item $nodi.Reply -ErrorAction SilentlyContinue
    if (-not [NodiHook]::Send($nodi.Hwnd, $command)) { throw "hook command failed: $command" }
    if (-not (Test-Path $nodi.Reply)) { return '' }
    return [IO.File]::ReadAllText($nodi.Reply, [Text.Encoding]::UTF8)
}

# Polls the save state; returns the elapsed ms, or -1 on timeout.
function Wait-State($nodi, [string]$wanted, [int]$timeoutMs = 3000) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.ElapsedMilliseconds -lt $timeoutMs) {
        if ((Invoke-Nodi $nodi 'state') -eq $wanted) { return $watch.ElapsedMilliseconds }
        Start-Sleep -Milliseconds 25
    }
    return -1
}

function Close-Dialog($nodi, [int]$button) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.ElapsedMilliseconds -lt 3000) {
        $dialog = [NodiHook]::FindDialog($nodi.Process.Id)
        if ($dialog -ne 0) {
            # A lone OK button in a MessageBox has id IDCANCEL (2), not IDOK.
            if (-not [NodiHook]::Click($dialog, $button) -and -not ($button -eq 1 -and [NodiHook]::Click($dialog, 2))) {
                [NodiHook]::PostMessageW($dialog, $WM_COMMAND, [IntPtr]$button, [IntPtr]::Zero) | Out-Null
            }
            # Wait until it is really gone, so the next call cannot hit it again.
            while ([NodiHook]::FindDialog($nodi.Process.Id) -eq $dialog -and
                   $watch.ElapsedMilliseconds -lt 5000) { Start-Sleep -Milliseconds 20 }
            return $true
        }
        Start-Sleep -Milliseconds 50
    }
    return $false
}

function Stop-Nodi($nodi, [int]$answerDialog = 0) {
    [NodiHook]::PostMessageW($nodi.Hwnd, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if ($answerDialog -ne 0) { Close-Dialog $nodi $answerDialog | Out-Null }
    if (-not $nodi.Process.WaitForExit(10000)) {
        $nodi.Process.Kill()
        throw 'nodi.exe did not exit within 10 s'
    }
}

function Read-Tree([string]$data) {
    Get-Content -Raw -Encoding utf8 (Join-Path $data 'tree.json') | ConvertFrom-Json
}

$results = [ordered]@{}
function Test-Case([string]$id, [string]$name, [scriptblock]$body) {
    if ($Only.Count -gt 0 -and $Only -notcontains $id) { return }
    try {
        $detail = & $body
        $results["$id $name"] = "PASS $detail"
    } catch {
        $results["$id $name"] = "FAIL $($_.Exception.Message)"
    }
    Get-Process nodi -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $Exe } | Stop-Process -Force -ErrorAction SilentlyContinue
}
function Assert([bool]$condition, [string]$message) { if (-not $condition) { throw $message } }

# ----------------------------------------------------------------------- tests

Test-Case 'A' 'quick note survives restart without Ctrl+S' {
    $data = New-DataDir 'A'
    $nodi = Start-Nodi $data
    Assert ((Invoke-Nodi $nodi 'tabs') -match 'Untitled') 'first run did not open an Untitled note'
    $text = "Halo dunia — catatan 😀 ünïcödé"
    Invoke-Nodi $nodi "type $text" | Out-Null
    $ms = Wait-State $nodi 'Saved' 3000
    Assert ($ms -ge 0) "state never became Saved (last: $(Invoke-Nodi $nodi 'state'))"
    Stop-Nodi $nodi
    $hit = Get-ChildItem "$data\notes\*.txt" | Where-Object {
        [IO.File]::ReadAllText($_.FullName, [Text.Encoding]::UTF8).Contains($text) }
    Assert ($null -ne $hit) 'text is not in any notes\*.txt sidecar'
    $nodi = Start-Nodi $data
    $reloaded = Invoke-Nodi $nodi 'text'
    Stop-Nodi $nodi
    Assert ($reloaded.Contains($text)) "reloaded text differs: '$reloaded'"
    "(Saved after ${ms} ms)"
}

Test-Case 'B' 'nested notes keep hierarchy after rename + restart' {
    $data = New-DataDir 'B'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'cmd 170' | Out-Null            # New Note (root sibling)
    Invoke-Nodi $nodi 'rename-selected A' | Out-Null
    Invoke-Nodi $nodi 'cmd 172' | Out-Null            # New Child Note
    Invoke-Nodi $nodi 'rename-selected B' | Out-Null
    Invoke-Nodi $nodi 'cmd 172' | Out-Null
    Invoke-Nodi $nodi 'rename-selected C' | Out-Null
    Assert ((Invoke-Nodi $nodi 'select-title B') -eq 'ok') 'could not select B'
    Invoke-Nodi $nodi 'rename-selected B renamed' | Out-Null
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    Stop-Nodi $nodi
    $nodes = (Read-Tree $data).nodes
    $a = $nodes | Where-Object title -eq 'A'
    $b = $nodes | Where-Object title -eq 'B renamed'
    $c = $nodes | Where-Object title -eq 'C'
    Assert ($a -and $b -and $c) "missing nodes: $(($nodes | ForEach-Object title) -join ', ')"
    Assert ($b.parent -eq $a.id) 'B is not under A'
    Assert ($c.parent -eq $b.id) 'C is not under B'
    Assert ($null -eq $a.parent) 'A is not at the root'
    ''
}

Test-Case 'C' 'bold survives restart' {
    $data = New-DataDir 'C'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'type plain bold text' | Out-Null
    Invoke-Nodi $nodi 'select 6 10' | Out-Null
    Invoke-Nodi $nodi 'cmd 120' | Out-Null            # Format > Bold
    Invoke-Nodi $nodi 'select 11 15' | Out-Null
    Invoke-Nodi $nodi 'cmd 121' | Out-Null            # Format > Italic
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    $bold = Invoke-Nodi $nodi 'fmt 6 10'
    $italic = Invoke-Nodi $nodi 'fmt 11 15'
    $plain = Invoke-Nodi $nodi 'fmt 0 5'
    Stop-Nodi $nodi
    Assert ($bold -match 'bold=1') "bold lost: $bold"
    Assert ($italic -match 'italic=1') "italic lost: $italic"
    Assert ($plain -match 'bold=0 italic=0') "plain text changed: $plain"
    ''
}

Test-Case 'E' 'write failure shows Save Failed, never Saved' {
    $data = New-DataDir 'E'
    $nodi = Start-Nodi $data @{ NODI_FAIL_WRITES = '1' }
    Invoke-Nodi $nodi 'type must not be reported as saved' | Out-Null
    $sawSaved = $false
    $failed = $false
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.ElapsedMilliseconds -lt 3000) {
        $state = Invoke-Nodi $nodi 'state'
        if ($state -eq 'Saved') { $sawSaved = $true }
        if ($state -eq 'Save Failed') { $failed = $true; break }
        Start-Sleep -Milliseconds 25
    }
    Assert (Close-Dialog $nodi 1) 'no error dialog was shown'
    Assert (-not $sawSaved) 'state claimed Saved while writes fail'
    Assert $failed 'state never became Save Failed'
    Stop-Nodi $nodi 6                                    # "Close anyway?" -> Yes
    $leaked = Get-ChildItem "$data\notes\*.txt" -ErrorAction SilentlyContinue | Where-Object {
        [IO.File]::ReadAllText($_.FullName).Contains('must not be reported') }
    Assert ($null -eq $leaked) 'text reached disk although writes were failing'
    ''
}

function Get-Units($nodi) { (Invoke-Nodi $nodi 'units') -split ' ' }
function Count-Unit($nodi, [string]$hex) { @(Get-Units $nodi | Where-Object { $_ -eq $hex }).Count }

Test-Case 'F' 'Enter keeps indentation, Tab/Shift+Tab indent lines' {
    $data = New-DataDir 'F'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'type \tif x:' | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'type y = 1' | Out-Null
    $text = (Invoke-Nodi $nodi 'text') -replace "`r", '|'
    Assert ($text -like "`tif x:|`ty = 1*") "auto-indent failed: '$text'"
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'shift-tab' | Out-Null
    Invoke-Nodi $nodi 'type done' | Out-Null
    $text = (Invoke-Nodi $nodi 'text') -replace "`r", '|'
    Assert ($text -like "`tif x:|`ty = 1|done*") "shift-tab did not outdent: '$text'"
    Invoke-Nodi $nodi 'select 0 30' | Out-Null
    Invoke-Nodi $nodi 'tab' | Out-Null
    $text = (Invoke-Nodi $nodi 'text') -replace "`r", '|'
    Assert ($text -like "`t`tif x:|`t`ty = 1|`tdone*") "block indent failed: '$text'"
    Invoke-Nodi $nodi 'shift-tab' | Out-Null
    $text = (Invoke-Nodi $nodi 'text') -replace "`r", '|'
    Assert ($text -like "`tif x:|`ty = 1|done*") "block outdent failed: '$text'"
    Stop-Nodi $nodi
    ''
}

Test-Case 'G' 'image persists and resizes' {
    . "$PSScriptRoot\probe-lib.ps1"
    $png = New-TestPng (Join-Path $Root 'e2e-image.png') 240 120
    $data = New-DataDir 'G'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'type Picture:\n' | Out-Null
    $inserted = Invoke-Nodi $nodi "insert-image $png"
    Assert ($inserted -eq 'ok') "insert failed: $inserted"
    Invoke-Nodi $nodi 'type \nafter' | Out-Null
    Invoke-Nodi $nodi 'select 9 9' | Out-Null
    Invoke-Nodi $nodi 'cmd 242' | Out-Null                # Image 50%
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    $pictures = Invoke-Nodi $nodi 'pictures'
    Invoke-Nodi $nodi 'select 9 10' | Out-Null
    $rtf = Invoke-Nodi $nodi 'sel-rtf'
    $text = (Invoke-Nodi $nodi 'text') -replace "`r", '|'
    Stop-Nodi $nodi
    Assert ($pictures -eq '1') "expected 1 picture after restart, got $pictures"
    Assert ($rtf -match 'picwgoal1800') "picture was not resized to 50% (120 px = 1800 twips)"
    Assert ($text -like 'Picture:|*|after*') "text around the picture moved: '$text'"
    ''
}

Test-Case 'H' 'table: insert, add/delete rows and columns, persist' {
    $data = New-DataDir 'H'
    $nodi = Start-Nodi $data
    Assert ((Invoke-Nodi $nodi 'table 2 3') -eq 'ok') 'table insert failed'
    Assert ((Count-Unit $nodi 'FFF9') -eq 2) 'expected 2 rows'
    Assert ((Count-Unit $nodi '0007') -eq 6) 'expected 6 cells'
    Invoke-Nodi $nodi 'cmd 231' | Out-Null                # row below
    Assert ((Count-Unit $nodi 'FFF9') -eq 3) "row insert failed: $(Count-Unit $nodi 'FFF9') rows"
    Invoke-Nodi $nodi 'cmd 233' | Out-Null                # column right
    Assert ((Count-Unit $nodi '0007') -eq 12) "column insert failed: $(Count-Unit $nodi '0007') cells"
    Invoke-Nodi $nodi 'cmd 235' | Out-Null                # delete column
    Assert ((Count-Unit $nodi '0007') -eq 9) "column delete failed: $(Count-Unit $nodi '0007') cells"
    Invoke-Nodi $nodi 'cmd 234' | Out-Null                # delete row (the middle one)
    Assert ((Count-Unit $nodi 'FFF9') -eq 2) "row delete failed: $(Count-Unit $nodi 'FFF9') rows"
    $units = (Invoke-Nodi $nodi 'units')
    Assert (-not $units.Contains('FFFB 000D 000D FFF9')) "deleting a row split the table: $units"
    Invoke-Nodi $nodi 'select 2 2' | Out-Null
    Invoke-Nodi $nodi 'type Nama' | Out-Null
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    $rows = Count-Unit $nodi 'FFF9'
    $cells = Count-Unit $nodi '0007'
    $markdown = Invoke-Nodi $nodi 'markdown'
    Stop-Nodi $nodi
    Assert ($rows -eq 2 -and $cells -eq 6) "after restart: $rows rows, $cells cells"
    Assert ($markdown -match '\| Nama \|') "markdown table missing: $markdown"
    ''
}

Test-Case 'I' 'code block keeps indentation and exports as fenced code' {
    $data = New-DataDir 'I'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'type Contoh:' | Out-Null
    Invoke-Nodi $nodi 'cmd 205' | Out-Null                # code block
    Invoke-Nodi $nodi 'type fn main() {' | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'tab' | Out-Null
    Invoke-Nodi $nodi 'type x();' | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'type y();' | Out-Null
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    $markdown = Invoke-Nodi $nodi 'markdown'
    Stop-Nodi $nodi
    $fence = [string]::new([char]96, 3)
    $expected = "$fence`nfn main() {`n`tx();`n`ty();`n$fence"
    Assert ($markdown.Contains($expected)) "code block lost after restart:`n$markdown"
    ''
}

Test-Case 'J' 'formulas render, persist and keep their LaTeX' {
    $data = New-DataDir 'J'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'type Energi: ' | Out-Null
    $first = Invoke-Nodi $nodi 'formula E = mc^2'
    Assert ($first -eq 'ok') "formula 1: $first"
    Invoke-Nodi $nodi 'type \nLoss: ' | Out-Null
    $loss = 'L = \frac{1}{n} \sum_{i=1}^{n} (\hat{y}_i - y_i)^2'
    $second = Invoke-Nodi $nodi "formula $loss"
    Assert ($second -eq 'ok') "formula 2: $second"
    $bad = Invoke-Nodi $nodi 'formula \frac{a'
    Assert ($bad -like 'error:*') "a broken formula should be rejected, got: $bad"
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    $pictures = Invoke-Nodi $nodi 'pictures'
    $markdown = Invoke-Nodi $nodi 'markdown'
    Invoke-Nodi $nodi 'select 8 9' | Out-Null
    $source = Invoke-Nodi $nodi 'formula-source'
    Stop-Nodi $nodi
    Assert ($pictures -eq '2') "expected 2 formula pictures, got $pictures"
    Assert ($markdown.Contains('$E = mc^2$')) "markdown lost formula 1: $markdown"
    Assert ($markdown.Contains("`$$loss`$")) "markdown lost formula 2: $markdown"
    Assert ($source -eq 'E = mc^2') "formula source not editable: '$source'"
    ''
}

Test-Case 'K' 'links: web and internal note links survive restart' {
    $data = New-DataDir 'K'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'type Lihat juga: ' | Out-Null
    Invoke-Nodi $nodi 'link Gradient Descent|nodi://note/0123456789abcdef0123456789abcdef' | Out-Null
    Invoke-Nodi $nodi 'type  dan ' | Out-Null
    Invoke-Nodi $nodi 'link docs|https://example.com/a b' | Out-Null
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    $markdown = Invoke-Nodi $nodi 'markdown'
    Invoke-Nodi $nodi 'select 13 20' | Out-Null
    $target = Invoke-Nodi $nodi 'link-target'
    Stop-Nodi $nodi
    Assert ($markdown.Contains('[Gradient Descent](nodi://note/0123456789abcdef0123456789abcdef)')) "internal link lost: $markdown"
    Assert ($markdown.Contains('[docs](https://example.com/a%20b)')) "web link lost: $markdown"
    Assert ($target -eq 'nodi://note/0123456789abcdef0123456789abcdef') "link target not readable: '$target'"
    ''
}

Test-Case 'L' 'headings, lists and Enter-after-heading' {
    $data = New-DataDir 'L'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'type Machine Learning' | Out-Null
    Invoke-Nodi $nodi 'cmd 210' | Out-Null                # Heading 1
    Invoke-Nodi $nodi 'select 16 16' | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'type Badan teks.' | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'cmd 214' | Out-Null                # bullets
    Invoke-Nodi $nodi 'type satu' | Out-Null
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    $markdown = Invoke-Nodi $nodi 'markdown'
    Stop-Nodi $nodi
    Assert ($markdown.StartsWith("# Machine Learning`n`nBadan teks.")) "heading/body wrong:`n$markdown"
    Assert ($markdown.Contains('- satu')) "list lost:`n$markdown"
    ''
}

Test-Case 'M' 'many notes stay responsive' {
    $data = New-DataDir 'M'
    $nodi = Start-Nodi $data
    $watch = [Diagnostics.Stopwatch]::StartNew()
    for ($i = 0; $i -lt 150; $i++) { Invoke-Nodi $nodi 'cmd 170' | Out-Null }
    $create = $watch.ElapsedMilliseconds
    Stop-Nodi $nodi
    $watch.Restart()
    $nodi = Start-Nodi $data
    $start = $watch.ElapsedMilliseconds
    $responding = $nodi.Process.Responding
    Stop-Nodi $nodi
    $count = @((Read-Tree $data).nodes).Count
    Assert ($count -eq 151) "expected 151 notes, found $count"
    Assert $responding 'not responding after restart'
    "(150 notes created in $create ms, restart to window $start ms)"
}

Test-Case 'O' 'long session: switching tabs while typing loses nothing' {
    $data = New-DataDir 'O'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'rename-selected Satu' | Out-Null
    Invoke-Nodi $nodi 'cmd 170' | Out-Null
    Invoke-Nodi $nodi 'rename-selected Dua' | Out-Null
    Invoke-Nodi $nodi 'cmd 170' | Out-Null
    Invoke-Nodi $nodi 'rename-selected Tiga' | Out-Null
    for ($round = 1; $round -le 20; $round++) {
        for ($tab = 0; $tab -lt 3; $tab++) {
            Invoke-Nodi $nodi "type r$round-t$tab " | Out-Null
            Invoke-Nodi $nodi 'cmd 150' | Out-Null          # next tab (flushes)
        }
    }
    Stop-Nodi $nodi
    $texts = Get-ChildItem "$data\notes\*.txt" | ForEach-Object {
        [IO.File]::ReadAllText($_.FullName, [Text.Encoding]::UTF8) }
    $all = $texts -join "`n"
    $missing = @()
    for ($round = 1; $round -le 20; $round++) {
        for ($tab = 0; $tab -lt 3; $tab++) {
            if (-not $all.Contains("r$round-t$tab ")) { $missing += "r$round-t$tab" }
        }
    }
    Assert ($missing.Count -eq 0) "lost tokens: $($missing -join ', ')"
    '(60 edits across 3 tabs)'
}

# --------------------------------------------------------------------- report

$failed = 0
foreach ($entry in $results.GetEnumerator()) {
    "{0,-60} {1}" -f $entry.Key, $entry.Value
    if ($entry.Value -like 'FAIL*') { $failed++ }
}
"--- $($results.Count - $failed) passed, $failed failed"
exit $failed
