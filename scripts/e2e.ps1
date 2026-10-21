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

# --------------------------------------------------------------------- report

$failed = 0
foreach ($entry in $results.GetEnumerator()) {
    "{0,-60} {1}" -f $entry.Key, $entry.Value
    if ($entry.Value -like 'FAIL*') { $failed++ }
}
"--- $($results.Count - $failed) passed, $failed failed"
exit $failed
