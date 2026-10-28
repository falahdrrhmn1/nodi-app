# Save-reliability torture test - docs/tasks/rev2-batch5.md section 12.
#
# One fresh data folder per run, kept across all cycles of that run. Every cycle:
#   1. start Nodi and verify that the *previous* cycle's state came back exactly
#      (per note: `text` hook output + `markdown` hook output; the tree's titles
#      and parents from tree.json),
#   2. make varied edits through the hooks (typing, Enter, Tab, headings, lists,
#      tables, code blocks, separators, formulas, renames, a new note, moves),
#   3. record the expected state from the hooks *before* closing,
#   4. close in a rotating way:
#        1  WM_CLOSE immediately after the last edit (no wait for `Saved`)
#        2  Ctrl+S (File > Save, id 101) and then WM_CLOSE
#        3  WM_QUERYENDSESSION + WM_ENDSESSION(TRUE) - a Windows shutdown
#        4  wait for `Saved`, then Stop-Process -Force
# Any difference is a mismatch: a short diff is printed and the run continues.
#
# Usage: pwsh -File scripts\persist-torture.ps1 [-Cycles 20] [-Runs 1]
#
# Exit code = number of mismatches.

param(
    [string]$Exe = "$PSScriptRoot\..\target\debug\nodi.exe",
    [int]$Cycles = 20,
    [int]$Runs = 1
)

$ErrorActionPreference = 'Stop'

# The e2e helpers (Start-Nodi, Invoke-Nodi, Wait-State, Stop-Nodi, Read-Tree).
# `-Only none` runs no test case, exactly like scripts\probe-lib.ps1 uses it.
. "$PSScriptRoot\e2e.ps1" -Exe $Exe -Only none | Out-Null

$WM_QUERYENDSESSION = 0x0011
$WM_ENDSESSION = 0x0016
$IDM_FILE_SAVE = 101
$IDM_NOTES_OPEN = 171
$IDM_NOTES_NEW = 170
$IDM_FORMAT_H1 = 210
$IDM_FORMAT_BULLETS = 214
$IDM_INSERT_CODE_BLOCK = 205
$IDM_INSERT_SEPARATOR = 207
$IDM_TABLE_COL_RIGHT = 233

# Trimmed one-line excerpt for the diff output.
function Short-Text([string]$text, [int]$max = 120) {
    $flat = $text -replace "`r", '\r' -replace "`n", '\n'
    if ($flat.Length -le $max) { return "'$flat'" }
    return "'$($flat.Substring(0, $max))...' (len $($text.Length))"
}

# First difference between two multi-line strings, as one short line.
function Format-Diff([string]$label, [string]$want, [string]$got) {
    $w = @($want -split "`r`n|`r|`n")
    $g = @($got -split "`r`n|`r|`n")
    $shared = [Math]::Min($w.Count, $g.Count)
    for ($i = 0; $i -lt $shared; $i++) {
        if ($w[$i] -cne $g[$i]) {
            return "$label line $($i + 1): expected $(Short-Text $w[$i]), got $(Short-Text $g[$i])"
        }
    }
    if ($w.Count -ne $g.Count) {
        $extra = if ($w.Count -gt $g.Count) { "missing line $(Short-Text $w[$shared])" } else { "extra line $(Short-Text $g[$shared])" }
        return "${label}: expected $($w.Count) lines, got $($g.Count) ($extra)"
    }
    return "${label}: same lines, different bytes (expected len $($want.Length), got $($got.Length))"
}

# Caret to the very end of the active note; returns the text length.
function Select-End($nodi) {
    $raw = Invoke-Nodi $nodi 'sel'
    $match = [regex]::Match($raw, 'len=(\d+)')
    if (-not $match.Success) { throw "unexpected 'sel' reply: $raw" }
    $length = [int]$match.Groups[1].Value
    Invoke-Nodi $nodi "select $length $length" | Out-Null
    return $length
}

