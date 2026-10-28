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
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr hwnd, int command);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] static extern IntPtr GetThreadDesktop(uint threadId);
    [DllImport("user32.dll")] static extern bool SwitchDesktop(IntPtr hDesktop);
    [DllImport("user32.dll")] static extern bool AttachThreadInput(uint attach, uint to, bool on);
    [DllImport("user32.dll")] static extern void SwitchToThisWindow(IntPtr hwnd, bool altTab);
    [DllImport("user32.dll")] static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);

    // Raw clipboard write: .NET's DataObject turns a custom-format byte[] into
    // the string "System.Byte[]", so CF_HTML is put there with Win32 calls.
    [DllImport("user32.dll")] static extern bool OpenClipboard(IntPtr hwnd);
    [DllImport("user32.dll")] static extern bool EmptyClipboard();
    [DllImport("user32.dll")] static extern bool CloseClipboard();
    [DllImport("user32.dll")] static extern IntPtr SetClipboardData(uint format, IntPtr memory);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern uint RegisterClipboardFormatW(string name);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalAlloc(uint flags, UIntPtr size);
    [DllImport("kernel32.dll")] static extern IntPtr GlobalLock(IntPtr memory);
    [DllImport("kernel32.dll")] static extern bool GlobalUnlock(IntPtr memory);

    static IntPtr CopyBytes(byte[] bytes) {
        IntPtr memory = GlobalAlloc(0x0042, (UIntPtr)(bytes.Length + 1));
        if (memory == IntPtr.Zero) return IntPtr.Zero;
        IntPtr target = GlobalLock(memory);
        Marshal.Copy(bytes, 0, target, bytes.Length);
        Marshal.WriteByte(target, bytes.Length, 0);
        GlobalUnlock(memory);
        return memory;
    }

    /// Puts CF_HTML (bytes) and CF_UNICODETEXT on the clipboard.
    public static bool SetHtml(byte[] html, string text) {
        if (!OpenClipboard(IntPtr.Zero)) return false;
        try {
            EmptyClipboard();
            IntPtr htmlMemory = CopyBytes(html);
            if (htmlMemory != IntPtr.Zero) {
                SetClipboardData(RegisterClipboardFormatW("HTML Format"), htmlMemory);
            }
            byte[] utf16 = Encoding.Unicode.GetBytes(text + "\0");
            IntPtr textMemory = CopyBytes(utf16);
            if (textMemory != IntPtr.Zero) SetClipboardData(13, textMemory); // CF_UNICODETEXT
            return true;
        } finally {
            CloseClipboard();
        }
    }

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

    /// Sends a test command from a background thread so the caller can keep
    /// answering a modal dialog the command opens. Returns the worker thread.
    public static System.Threading.Thread SendAsync(IntPtr hwnd, string command) {
        System.Threading.Thread thread = new System.Threading.Thread(() => { Send(hwnd, command); });
        thread.IsBackground = true;
        thread.Start();
        return thread;
    }

    /// Brings a window to the foreground from a PowerShell thread. The
    /// `AttachThreadInput` dance is the usual workaround for the foreground
    /// lock; the return value is checked by comparing `GetForegroundWindow`.
    public static bool Foreground(IntPtr hwnd) {
        IntPtr cur = GetThreadDesktop(GetCurrentThreadId());
        if (cur != IntPtr.Zero) SwitchDesktop(cur);
        ShowWindow(hwnd, 9); // SW_RESTORE
        // The "Alt trick": a synthetic Alt press ends the foreground lock.
        keybd_event(0x12, 0, 0, UIntPtr.Zero);
        IntPtr existing = GetForegroundWindow();
        uint existingPid;
        uint existingThread = existing == IntPtr.Zero ? 0 : GetWindowThreadProcessId(existing, out existingPid);
        uint current = GetCurrentThreadId();
        bool attached = existingThread != 0 && existingThread != current &&
            AttachThreadInput(current, existingThread, true);
        SetForegroundWindow(hwnd);
        if (attached) AttachThreadInput(current, existingThread, false);
        keybd_event(0x12, 0, 2, UIntPtr.Zero);
        System.Threading.Thread.Sleep(100);
        if (GetForegroundWindow() != hwnd) {
            SwitchToThisWindow(hwnd, false);
            System.Threading.Thread.Sleep(150);
        }
        return GetForegroundWindow() == hwnd;
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

# tree.json is written by a background thread, so a read right after a command
# can still see the previous state. Poll until the check passes.
function Wait-Tree([string]$data, [scriptblock]$check, [int]$timeoutMs = 3000) {
    $deadline = [DateTime]::Now.AddMilliseconds($timeoutMs)
    $nodes = @()
    while ([DateTime]::Now -lt $deadline) {
        try {
            $nodes = @((Read-Tree $data).nodes)
        } catch {
            $nodes = @()
        }
        if ($nodes.Count -gt 0 -and (& $check $nodes)) { break }
        Start-Sleep -Milliseconds 50
    }
    return , $nodes
}

function Root-Titles($nodes) {
    return (@($nodes | Where-Object { $null -eq $_.parent } | Sort-Object order |
        ForEach-Object title) -join ',')
}

# Puts a CF_HTML stream (with real StartFragment/EndFragment offsets) on the
# clipboard, the way a browser does when copying a selection.
function Set-ClipboardHtml([string]$fragment) {
    Add-Type -AssemblyName System.Windows.Forms
    $body = "<html><body><!--StartFragment-->$fragment<!--EndFragment--></body></html>"
    $template = "Version:0.9`r`nStartHTML:{0:D10}`r`nEndHTML:{1:D10}`r`nStartFragment:{2:D10}`r`nEndFragment:{3:D10}`r`n"
    $headerLength = $template.Length
    $start = $headerLength + $body.IndexOf('<!--StartFragment-->') + 20
    $end = $headerLength + $body.IndexOf('<!--EndFragment-->')
    $header = [string]::Format($template, $headerLength, $headerLength + $body.Length, $start, $end)
    $text = $fragment -replace '<[^>]+>', ' '
    if (-not [NodiHook]::SetHtml([Text.Encoding]::UTF8.GetBytes($header + $body), $text)) {
        throw 'could not write CF_HTML to the clipboard'
    }
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

function Get-TabTwips($nodi, [int]$index) {
    $raw = Invoke-Nodi $nodi "tabpos $index"
    return [int]([regex]::Match($raw, 'twips=(-?\d+)').Groups[1].Value)
}

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
    Assert ($text -like "`tif x:|`ty = 1|done*") "block tab changed text: '$text'"
    Invoke-Nodi $nodi 'select 0 0' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    Assert ($para -like 'start=360*') "block indent missing: $para"
    Invoke-Nodi $nodi 'select 0 30' | Out-Null
    Invoke-Nodi $nodi 'shift-tab' | Out-Null
    Invoke-Nodi $nodi 'select 0 0' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    $text = (Invoke-Nodi $nodi 'text') -replace "`r", '|'
    Assert ($para -like 'start=0*') "block outdent left an indent: $para"
    Assert ($text -like "`tif x:|`ty = 1|done*") "block outdent changed text: '$text'"
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
    Invoke-Nodi $nodi 'select 18 18' | Out-Null
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

Test-Case 'P' 'Heading 4 and Normal via the format bar and markdown' {
    $data = New-DataDir 'P'
    $nodi = Start-Nodi $data
    $combos = (Invoke-Nodi $nodi 'combos') -split ' '
    Assert ($combos.Count -eq 3 -and $combos -notcontains 'none') "format combos missing: $($combos -join ' ')"
    Assert ($combos[0] -match '^(\d+)x' -and [int]$Matches[1] -ge 80) "style combo not placed: $($combos -join ' ')"
    Invoke-Nodi $nodi 'type Judul empat' | Out-Null
    Invoke-Nodi $nodi 'cmd 223' | Out-Null            # Format > Heading 4
    Assert ((Invoke-Nodi $nodi 'style') -eq 'Heading 4') "style combo shows '$(Invoke-Nodi $nodi 'style')'"
    Assert ((Invoke-Nodi $nodi 'fmt 0 5') -match 'size=250') "H4 size wrong: $(Invoke-Nodi $nodi 'fmt 0 5')"
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    $heading = (Invoke-Nodi $nodi 'markdown') -replace "`0", ''
    Assert ((Invoke-Nodi $nodi 'style') -eq 'Heading 4') 'H4 style not shown after restart'
    Invoke-Nodi $nodi 'style-set 0' | Out-Null        # combobox -> Normal
    Assert ((Invoke-Nodi $nodi 'style') -eq 'Normal') "Normal not shown: '$(Invoke-Nodi $nodi 'style')'"
    $plain = (Invoke-Nodi $nodi 'markdown') -replace "`0", ''
    Stop-Nodi $nodi
    Assert ($heading.Trim() -eq '#### Judul empat') "H4 markdown wrong: len=$($heading.Length) hex=$([BitConverter]::ToString([Text.Encoding]::UTF8.GetBytes($heading)))"
    Assert ($plain.Trim() -eq 'Judul empat') "Normal markdown wrong:`n$plain"
    "(combos $($combos -join ' '))"
}

Test-Case 'Q' 'tab stops are 360 twips (480 in code blocks)' {
    $data = New-DataDir 'Q'
    $nodi = Start-Nodi $data
    # Measure the distance between two stops, so the control's left inset and
    # the paragraph indent cancel out. Text "\t\tX": stop 1 at index 1, stop 2 at index 2.
    Invoke-Nodi $nodi 'type \t\tX' | Out-Null
    $first = (Get-TabTwips $nodi 2) - (Get-TabTwips $nodi 1)
    Assert ($first -ge 330 -and $first -le 390) "tab step is $first twips, expected 360"
    Invoke-Nodi $nodi 'cmd 170' | Out-Null            # new note
    Invoke-Nodi $nodi 'cmd 205' | Out-Null            # code block
    Invoke-Nodi $nodi 'type \tY' | Out-Null
    $units = Get-Units $nodi
    $tabIndex = [Array]::LastIndexOf($units, '0009')
    $code = (Get-TabTwips $nodi ($tabIndex + 1)) - (Get-TabTwips $nodi $tabIndex)
    Assert ($code -ge 450 -and $code -le 510) "code tab step is $code twips (units=$($units -join ' '))"
    Stop-Nodi $nodi
    "(body $first twips, code $code twips)"
}

Test-Case 'S' 'Trash asks for confirmation, counts sub-notes, Empty Trash purges' {
    $data = New-DataDir 'S'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'cmd 170' | Out-Null
    Invoke-Nodi $nodi 'rename-selected Induk' | Out-Null
    Invoke-Nodi $nodi 'cmd 172' | Out-Null            # child note
    Invoke-Nodi $nodi 'rename-selected Anak' | Out-Null
    Assert ((Invoke-Nodi $nodi 'trashitem') -eq 'Trash (0)') "initial pseudo-item: '$(Invoke-Nodi $nodi 'trashitem')'"
    Invoke-Nodi $nodi 'select-title Induk' | Out-Null
    # The hook blocks while the confirmation runs, so send from a worker thread
    # and answer the dialog from the main one.
    [NodiHook]::SendAsync($nodi.Hwnd, 'cmd 178') | Out-Null
    Assert (Close-Dialog $nodi 6) 'no confirmation before moving to the Trash'
    Start-Sleep -Milliseconds 150
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.ElapsedMilliseconds -lt 3000 -and (Invoke-Nodi $nodi 'trashitem') -ne 'Trash (1)') {
        Start-Sleep -Milliseconds 25
    }
    Assert ((Invoke-Nodi $nodi 'trashitem') -eq 'Trash (1)') "pseudo-item after trash: '$(Invoke-Nodi $nodi 'trashitem')'"
    $trashed = (Read-Tree $data).nodes | Where-Object title -eq 'Induk'
    Assert ($null -ne $trashed -and $trashed.trashed) 'Induk is not marked trashed in tree.json'
    [NodiHook]::SendAsync($nodi.Hwnd, 'cmd 185') | Out-Null   # Empty Trash...
    Assert (Close-Dialog $nodi 6) 'no confirmation before emptying the Trash'
    Start-Sleep -Milliseconds 150
    $watch.Restart()
    while ($watch.ElapsedMilliseconds -lt 3000 -and (Invoke-Nodi $nodi 'trashitem') -ne 'Trash (0)') {
        Start-Sleep -Milliseconds 25
    }
    Assert ((Invoke-Nodi $nodi 'trashitem') -eq 'Trash (0)') "pseudo-item after emptying: '$(Invoke-Nodi $nodi 'trashitem')'"
    Stop-Nodi $nodi
    $remaining = (Read-Tree $data).nodes | Where-Object title -in 'Induk', 'Anak'
    Assert ($null -eq $remaining) 'purged notes are still in tree.json'
    $recovered = @(Get-ChildItem "$data\recovery" -ErrorAction SilentlyContinue)
    Assert ($recovered.Count -gt 0) 'nothing was moved to data/recovery'
    ''
}

Test-Case 'R' 'real keyboard: foreground window, Ctrl+A/B, Ctrl+N, F2 rename' {
    $data = New-DataDir 'R'
    $nodi = Start-Nodi $data
    if (-not [NodiHook]::Foreground($nodi.Hwnd)) {
        Stop-Nodi $nodi
        return 'NOT TESTED: nodi.exe could not be brought to the foreground'
    }
    Add-Type -AssemblyName System.Windows.Forms
    Start-Sleep -Milliseconds 250
    # Releasing Alt alone (the foreground trick) enters menu-bar mode, which
    # would eat the next key as a mnemonic; Esc leaves it, as a user would.
    [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
    Start-Sleep -Milliseconds 100
    [System.Windows.Forms.SendKeys]::SendWait('Kunci asli')
    Start-Sleep -Milliseconds 250
    $text = Invoke-Nodi $nodi 'text'
    Assert ($text.Contains('Kunci asli')) "real keys did not reach the editor: '$text'"
    [System.Windows.Forms.SendKeys]::SendWait('^a')
    Start-Sleep -Milliseconds 150
    [System.Windows.Forms.SendKeys]::SendWait('^b')
    Start-Sleep -Milliseconds 250
    $bold = Invoke-Nodi $nodi 'fmt 0 5'
    Assert ($bold -match 'bold=1') "Ctrl+B did not bold the selection: $bold"
    [System.Windows.Forms.SendKeys]::SendWait('^n')
    Start-Sleep -Milliseconds 300
    $tabs = Invoke-Nodi $nodi 'tabs'
    Assert (@($tabs -split "`n")[1].Split('|').Count -eq 2) "Ctrl+N did not open a second tab: $tabs"
    [System.Windows.Forms.SendKeys]::SendWait('{F2}')
    Start-Sleep -Milliseconds 400
    [System.Windows.Forms.SendKeys]::SendWait('Renamed{ENTER}')
    Start-Sleep -Milliseconds 300
    Stop-Nodi $nodi
    $renamed = @((Read-Tree $data).nodes | Where-Object title -eq 'Renamed')
    Assert ($renamed.Count -eq 1) "F2 rename did not reach tree.json: $((Read-Tree $data).nodes | ForEach-Object title)"
    ''
}

# -------------------------------------------------------------------- batch 2

Test-Case 'T' 'Markdown lists: typing, nesting, Backspace keeps indent' {
    $data = New-DataDir 'T'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'keys - satu' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    Assert ($para -match 'numbering=1' -and $para -match 'level=1' -and $para -match 'text="satu"') "bullet not formed: $para"
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'keys dua' | Out-Null
    Invoke-Nodi $nodi 'tab' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    Assert ($para -match 'start=360' -and $para -match 'level=2' -and $para -match 'numbering=0') "Tab did not nest: $para"
    Invoke-Nodi $nodi 'shift-tab' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    Assert ($para -match 'start=0' -and $para -match 'level=1' -and $para -match 'numbering=1') "Shift+Tab did not unnest: $para"
    Invoke-Nodi $nodi 'select 0 0' | Out-Null
    Invoke-Nodi $nodi 'backspace' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    $text = (Invoke-Nodi $nodi 'text') -replace "`r", '|'
    Assert ($para -match 'numbering=0' -and $para -match 'start=360' -and $para -match 'text="satu"') "Backspace changed the wrong things: $para"
    Assert ($text -eq 'satu|dua') "text changed: '$text'"
    Invoke-Nodi $nodi 'select 6 6' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    Assert ($para -match 'numbering=1' -and $para -match 'text="dua"') "second item lost: $para"
    ''
}

Test-Case 'U' 'empty item ends the list, Ctrl+Z restores a typed marker' {
    $data = New-DataDir 'U'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'keys - ' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    Assert ($para -match 'numbering=1' -and $para -match 'text=""') "empty bullet not created: $para"
    Invoke-Nodi $nodi 'cmd 110' | Out-Null                 # Undo the conversion
    $text = Invoke-Nodi $nodi 'text'
    $para = Invoke-Nodi $nodi 'para'
    Assert ($text -eq '- ') "Undo did not restore the typed text: '$text'"
    Assert ($para -match 'numbering=0' -and $para -match 'start=0') "Undo left list formatting: $para"

    Invoke-Nodi $nodi 'select 0 2' | Out-Null
    Invoke-Nodi $nodi 'type ' | Out-Null
    Invoke-Nodi $nodi 'keys - alpha' | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'keys beta' | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null                   # empty item -> end list
    Invoke-Nodi $nodi 'keys tail' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    Assert ($para -match 'numbering=0' -and $para -match 'start=360' -and $para -match 'text="tail"') "empty item did not end the list: $para"
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi

    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'select 0 0' | Out-Null
    $first = Invoke-Nodi $nodi 'para'
    Invoke-Nodi $nodi 'select 6 6' | Out-Null
    $second = Invoke-Nodi $nodi 'para'
    $markdown = (Invoke-Nodi $nodi 'markdown') -replace "`0", ''
    Stop-Nodi $nodi
    Assert ($first -match 'numbering=1' -and $first -match 'text="alpha"') "first item lost: $first"
    Assert ($second -match 'numbering=1' -and $second -match 'text="beta"') "second item lost: $second"
    Assert ($markdown.Contains('- alpha') -and $markdown.Contains('- beta')) "list markdown missing:`n$markdown"
    ''
}

Test-Case 'V' 'paragraph indentation survives wrapping and restart' {
    $data = New-DataDir 'V'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'keys baris panjang' | Out-Null
    Invoke-Nodi $nodi 'select 0 0' | Out-Null
    Invoke-Nodi $nodi 'tab' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    Assert ($para -match 'start=360' -and $para -match 'offset=0') "Tab did not indent the paragraph: $para"
    Invoke-Nodi $nodi 'select 13 13' | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'keys baris kedua' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    Assert ($para -match 'start=360') "Enter lost the indent: $para"
    $markdown = (Invoke-Nodi $nodi 'markdown') -replace "`0", ''
    $indent = [string][char]0x00A0 + [char]0x00A0
    Assert ($markdown.Contains("$indent" + 'baris panjang') -and $markdown.Contains("$indent" + 'baris kedua')) "markdown indent missing:`n$markdown"
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'select 0 0' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    Stop-Nodi $nodi
    Assert ($para -match 'start=360') "indent lost after restart: $para"
    ''
}

Test-Case 'W' 'Markdown headings: hidden marker, level changes, restart' {
    $data = New-DataDir 'W'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'keys ## Judul' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    Assert ($para -match 'text="## Judul"') "marker not kept: $para"
    Assert ((Invoke-Nodi $nodi 'fmt 0 3') -match 'bold=1.*size=320') "H2 formatting missing: $(Invoke-Nodi $nodi 'fmt 0 3')"
    Assert ((Invoke-Nodi $nodi 'hidden 0 3') -eq 'false') 'marker is not visible while editing'
    Invoke-Nodi $nodi 'enter' | Out-Null
    Assert ((Invoke-Nodi $nodi 'hidden 0 3') -eq 'true') 'marker is not hidden after leaving the heading'
    Invoke-Nodi $nodi 'keys isi' | Out-Null
    # One more `#` upgrades the heading; deleting it goes back.
    Invoke-Nodi $nodi 'select 0 0' | Out-Null
    Invoke-Nodi $nodi 'keys #' | Out-Null
    Assert ((Invoke-Nodi $nodi 'fmt 0 4') -match 'size=280') "adding # did not make H3: $(Invoke-Nodi $nodi 'fmt 0 4')"
    Invoke-Nodi $nodi 'select 0 1' | Out-Null
    Invoke-Nodi $nodi 'type ' | Out-Null
    Assert ((Invoke-Nodi $nodi 'fmt 0 3') -match 'size=320') "deleting # did not restore H2: $(Invoke-Nodi $nodi 'fmt 0 3')"
    Invoke-Nodi $nodi 'select 12 12' | Out-Null
    Assert ((Invoke-Nodi $nodi 'hidden 0 3') -eq 'true') 'marker stayed visible outside the heading'
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi

    $nodi = Start-Nodi $data
    Assert ((Invoke-Nodi $nodi 'hidden 0 3') -eq 'true') 'marker was visible after restart'
    Invoke-Nodi $nodi 'select 0 0' | Out-Null
    $para = Invoke-Nodi $nodi 'para'
    $markdown = (Invoke-Nodi $nodi 'markdown') -replace "`0", ''
    $inside = Invoke-Nodi $nodi 'hidden 0 3'
    Stop-Nodi $nodi
    Assert ($para -match 'text="## Judul"') "heading text lost after restart: $para"
    Assert ($markdown.Trim() -like "## Judul`n`nisi*") "heading markdown wrong:`n$markdown"
    Assert ($inside -eq 'false') 'marker did not become visible when re-entering the heading'
    ''
}

Test-Case 'X' 'separator: --- + Enter, Insert menu, restart and markdown' {
    $data = New-DataDir 'X'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'keys atas' | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'keys ---' | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    $pictures = Invoke-Nodi $nodi 'pictures'
    Assert ($pictures -eq '1') "separator picture missing: $pictures"
    Invoke-Nodi $nodi 'keys bawah' | Out-Null
    $markdown = (Invoke-Nodi $nodi 'markdown') -replace "`0", ''
    Assert ($markdown -match '(?s)atas.*---.*bawah') "separator markdown wrong:`n$markdown"
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi

    $nodi = Start-Nodi $data
    $pictures = Invoke-Nodi $nodi 'pictures'
    $markdown = (Invoke-Nodi $nodi 'markdown') -replace "`0", ''
    Assert ($pictures -eq '1') "separator lost after restart: $pictures"
    Assert ($markdown -match '(?s)atas.*---.*bawah') "separator markdown lost after restart:`n$markdown"
    Invoke-Nodi $nodi 'cmd 207' | Out-Null                # Insert > Separator
    Assert ((Invoke-Nodi $nodi 'pictures') -eq '2') "Insert > Separator did nothing: $(Invoke-Nodi $nodi 'pictures')"
    Stop-Nodi $nodi
    ''
}

# --------------------------------------------------------------- batch 3 (Y)
# Real keyboard reproduction of the batch-2 bugs: heading colour, Tab inside a
# list item and the `---` separator width/indent.
Test-Case 'Y' 'real keys: heading colour, list Tab nesting, separator width' {
    $data = New-DataDir 'Y'
    $nodi = Start-Nodi $data
    if (-not [NodiHook]::Foreground($nodi.Hwnd)) {
        Stop-Nodi $nodi
        return 'NOT TESTED: nodi.exe could not be brought to the foreground'
    }
    Add-Type -AssemblyName System.Windows.Forms
    Start-Sleep -Milliseconds 250
    [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
    Start-Sleep -Milliseconds 100
    [System.Windows.Forms.SendKeys]::SendWait('## Bagian satu{ENTER}')
    Start-Sleep -Milliseconds 150
    [System.Windows.Forms.SendKeys]::SendWait('- long text yang cukup panjang untuk membungkus{ENTER}')
    Start-Sleep -Milliseconds 150
    [System.Windows.Forms.SendKeys]::SendWait('{TAB}sub poin{ENTER}')
    Start-Sleep -Milliseconds 150
    [System.Windows.Forms.SendKeys]::SendWait('+{TAB}poin kedua{ENTER}{ENTER}')
    Start-Sleep -Milliseconds 150
    [System.Windows.Forms.SendKeys]::SendWait('---{ENTER}Teks')
    Start-Sleep -Milliseconds 400

    $text = (Invoke-Nodi $nodi 'text') -replace "`r", '|'
    # The heading body (after `## `) must use the normal text colour.
    $colour = Invoke-Nodi $nodi 'fmt 3 9'
    # The sub-item: a level-2 bullet (◦) with the bullet kept.
    $at = Invoke-Nodi $nodi 'findtext sub poin'
    $sub = Invoke-Nodi $nodi 'para'
    Assert ($at -ne 'not found') "sub item missing: '$text'"
    Assert ($colour -match 'colour=(auto|#000000)') "heading text is not black: $colour"
    Assert ($sub -match 'numbering=0' -and $sub -match 'level=2' -and $sub -match 'start=360') "Tab did not nest the list item: $sub (text='$text')"
    Invoke-Nodi $nodi 'findtext poin kedua' | Out-Null
    $back = Invoke-Nodi $nodi 'para'
    Assert ($back -match 'numbering=1' -and $back -match 'level=1' -and $back -match 'start=0' -and $back -match 'text="poin kedua"') "Shift+Tab did not return to the level-1 bullet: $back"
    # The separator must span the editor and be followed by an unindented
    # paragraph.
    Assert ((Invoke-Nodi $nodi 'pictures') -eq '1') "separator picture missing: '$text'"
    $goal = Invoke-Nodi $nodi 'picgoal'
    $client = Invoke-Nodi $nodi 'client'
    $goalWidth = [int](($goal -split ' ')[0])
    $clientWidth = [int]([regex]::Match($client, 'twips=(-?\d+)').Groups[1].Value)
    Assert ($goalWidth -ge $clientWidth * 0.9) "separator is $goalWidth twips wide, editor $clientWidth"
    $at = Invoke-Nodi $nodi 'findtext Teks'
    $after = Invoke-Nodi $nodi 'para'
    Assert ($after -match 'start=0' -and $after -match 'numbering=0' -and $after -match 'text="Teks"') "paragraph after the separator is not normal: $after"
    Stop-Nodi $nodi
    "(heading $colour; sub $sub; separator $goalWidth/$clientWidth twips)"
}

# --------------------------------------------------------------- batch 3 (Z, AA)
Test-Case 'Z' 'paste Markdown keeps headings, lists, code, table and links' {
    $data = New-DataDir 'Z'
    $nodi = Start-Nodi $data
    $markdown = @'
# Judul

Teks **tebal** dan `kode`.

- satu
  - anak

```rust
fn main() {}
```

| Nama | Nilai |
| --- | --- |
| A | 92% |

[Lihat](https://example.com)
'@
    Set-Clipboard -Value $markdown
    Invoke-Nodi $nodi 'cmd 114' | Out-Null          # Edit > Paste
    Assert ((Wait-State $nodi 'Saved' 4000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    $out = Invoke-Nodi $nodi 'markdown'
    Stop-Nodi $nodi
    Assert ($out.Contains('# Judul')) "heading lost:`n$out"
    Assert ($out.Contains('**tebal**')) "bold lost:`n$out"
    Assert ($out.Contains('`kode`')) "inline code lost:`n$out"
    Assert ($out.Contains('- satu')) "list lost:`n$out"
    Assert ($out.Contains('  - anak')) "nested list lost:`n$out"
    Assert ($out.Contains('fn main() {}')) "code block lost:`n$out"
    Assert ($out.Contains('```rust')) "fence language lost:`n$out"
    Assert ($out.Contains('| Nama | Nilai |') -and $out.Contains('| A | 92% |')) "table lost:`n$out"
    Assert ($out.Contains('[Lihat](https://example.com)')) "link lost:`n$out"
    ''
}

Test-Case 'AA' 'paste HTML (CF_HTML) keeps structure' {
    $data = New-DataDir 'AA'
    $nodi = Start-Nodi $data
    $fragment = '<h2>Bagian</h2><p>Teks <strong>tebal</strong> dan <a href="https://example.com">tautan</a>.</p><ul><li>satu</li><li>dua</li></ul><pre><code class="language-rust">fn main() {}</code></pre>'
    Set-ClipboardHtml $fragment
    Invoke-Nodi $nodi 'cmd 114' | Out-Null
    Assert ((Wait-State $nodi 'Saved' 4000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    $out = Invoke-Nodi $nodi 'markdown'
    Stop-Nodi $nodi
    Assert ($out.Contains('## Bagian')) "heading lost:`n$out"
    Assert ($out.Contains('**tebal**')) "bold lost:`n$out"
    Assert ($out.Contains('[tautan](https://example.com)')) "link lost:`n$out"
    Assert ($out.Contains('- satu')) "list lost:`n$out"
    Assert ($out.Contains('fn main() {}')) "code lost:`n$out"
    ''
}

Test-Case 'AB' 'code block Copy link copies the code as plain text' {
    $data = New-DataDir 'AB'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'cmd 205' | Out-Null            # code block
    Invoke-Nodi $nodi 'type fn main() {}' | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'type x();' | Out-Null
    $target = Invoke-Nodi $nodi 'findtext Copy'
    Assert ($target -ne 'not found') "Copy link missing: '$target'"
    Invoke-Nodi $nodi "select $target $([int]$target + 4)" | Out-Null
    $link = Invoke-Nodi $nodi 'link-target'
    Assert ($link -eq 'nodi-copy:') "Copy link target is '$link'"
    $at = Invoke-Nodi $nodi 'findtext Copy'
    $point = (Invoke-Nodi $nodi "pos $at") -split ' '
    $x = [int]$point[0] + 4
    $y = [int]$point[1] + 6
    Set-Clipboard -Value 'unchanged'
    Invoke-Nodi $nodi "drag $x $y $x $y" | Out-Null
    Start-Sleep -Milliseconds 400
    $copied = ((Get-Clipboard) -join "`n") -replace "`r`n", "`n"
    Stop-Nodi $nodi
    Assert ($copied -eq "fn main() {}`nx();") "clipboard holds '$copied'"
    ''
}

Test-Case 'AC' 'output block inserts, persists and exports ```output' {
    $data = New-DataDir 'AC'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'cmd 208' | Out-Null            # output block
    Invoke-Nodi $nodi 'type loss: 0.1' | Out-Null
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    $out = Invoke-Nodi $nodi 'markdown'
    Stop-Nodi $nodi
    Assert ($out.Contains('```output')) "output fence lost:`n$out"
    Assert ($out.Contains('loss: 0.1')) "output text lost:`n$out"
    ''
}

# --------------------------------------------------------------- batch 3 (AD)
Test-Case 'AD' 'image crop survives save and restart' {
    . "$PSScriptRoot\probe-lib.ps1"
    $png = New-TestPng (Join-Path $Root 'e2e-crop.png') 200 100
    $data = New-DataDir 'AD'
    $nodi = Start-Nodi $data
    Assert ((Invoke-Nodi $nodi "insert-image $png") -eq 'ok') 'insert failed'
    Invoke-Nodi $nodi 'select 0 0' | Out-Null
    $goalBefore = Invoke-Nodi $nodi 'picgoal'
    $applied = Invoke-Nodi $nodi 'crop 10 5 20 15'
    Assert ($applied -eq 'ok') "crop failed: $applied"
    $crop = Invoke-Nodi $nodi 'piccrop'
    Assert ($crop -eq '150 75 300 225') "crop in RTF is '$crop' twips"
    $goalAfter = Invoke-Nodi $nodi 'picgoal'
    Assert ($goalAfter -eq $goalBefore) "crop changed the display size: $goalBefore -> $goalAfter"
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'select 0 0' | Out-Null
    $crop = Invoke-Nodi $nodi 'piccrop'
    $pictures = Invoke-Nodi $nodi 'pictures'
    Stop-Nodi $nodi
    Assert ($pictures -eq '1') "picture lost after restart: $pictures"
    Assert ($crop -eq '150 75 300 225') "crop lost after restart: '$crop'"
    ''
}

# --------------------------------------------------------------- batch 3 (AE)
Test-Case 'AE' 'typed and pasted local paths become links' {
    $data = New-DataDir 'AE'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'cmd 170' | Out-Null
    # Typed: the trailing space completes the path and conversion.
    Invoke-Nodi $nodi 'keys Buka C:\\Windows ' | Out-Null
    $at = Invoke-Nodi $nodi 'findtext C:\Windows'
    Assert ($at -ne 'not found') "typed path not found: '$at'"
    Invoke-Nodi $nodi "select $at $([int]$at + 10)" | Out-Null
    $target = Invoke-Nodi $nodi 'link-target'
    Assert ($target -eq 'C:\Windows') "typed path target is '$target'"
    # Collapse the selection (Right arrow) before typing on: Enter must land
    # after the link, not replace it.
    Invoke-Nodi $nodi 'raw-key both 39' | Out-Null
    # A quoted path with spaces is completed by the closing quote.
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'keys "C:\\Program Files" ' | Out-Null
    $at = Invoke-Nodi $nodi 'findtext C:\Program Files'
    Assert ($at -ne 'not found') "quoted path not found: '$at'; text='$((Invoke-Nodi $nodi 'text') -replace "`r", '|')'"
    Invoke-Nodi $nodi "select $at $([int]$at + 16)" | Out-Null
    $target = Invoke-Nodi $nodi 'link-target'
    Assert ($target -eq 'C:\Program Files') "quoted path target is '$target'; text='$((Invoke-Nodi $nodi 'text') -replace "`r", '|')'"
    Invoke-Nodi $nodi 'raw-key both 39' | Out-Null
    # Pasting one path becomes a link too.
    Invoke-Nodi $nodi 'enter' | Out-Null
    Set-Clipboard -Value 'D:\Data\notes'
    Invoke-Nodi $nodi 'cmd 114' | Out-Null
    $at = Invoke-Nodi $nodi 'findtext D:\Data\notes'
    Assert ($at -ne 'not found') "pasted path not found: '$at'"
    Invoke-Nodi $nodi "select $at $([int]$at + 13)" | Out-Null
    $target = Invoke-Nodi $nodi 'link-target'
    Assert ($target -eq 'D:\Data\notes') "pasted path target is '$target'"
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    $out = Invoke-Nodi $nodi 'markdown'
    Stop-Nodi $nodi
    Assert ($out.Contains('[C:\\Windows](C:\Windows)')) "typed link lost:`n$out"
    Assert ($out.Contains('[C:\\Program Files](C:\Program%20Files)')) "quoted link lost:`n$out"
    Assert ($out.Contains('[D:\\Data\\notes](D:\Data\notes)')) "pasted link lost:`n$out"
    ''
}

# --------------------------------------------------------------- batch 3 (AF)
Test-Case 'AF' 'tree drag and drop before / after / inside' {
    $data = New-DataDir 'AF'
    $nodi = Start-Nodi $data
    # The first run opens with one "Untitled" note: reuse it as Alpha.
    Invoke-Nodi $nodi 'rename-selected Alpha' | Out-Null
    foreach ($title in @('Beta', 'Gamma')) {
        Invoke-Nodi $nodi 'cmd 170' | Out-Null        # New Note
        Invoke-Nodi $nodi "rename-selected $title" | Out-Null
    }
    Invoke-Nodi $nodi 'cmd 174' | Out-Null            # New Folder
    Invoke-Nodi $nodi "rename-selected Box" | Out-Null
    Invoke-Nodi $nodi 'cancel-rename' | Out-Null

    # before: Gamma becomes the first root sibling.
    Assert ((Invoke-Nodi $nodi 'move-rel Gamma before Alpha') -eq 'ok') 'move before failed'
    $nodes = Wait-Tree $data { param($n) (Root-Titles $n) -eq 'Gamma,Alpha,Beta,Box' }
    Assert ((Root-Titles $nodes) -eq 'Gamma,Alpha,Beta,Box') "before order is $(Root-Titles $nodes)"
    Assert (@($nodes | Where-Object title -eq 'Gamma')[0].order -eq 0) 'Gamma is not first'

    # after: Beta goes to the very end of the root.
    Assert ((Invoke-Nodi $nodi 'move-rel Beta after Box') -eq 'ok') 'move after failed'
    $nodes = Wait-Tree $data { param($n) (Root-Titles $n) -eq 'Gamma,Alpha,Box,Beta' }
    Assert ((Root-Titles $nodes) -eq 'Gamma,Alpha,Box,Beta') "after order is $(Root-Titles $nodes)"

    # inside: Alpha becomes the last child of the Box folder.
    Assert ((Invoke-Nodi $nodi 'move-rel Alpha inside Box') -eq 'ok') 'move inside failed'
    $nodes = Wait-Tree $data {
        param($n)
        @($n | Where-Object title -eq 'Alpha')[0].parent -eq @($n | Where-Object title -eq 'Box')[0].id
    }
    $alpha = @($nodes | Where-Object title -eq 'Alpha')[0]
    $box = @($nodes | Where-Object title -eq 'Box')[0]
    Assert ($alpha.parent -eq $box.id) 'Alpha is not inside Box'

    # inside a note too, then a cycle is refused without changing anything.
    Assert ((Invoke-Nodi $nodi 'move-rel Beta inside Alpha') -eq 'ok') 'move into note failed'
    $nodes = Wait-Tree $data {
        param($n)
        @($n | Where-Object title -eq 'Beta')[0].parent -eq @($n | Where-Object title -eq 'Alpha')[0].id
    }
    $cycle = Invoke-Nodi $nodi 'move-rel Box inside Alpha'
    Assert ($cycle -like 'error*') "cycle was not refused: '$cycle'"
    $nodes = (Read-Tree $data).nodes
    $beta = @($nodes | Where-Object title -eq 'Beta')[0]
    $alpha = @($nodes | Where-Object title -eq 'Alpha')[0]
    $box = @($nodes | Where-Object title -eq 'Box')[0]
    Assert ($beta.parent -eq $alpha.id) 'Beta is not inside Alpha'
    Assert ($alpha.parent -eq $box.id) 'the refused move changed the tree'

    # Move Up / Down / Top Level still work.
    Invoke-Nodi $nodi 'select-title Gamma' | Out-Null
    Invoke-Nodi $nodi 'cmd 182' | Out-Null            # Move Down
    $nodes = Wait-Tree $data { param($n) (Root-Titles $n) -eq 'Box,Gamma' }
    Assert ((Root-Titles $nodes) -eq 'Box,Gamma') "move down order is $(Root-Titles $nodes)"
    Invoke-Nodi $nodi 'select-title Alpha' | Out-Null
    Invoke-Nodi $nodi 'cmd 184' | Out-Null            # Move to Top Level
    $nodes = Wait-Tree $data { param($n) (Root-Titles $n) -eq 'Box,Gamma,Alpha' }
    $alpha = @($nodes | Where-Object title -eq 'Alpha')[0]
    $root = Root-Titles $nodes
    Assert ($null -eq $alpha.parent) 'Move to Top Level did not work'
    Assert ($root -eq 'Box,Gamma,Alpha') "top level order is $root"

    # Everything survives a restart.
    Stop-Nodi $nodi
    $nodi = Start-Nodi $data
    Stop-Nodi $nodi
    $nodes = (Read-Tree $data).nodes
    $beta = @($nodes | Where-Object title -eq 'Beta')[0]
    $alpha = @($nodes | Where-Object title -eq 'Alpha')[0]
    Assert ($beta.parent -eq $alpha.id) 'Beta/Alpha link lost after restart'
    Assert (@($nodes | Where-Object title -eq 'Gamma')[0].order -eq 1) 'Gamma order changed'
    ''
}
# --------------------------------------------------------------- batch 4 (AG-AL)
Test-Case 'AG' 'dark theme: persists, editor is dark, stored colours stay automatic' {
    $data = New-DataDir 'AG'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'cmd 301' | Out-Null            # View > Theme > Dark
    Invoke-Nodi $nodi 'type Teks gelap' | Out-Null
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    $live = Invoke-Nodi $nodi 'fmt 0 4'
    Stop-Nodi $nodi
    $settings = Get-Content -Raw (Join-Path $data 'settings.json') | ConvertFrom-Json
    Assert ($settings.theme -eq 'dark') "theme not persisted: $($settings.theme)"

    $nodi = Start-Nodi $data
    $theme = Invoke-Nodi $nodi 'theme'
    Assert ($theme -like 'dark dark=1 bg=#1e1e1e*') "theme after restart: $theme"
    # The editor really paints dark: sample the empty area below the text.
    . (Join-Path $PSScriptRoot 'probe-lib.ps1')
    $shot = Join-Path $data 'shot.png'
    Save-Shot $nodi $shot | Out-Null
    $rect = (Invoke-Nodi $nodi 'editorrect') -split '[x+]'
    $window = New-Object NodiShot+RECT
    [NodiShot]::GetWindowRect($nodi.Hwnd, [ref]$window) | Out-Null
    $bitmap = [Drawing.Bitmap]::FromFile($shot)
    $x = [int]$rect[2] - $window.L + [int]$rect[0] / 2
    $y = [int]$rect[3] - $window.T + [int]$rect[1] - 20
    $pixel = $bitmap.GetPixel($x, $y)
    $bitmap.Dispose()
    Assert ($pixel.R -lt 60 -and $pixel.G -lt 60 -and $pixel.B -lt 60) "editor is not dark at ($x,$y): $pixel"

    # Back to light: the text is automatic (black), not the dark display colour.
    Invoke-Nodi $nodi 'cmd 300' | Out-Null
    $light = Invoke-Nodi $nodi 'fmt 0 4'
    Stop-Nodi $nodi
    Assert ($live -match 'colour=auto') "dark-theme text does not read as automatic: $live"
    Assert ($light -match 'colour=auto') "text is not automatic after switching back: $light"
    $rtf = Get-ChildItem (Join-Path $data 'notes') -Filter *.rtf | Select-Object -First 1
    $markdownView = [IO.File]::ReadAllText($rtf.FullName)
    Assert ($markdownView -notmatch '\\cf[1-9][0-9]* ?Teks gelap') 'stored text carries a display colour'
    "(pixel $($pixel.R),$($pixel.G),$($pixel.B))"
}

Test-Case 'AH' 'Markdown mirror follows create, rename and move' {
    $data = New-DataDir 'AH'
    $mirror = Join-Path $data 'markdown'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'rename-selected Rapat' | Out-Null
    Invoke-Nodi $nodi 'type Agenda minggu ini' | Out-Null
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    $file = Join-Path $mirror 'Rapat.md'
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while (-not ((Test-Path $file) -and (Get-Content -Raw $file).Contains('Agenda minggu ini')) -and $watch.ElapsedMilliseconds -lt 4000) {
        Start-Sleep -Milliseconds 100
    }
    Assert (Test-Path $file) 'Rapat.md was not written'
    $body = Get-Content -Raw $file
    Assert ($body.StartsWith('---')) "no front matter:`n$body"
    Assert ($body.Contains('Agenda minggu ini')) "text missing:`n$body"

    Invoke-Nodi $nodi 'rename-selected Rapat Besar' | Out-Null
    Invoke-Nodi $nodi 'cmd 174' | Out-Null            # New Folder
    Invoke-Nodi $nodi 'rename-selected Kantor' | Out-Null
    Invoke-Nodi $nodi 'cancel-rename' | Out-Null
    Assert ((Invoke-Nodi $nodi 'move-rel Rapat Besar inside Kantor') -eq 'ok') 'move failed'
    $moved = Join-Path $mirror 'Kantor\Rapat Besar.md'
    $watch.Restart()
    while (-not (Test-Path $moved) -and $watch.ElapsedMilliseconds -lt 4000) { Start-Sleep -Milliseconds 100 }
    Stop-Nodi $nodi
    Assert (Test-Path $moved) 'the mirror did not follow the rename + move'
    Assert ((Get-Content -Raw $moved).Contains('Agenda minggu ini')) 'moved mirror lost its text'
    Assert (-not (Test-Path $file)) 'the old mirror file is still there'
    $kept = @(Get-ChildItem -Recurse (Join-Path $mirror '.trash') -Filter 'Rapat.md').Count
    Assert ($kept -ge 1) 'the old mirror file was not kept in .trash'
    $index = Get-Content -Raw (Join-Path $mirror 'INDEX.md')
    Assert ($index.Contains('Rapat Besar')) "INDEX.md:`n$index"
    $json = Get-Content -Raw (Join-Path $mirror 'index.json') | ConvertFrom-Json
    Assert ($json.notes.Count -ge 1 -or $json.Count -ge 1) 'index.json is empty'
    ''
}

Test-Case 'AI' 'status bar counts words and characters' {
    $data = New-DataDir 'AI'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'type satu dua tiga' | Out-Null
    $counts = Invoke-Nodi $nodi 'counts'
    Invoke-Nodi $nodi 'select 0 8' | Out-Null
    $selected = Invoke-Nodi $nodi 'counts'
    Stop-Nodi $nodi
    Assert ($counts -match '\b3 words') "note counts: $counts"
    Assert ($counts -match '\b13 ch') "note chars: $counts"
    Assert ($selected -match '\b2 words') "selection counts: $selected"
    "($counts / $selected)"
}

Test-Case 'AJ' 'tab menu: Close Others keeps one tab' {
    $data = New-DataDir 'AJ'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'cmd 170' | Out-Null
    Invoke-Nodi $nodi 'cmd 170' | Out-Null
    $before = Invoke-Nodi $nodi 'tabcount'
    $after = Invoke-Nodi $nodi 'close-others 1'
    $tree = @((Read-Tree $data).nodes).Count
    Stop-Nodi $nodi
    Assert ($before -eq '3') "expected 3 tabs, got $before"
    Assert ($after -eq '1') "Close Others left $after tabs"
    Assert ($tree -eq 3) 'closing tabs must not delete notes'
    ''
}

Test-Case 'AK' 'restore backup round trip' {
    $data = New-DataDir 'AK'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'rename-selected Sebelum' | Out-Null
    Assert ((Wait-State $nodi 'Saved' 3000) -ge 0) 'never Saved'
    $backups = Join-Path $data 'backups'
    $count = @(Get-ChildItem $backups -Directory -ErrorAction SilentlyContinue).Count
    Invoke-Nodi $nodi 'cmd 190' | Out-Null            # Tools > Backup Now
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while (@(Get-ChildItem $backups -Directory -ErrorAction SilentlyContinue).Count -le $count -and $watch.ElapsedMilliseconds -lt 5000) {
        Start-Sleep -Milliseconds 100
    }
    Start-Sleep -Milliseconds 300
    Invoke-Nodi $nodi 'rename-selected Sesudah' | Out-Null
    $restored = Invoke-Nodi $nodi 'restore-latest'
    $title = (Invoke-Nodi $nodi 'tabs') -split "`n" | Select-Object -Last 1
    Stop-Nodi $nodi
    $nodes = (Read-Tree $data).nodes
    Assert (@($nodes | Where-Object title -eq 'Sebelum').Count -eq 1) "restore did not bring 'Sebelum' back ($restored): $(($nodes | ForEach-Object title) -join ',')"
    $snapshots = @(Get-ChildItem $backups -Directory).Count
    Assert ($snapshots -ge $count + 2) "restore must snapshot the current state first ($snapshots snapshots)"
    "(restored $restored, tab '$title')"
}

Test-Case 'AL' 'recent notes and Ctrl+Alt+O with real keys' {
    $data = New-DataDir 'AL'
    $nodi = Start-Nodi $data
    Invoke-Nodi $nodi 'rename-selected Pertama' | Out-Null
    Invoke-Nodi $nodi 'cmd 170' | Out-Null
    Invoke-Nodi $nodi 'rename-selected Kedua' | Out-Null
    Invoke-Nodi $nodi 'select-title Pertama' | Out-Null
    $recent = Invoke-Nodi $nodi 'recent'
    Assert ($recent -match 'Pertama' -and $recent -match 'Kedua') "recent list: $recent"
    if (-not [NodiHook]::Foreground($nodi.Hwnd)) {
        Stop-Nodi $nodi
        return "NOT TESTED (real keys): foreground; recent '$recent'"
    }
    Add-Type -AssemblyName System.Windows.Forms
    Start-Sleep -Milliseconds 250
    [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
    Start-Sleep -Milliseconds 100
    [System.Windows.Forms.SendKeys]::SendWait('^%o')
    Start-Sleep -Milliseconds 300
    $last = Invoke-Nodi $nodi 'lastcmd'
    [System.Windows.Forms.SendKeys]::SendWait('{ESC}')
    Stop-Nodi $nodi
    $session = Get-Content -Raw (Join-Path $data 'session.json')
    Assert ($last -match '142') "Ctrl+Alt+O did not reach View > Outline: $last"
    Assert ($session -match 'recent') 'recent notes are not stored in the session'
    "(recent $recent)"
}

# --------------------------------------------------------------------- report

$failed = 0
foreach ($entry in $results.GetEnumerator()) {
    "{0,-60} {1}" -f $entry.Key, $entry.Value
    if ($entry.Value -like 'FAIL*') { $failed++ }
}
"--- $($results.Count - $failed) passed, $failed failed"
# Dot-sourcing (probe scripts) must not exit the caller.
if ($MyInvocation.InvocationName -ne '.') { exit $failed }