# Selects a note in the tree and opens its tab (Notes > Open).
function Open-Note($nodi, [string]$title) {
    $found = Invoke-Nodi $nodi "select-title $title"
    if ($found -ne 'ok') { throw "note '$title' not found: $found" }
    Invoke-Nodi $nodi "cmd $IDM_NOTES_OPEN" | Out-Null
}

# One cycle of varied edits. A long-lived note ("Torture") collects a log line
# per cycle; every cycle also creates a new note ("C<n>", renamed to "C<n>-x")
# with a rotating structure, a formula, and is nested under "Torture" (the
# previous cycle's note is moved back to the top level).
function Invoke-CycleEdits($nodi, [int]$cycle) {
    if ($cycle -eq 1) {
        # The first run opens one "Untitled" note; make it the long-lived one.
        $renamed = Invoke-Nodi $nodi 'rename-selected Torture'
        if ($renamed -ne 'ok') { throw "could not rename the first note: $renamed" }
    }

    Open-Note $nodi 'Torture'
    Select-End $nodi | Out-Null
    Invoke-Nodi $nodi "type cycle $cycle" | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi 'tab' | Out-Null                 # paragraph indent
    Invoke-Nodi $nodi "type indent $cycle" | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null
    Invoke-Nodi $nodi "type line $($cycle) of the log" | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null

    # A fresh note with a structure that rotates over the cycles.
    Invoke-Nodi $nodi "cmd $IDM_NOTES_NEW" | Out-Null
    Invoke-Nodi $nodi "rename-selected C$cycle" | Out-Null
    switch (($cycle - 1) % 5) {
        0 {
            Invoke-Nodi $nodi "cmd $IDM_FORMAT_H1" | Out-Null
            Invoke-Nodi $nodi "type Heading $cycle" | Out-Null
            Invoke-Nodi $nodi 'enter' | Out-Null
            Invoke-Nodi $nodi "type body $cycle" | Out-Null
            Invoke-Nodi $nodi 'enter' | Out-Null
        }
        1 {
            Invoke-Nodi $nodi "cmd $IDM_FORMAT_BULLETS" | Out-Null
            Invoke-Nodi $nodi "type satu $cycle" | Out-Null
            Invoke-Nodi $nodi 'enter' | Out-Null
            Invoke-Nodi $nodi "type dua $cycle" | Out-Null
            Invoke-Nodi $nodi 'tab' | Out-Null         # nest the list item
            Invoke-Nodi $nodi 'enter' | Out-Null
            Invoke-Nodi $nodi "type tiga $cycle" | Out-Null
            Invoke-Nodi $nodi 'enter' | Out-Null
            Invoke-Nodi $nodi 'enter' | Out-Null       # empty item ends the list
            Invoke-Nodi $nodi "type after $cycle" | Out-Null
            Invoke-Nodi $nodi 'enter' | Out-Null
        }
        2 {
            Invoke-Nodi $nodi "cmd $IDM_INSERT_CODE_BLOCK" | Out-Null
            Invoke-Nodi $nodi "type fn f$cycle() {" | Out-Null
            Invoke-Nodi $nodi 'enter' | Out-Null
            Invoke-Nodi $nodi 'tab' | Out-Null
            Invoke-Nodi $nodi "type x$cycle();" | Out-Null
            Invoke-Nodi $nodi 'enter' | Out-Null
            Invoke-Nodi $nodi 'type }' | Out-Null
        }
        3 {
            Invoke-Nodi $nodi 'table 2 2' | Out-Null
            Invoke-Nodi $nodi "type cell $cycle" | Out-Null
            Invoke-Nodi $nodi "cmd $IDM_TABLE_COL_RIGHT" | Out-Null
            Invoke-Nodi $nodi "type more $cycle" | Out-Null
        }
        4 {
            Invoke-Nodi $nodi "type atas $cycle" | Out-Null
            Invoke-Nodi $nodi 'enter' | Out-Null
            Invoke-Nodi $nodi "cmd $IDM_INSERT_SEPARATOR" | Out-Null
            Select-End $nodi | Out-Null
            Invoke-Nodi $nodi "type bawah $cycle" | Out-Null
            Invoke-Nodi $nodi 'enter' | Out-Null
        }
    }

    # A formula, then a plain line after it.
    Select-End $nodi | Out-Null
    $latex = "\frac{$cycle}{b} + \sqrt{x}"
    $rendered = Invoke-Nodi $nodi "formula $latex"
    if ($rendered -ne 'ok') { throw "formula failed: $rendered" }
    Invoke-Nodi $nodi "type tail $cycle" | Out-Null
    Invoke-Nodi $nodi 'enter' | Out-Null

    # Rename the new note a second time and nest it under the log note.
    if ((Invoke-Nodi $nodi "select-title C$cycle") -ne 'ok') { throw "note C$cycle is missing" }
    Invoke-Nodi $nodi "rename-selected C$cycle-x" | Out-Null
    $nested = Invoke-Nodi $nodi "move-rel C$cycle-x inside Torture"
    if ($nested -ne 'ok') { throw "move-rel inside: $nested" }
    if ($cycle -gt 1) {
        $back = Invoke-Nodi $nodi "move-rel C$($cycle - 1)-x after Torture"
        if ($back -ne 'ok') { throw "move-rel back to the top level: $back" }
    }
    # The new note stays the active tab: the caller records its state first and
    # edits the log note afterwards, so mode 1 closes with a dirty buffer.
}

# The last edit of the cycle: one more line at the end of the log note. It runs
# after the other notes were recorded, so mode 1 (close right away) really
# closes a modified buffer.
function Invoke-LastEdit($nodi, [int]$cycle) {
    Open-Note $nodi 'Torture'
    Select-End $nodi | Out-Null
    Invoke-Nodi $nodi "type end $cycle" | Out-Null
}

# `text` + `markdown` of one note, read through the hooks (the tab is opened).
function Get-NoteState($nodi, [string]$title) {
    Open-Note $nodi $title
    [pscustomobject]@{
        title = $title
        text = (Invoke-Nodi $nodi 'text')
        markdown = (Invoke-Nodi $nodi 'markdown')
    }
}

# Titles + parents of every live node, as sorted "title <- parent" lines.
function Get-TreeState([string]$data) {
    $nodes = @((Read-Tree $data).nodes)
    $byId = @{}
    foreach ($node in $nodes) { $byId["$($node.id)"] = $node }
    $rows = foreach ($node in $nodes) {
        if ($node.trashed) { continue }
        $parent = if ($null -eq $node.parent) { '<root>' } else { "$($byId["$($node.parent)"].title)" }
        "$($node.title) <- $parent"
    }
    return @($rows | Sort-Object)
}

# Waits until tree.json on disk shows what this cycle's moves should have done:
# the new note nested under "Torture" and, from cycle 2 on, the previous cycle's
# note back at the top level. tree.json is written by a background thread, so
# this also makes sure the state captured next is the state Nodi really wrote.
function Wait-TreeWritten([string]$data, [string]$title, [string]$previous, [int]$timeoutMs = 5000) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.ElapsedMilliseconds -lt $timeoutMs) {
        try {
            $nodes = @((Read-Tree $data).nodes)
            $child = $nodes | Where-Object title -eq $title
            $parent = $nodes | Where-Object title -eq 'Torture'
            $ok = $child -and $parent -and $child.parent -eq $parent.id
            if ($ok -and $previous) {
                $older = $nodes | Where-Object title -eq $previous
                $ok = $older -and ($null -eq $older.parent)
            }
            if ($ok) { return $true }
        } catch {
            # tree.json is being replaced; try again.
        }
        Start-Sleep -Milliseconds 50
    }
    return $false
}

# Every mismatch between `$expected` and what Nodi has now.
function Compare-State($nodi, [string]$data, $expected) {
    $problems = @()
    foreach ($note in $expected.notes) {
        $now = Get-NoteState $nodi $note.title
        if ($now.text -cne $note.text) {
            $problems += Format-Diff "note '$($note.title)' text" $note.text $now.text
        }
        if ($now.markdown -cne $note.markdown) {
            $problems += Format-Diff "note '$($note.title)' markdown" $note.markdown $now.markdown
        }
    }
    $tree = Get-TreeState $data
    $want = $expected.tree -join "`n"
    $got = $tree -join "`n"
    if ($want -cne $got) {
        $problems += Format-Diff 'tree (title <- parent)' $want $got
    }
    return $problems
}

# Waits for the process to exit; returns $true when it did.
function Wait-Exit($nodi, [int]$timeoutMs) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while ($watch.ElapsedMilliseconds -lt $timeoutMs) {
        if ($nodi.Process.HasExited) { return $true }
        Start-Sleep -Milliseconds 20
    }
    return $nodi.Process.HasExited
}

# Closes the app the way cycle `mode` says. Returns a short description of what
# happened plus any warning worth printing.
function Close-Cycle($nodi, [int]$mode) {
    switch ($mode) {
        1 {
            # WM_CLOSE straight after the last edit: no Ctrl+S, no `Saved` wait.
            $before = Invoke-Nodi $nodi 'state'
            [NodiHook]::PostMessageW($nodi.Hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
            $asked = $false
            $watch = [Diagnostics.Stopwatch]::StartNew()
            while (-not $nodi.Process.HasExited -and $watch.ElapsedMilliseconds -lt 15000) {
                if (-not $asked -and [NodiHook]::FindDialog($nodi.Process.Id) -ne 0) {
                    # Nodi only asks when its own flush did not save everything.
                    Close-Dialog $nodi 6 | Out-Null
                    $asked = $true
                }
                Start-Sleep -Milliseconds 20
            }
            if (-not $nodi.Process.HasExited) {
                $nodi.Process.Kill()
                $nodi.Process.WaitForExit(5000) | Out-Null
                return "WM_CLOSE (state '$before', did not exit -> killed)"
            }
            if ($asked) { return "WM_CLOSE (state '$before', asked 'close anyway?')" }
            return "WM_CLOSE (state '$before')"
        }
        2 {
            Invoke-Nodi $nodi "cmd $IDM_FILE_SAVE" | Out-Null        # Ctrl+S
            Start-Sleep -Milliseconds 50
            $before = Invoke-Nodi $nodi 'state'
            Stop-Nodi $nodi 6
            return "Ctrl+S then WM_CLOSE (state '$before')"
        }
        3 {
            [NodiHook]::PostMessageW($nodi.Hwnd, $WM_QUERYENDSESSION, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
            [NodiHook]::PostMessageW($nodi.Hwnd, $WM_ENDSESSION, [IntPtr]1, [IntPtr]::Zero) | Out-Null
            if (-not (Wait-Exit $nodi 3000)) {
                # A real shutdown terminates the process after WM_ENDSESSION.
                $nodi.Process.Kill()
                $nodi.Process.WaitForExit(5000) | Out-Null
                return 'WM_QUERYENDSESSION + WM_ENDSESSION (flushed, did not exit -> killed)'
            }
            return 'WM_QUERYENDSESSION + WM_ENDSESSION (exited)'
        }
        4 {
            $ms = Wait-State $nodi 'Saved' 8000
            $state = if ($ms -ge 0) { "Saved after $ms ms" } else { "never Saved: '$(Invoke-Nodi $nodi 'state')'" }
            $nodi.Process.Kill()
            $nodi.Process.WaitForExit(5000) | Out-Null
            return "waited $state, then Stop-Process -Force"
        }
    }
}

# ------------------------------------------------------------------- the run

$totalMismatches = 0
$runSummaries = @()

for ($run = 1; $run -le $Runs; $run++) {
    $data = New-DataDir "persist$run"
    "run $run/$Runs on $data"
    $notes = @()          # expected note states, carried across cycles
    $tree = @()           # expected tree, carried across cycles
    $mismatches = 0
    $nodi = $null

    for ($cycle = 1; $cycle -le $Cycles; $cycle++) {
        $mode = (($cycle - 1) % 4) + 1
        $problems = @()
        $line = "cycle $cycle/$Cycles mode $mode"
        try {
            $nodi = Start-Nodi $data

            # 1. did the previous cycle survive?
            if ($cycle -gt 1) {
                $problems += @(Compare-State $nodi $data ([pscustomobject]@{
                    notes = $notes; tree = $tree
                }))
            }

            # 2. varied edits
            Invoke-CycleEdits $nodi $cycle

            # 3. expected state, read through the hooks before closing. The new
            # note is still the active tab; the log note is edited afterwards so
            # that its buffer is dirty when mode 1 closes the app.
            $previous = if ($cycle -gt 1) { "C$($cycle - 1)-x" } else { $null }
            if (-not (Wait-TreeWritten $data "C$cycle-x" $previous)) {
                $onDisk = @((Read-Tree $data).nodes | ForEach-Object { "$($_.title)<$($_.parent)>$($_.id)" }) -join ', '
                $problems += "cycle ${cycle}: tree.json does not show C$cycle-x under Torture (on disk: $onDisk; in app: under '$(Invoke-Nodi $nodi "parent C$cycle-x")', state $(Invoke-Nodi $nodi 'state'))"
                if ($previous) { $problems += "cycle ${cycle}: tree.json does not show $previous at the top level" }
            }
            $fresh = @((Get-NoteState $nodi "C$cycle-x"))
            $tree = Get-TreeState $data
            Invoke-LastEdit $nodi $cycle
            $fresh += @(Get-NoteState $nodi 'Torture')
            $notes = @($notes | Where-Object { $fresh.title -notcontains $_.title }) + $fresh

            # 4. close in rotation
            $line += ' close: ' + (Close-Cycle $nodi $mode)
            $nodi = $null
        } catch {
            $problems += "cycle $cycle exception: $($_.Exception.Message)"
            if ($nodi) {
                Get-Process -Id $nodi.Process.Id -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
                $nodi = $null
            }
        }

        if ($problems.Count -gt 0) {
            $mismatches += $problems.Count
            $line += " MISMATCH ($($problems.Count))"
        } else {
            $line += ' ok'
        }
        $line
        foreach ($problem in $problems) { "    $problem" }
    }

    # The last cycle's state is only checked by the next start, so start once
    # more and check it too.
    $problems = @()
    try {
        $nodi = Start-Nodi $data
        $problems += @(Compare-State $nodi $data ([pscustomobject]@{ notes = $notes; tree = $tree }))
        Stop-Nodi $nodi
        $nodi = $null
    } catch {
        $problems += "final check exception: $($_.Exception.Message)"
        if ($nodi) {
            Get-Process -Id $nodi.Process.Id -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
            $nodi = $null
        }
    }
    if ($problems.Count -gt 0) {
        $mismatches += $problems.Count
        "final check after cycle $Cycles MISMATCH ($($problems.Count))"
        foreach ($problem in $problems) { "    $problem" }
    } else {
        "final check after cycle $Cycles ok"
    }

    $totalMismatches += $mismatches
    $runSummaries += "run $run : $Cycles cycles, $mismatches mismatches"
}

foreach ($summary in $runSummaries) { $summary }
"$($Cycles * $Runs) cycles, $totalMismatches mismatches"

# Dot-sourcing (tests, probes) must not exit the caller.
if ($MyInvocation.InvocationName -ne '.') { exit $totalMismatches }
