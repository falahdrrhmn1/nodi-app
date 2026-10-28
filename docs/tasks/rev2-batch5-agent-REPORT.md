# Rev2 Batch 5 — agent part (README + save-reliability harness)

Scope: only `README.md` and the new `scripts/persist-torture.ps1` (plus this
report). No Rust file was touched, no `cargo fmt`, no commit, nothing deleted.

Binary under test: `target\debug\nodi.exe`. Two builds were used while I worked:

- **2026-10-28 19:21:03** — the first runs (the iterative `-Cycles 1/2/3/4/5/6`
  runs and the two 20-cycle runs at 19:33 and 19:36);
- **2026-10-28 19:40:49** — every run quoted in section C. That build already
  contains the orchestrator's new `testhook.rs` `move-rel` (it calls
  `app.move_node_relative`, the drag & drop/undo path, instead of
  `tree.move_relative` + `tree_save` + `rebuild_tree`).

The exe's mtime was identical before and after each individual run, so the
numbers below were not taken across a rebuild. `app.rs`/`menu.rs` were edited
again at 19:46/19:47, i.e. after the 19:40:49 build, so the final binary has not
been tested — re-run `pwsh -File scripts\persist-torture.ps1` on it.

## A. README.md

Rewritten: English, title **Nodi App**, no “Mark III” in the title, headings or
intro (the path `E:\nodi-app\mark-3` stays where it is a path). Contents, in
this order and kept short:

- intro (native Win32 + Rich Edit, Notepad++ window / KeepNote tree, WYSIWYG);
- feature list taken from `docs/STATUS.md` → “Done” (tree & Trash, editor,
  Markdown-style typing, tabs with × and + and Ctrl+T, autosave/`.prev`/honest
  status, backups, session restore, search, Markdown export, themes, counts, AI
  mirror, shortcuts);
- **Run (portable)** — `.\dist\Nodi\nodi.exe`, `dist\Nodi` movable as a whole,
  data in `dist\Nodi\data`, `target\…` builds use `E:\nodi-app\mark-3\data`,
  `NODI_DATA_DIR`;
- **Build** — `powershell -ExecutionPolicy Bypass -File scripts\build.ps1` →
  `dist\Nodi\nodi.exe`, plus the three prerequisites (Rust in `E:\Software\Rust`,
  pinned `stable-x86_64-pc-windows-gnu`, `windres` search order);
- **Data** — short layout of `data\` and the atomic-write / `Saved` /
  `Save Failed` / retry behaviour;
- **AI agents: the Markdown mirror** — `data\markdown`, start with `INDEX.md`,
  regenerated after each save, edits there are not read back, Tools > Rebuild;
- **Tests** — `cargo test`, `e2e.ps1`, `persist-torture.ps1`, `smoke.ps1`, and a
  link to `docs/STATUS.md`;
- link to [`docs/SHORTCUTS.md`](docs/SHORTCUTS.md) in the feature list.

Deliberately **not** claimed: File > Import Files / Import Folder / Export Note
As / Export Folder (spec §7). At the time of writing the tree has no import code
and `menu.rs` only has `IDM_FILE_EXPORT_MD` (“Export Note as Markdown…”), so
documenting them would make the README wrong. Add one bullet when §7 lands.

## B. scripts/persist-torture.ps1

Harness only — no product code was changed and no bug was fixed (one
unexplained tree state is reported, with a hardened check, in finding 5).

```
pwsh -File scripts\persist-torture.ps1 [-Cycles 20] [-Runs 1] [-Exe <nodi.exe>]
exit code = number of mismatches
```

Reuse of the e2e helpers: the script dot-sources `scripts\e2e.ps1 -Only none`
(same trick as `scripts\probe-lib.ps1`), so `Start-Nodi`, `Invoke-Nodi`,
`Wait-State`, `Stop-Nodi`, `Close-Dialog`, `FindDialog`, `New-DataDir`,
`Read-Tree`, `NodiHook.Send/PostMessageW` are the e2e ones. One fresh data
folder (`target\e2e\persist<run>-<time>`) per run, kept for all cycles of that
run; `-Runs N` repeats the whole thing on N fresh folders.

Every cycle:

1. **Verify the previous cycle** (skipped on cycle 1). Per known note:
   `select-title` + `cmd 171` (Notes > Open), then the `text` hook and the
   `markdown` hook, compared byte for byte (`-cne`). Tree compared from
   `tree.json`: every live node as `title <- parent title`, sorted, so only
   titles and parents count (not `order`, as the task says).
2. **Varied edits** through hooks only:
   - the long-lived note `Torture` gets `type`, `enter`, `tab` (paragraph
     indent) and log lines;
   - `cmd 170` (New Note) + `rename-selected C<n>`, then a structure that
     rotates over the cycles — `cmd 210` heading, `cmd 214` bullets with `enter`
     and a nested `tab`, `cmd 205` code block with `enter`/`tab`, `table 2 2` +
     `cmd 233` column-right, or `cmd 207` separator;
   - a `formula \frac{n}{b} + \sqrt{x}` every cycle, then `type`;
   - `rename-selected C<n>-x` (second rename), `move-rel C<n>-x inside Torture`,
     and `move-rel C<n-1>-x after Torture` for the previous cycle's note.
3. **Expected state, read before closing.** `Wait-TreeWritten` polls
   `tree.json` until it shows exactly what this cycle's moves should have done —
   `C<n>-x` nested under `Torture` *and* `C<n-1>-x` back at the top level (a
   failure there is reported as a mismatch). Then the new note is recorded
   *while it is still the active tab*, `tree.json` is read, and only then the
   last edit is made in `Torture` (`type end <n>`) and `Torture` is recorded
   without switching tabs. This ordering matters: switching tabs flushes, so
   recording the log note last would make mode 1 close a clean buffer. With
   this order mode 1 really closes a `Modified` buffer (see the runs below).
4. **Close in rotation** `mode = ((cycle-1) % 4) + 1`:

   | mode | how it closes |
   |---|---|
   | 1 | `WM_CLOSE` immediately after the last edit; no Ctrl+S, no `Saved` wait. If Nodi asks “close anyway?” the dialog is answered Yes and the line says so. |
   | 2 | `cmd 101` (File > Save, the Ctrl+S id from `menu.rs`) then `WM_CLOSE`. |
   | 3 | `PostMessage WM_QUERYENDSESSION` (0x11, wparam 0) + `WM_ENDSESSION` (0x16, wparam 1), wait up to 3 s for the exit; if it is still alive it is killed, exactly what Windows does after a shutdown. |
   | 4 | `Wait-State 'Saved'` (max 8 s) then `Stop-Process -Force` (`.Kill()`). |

5. After the last cycle one more start verifies the final expected state (that
   state is otherwise only checked by the next cycle's start). Mismatches there
   count too.

Output: one line per cycle (`cycle n/N mode m close: … ok|MISMATCH (k)`), one
indented diff line per problem (`note 'X' text line 4: expected 'end 1', got
'end 1SABOTAGE1'`, `tree (title <- parent) line 3: …`), a `final check` line,
`run r : N cycles, M mismatches`, then `N*Runs cycles, M mismatches`.

## C. Runs

### `-Cycles 6` (the requested smoke run)

```
run 1/1 on E:\nodi-app\mark-3\target\e2e\persist1-194632-202
cycle 1/6 mode 1 close: WM_CLOSE (state 'Modified') ok
cycle 2/6 mode 2 close: Ctrl+S then WM_CLOSE (state 'Saved') ok
cycle 3/6 mode 3 close: WM_QUERYENDSESSION + WM_ENDSESSION (flushed, did not exit -> killed) ok
cycle 4/6 mode 4 close: waited Saved after 513 ms, then Stop-Process -Force ok
cycle 5/6 mode 1 close: WM_CLOSE (state 'Modified') ok
cycle 6/6 mode 2 close: Ctrl+S then WM_CLOSE (state 'Saved') ok
final check after cycle 6 ok
run 1 : 6 cycles, 0 mismatches
6 cycles, 0 mismatches
```

Exit code 0.

### `-Cycles 20` (spec §12 asks for at least 20; 61 s wall clock)

Five 20-cycle runs were made: two on the 19:21 build (before the structural
check existed, one of them the anomaly in finding 5) and three on the 19:40:49
build with it — all five reported `20 cycles, 0 mismatches`. One, verbatim:

```
run 1/1 on E:\nodi-app\mark-3\target\e2e\persist1-194317-184
cycle 1/20 mode 1 close: WM_CLOSE (state 'Modified') ok
cycle 2/20 mode 2 close: Ctrl+S then WM_CLOSE (state 'Saved') ok
cycle 3/20 mode 3 close: WM_QUERYENDSESSION + WM_ENDSESSION (flushed, did not exit -> killed) ok
cycle 4/20 mode 4 close: waited Saved after 495 ms, then Stop-Process -Force ok
cycle 5/20 mode 1 close: WM_CLOSE (state 'Modified') ok
cycle 6/20 mode 2 close: Ctrl+S then WM_CLOSE (state 'Saved') ok
cycle 7/20 mode 3 close: WM_QUERYENDSESSION + WM_ENDSESSION (flushed, did not exit -> killed) ok
cycle 8/20 mode 4 close: waited Saved after 504 ms, then Stop-Process -Force ok
cycle 9/20 mode 1 close: WM_CLOSE (state 'Modified') ok
cycle 10/20 mode 2 close: Ctrl+S then WM_CLOSE (state 'Saved') ok
cycle 11/20 mode 3 close: WM_QUERYENDSESSION + WM_ENDSESSION (flushed, did not exit -> killed) ok
cycle 12/20 mode 4 close: waited Saved after 492 ms, then Stop-Process -Force ok
cycle 13/20 mode 1 close: WM_CLOSE (state 'Modified') ok
cycle 14/20 mode 2 close: Ctrl+S then WM_CLOSE (state 'Saved') ok
cycle 15/20 mode 3 close: WM_QUERYENDSESSION + WM_ENDSESSION (flushed, did not exit -> killed) ok
cycle 16/20 mode 4 close: waited Saved after 535 ms, then Stop-Process -Force ok
cycle 17/20 mode 1 close: WM_CLOSE (state 'Modified') ok
cycle 18/20 mode 2 close: Ctrl+S then WM_CLOSE (state 'Saved') ok
cycle 19/20 mode 3 close: WM_QUERYENDSESSION + WM_ENDSESSION (flushed, did not exit -> killed) ok
cycle 20/20 mode 4 close: waited Saved after 492 ms, then Stop-Process -Force ok
final check after cycle 20 ok
run 1 : 20 cycles, 0 mismatches
20 cycles, 0 mismatches
```

Exit code 0. End state of that data folder: 21 notes (`Torture` + `C1-x` …
`C20-x`), 21 `.rtf` + 21 `.txt` + the mirror (`INDEX.md`, `Torture.md`,
`C<n>-x.md`), `C20-x` nested under `Torture` and `C1-x` … `C19-x` at the top
level — i.e. every move/rename of the last 20 cycles was on disk after each
restart.

### Negative control (proof the harness can fail)

A throwaway copy of the script (outside the repo) with one extra edit inserted
between “record the expected state” and “close” — i.e. the kind of silent loss
the harness is looking for:

```
run 1/1 on E:\nodi-app\mark-3\target\e2e\persist1-193233-850
cycle 1/2 mode 1 close: WM_CLOSE (state 'Modified') ok
cycle 2/2 mode 2 close: Ctrl+S then WM_CLOSE (state 'Saved') MISMATCH (2)
    note 'Torture' text line 4: expected 'end 1', got 'end 1SABOTAGE1'
    note 'Torture' markdown line 7: expected '��end 1', got '��end 1SABOTAGE1'
final check after cycle 2 MISMATCH (2)
    note 'Torture' text line 7: expected 'end 2', got 'end 2SABOTAGE2'
    note 'Torture' markdown line 13: expected '����end 2', got '����end 2SABOTAGE2'
run 1 : 2 cycles, 4 mismatches
2 cycles, 4 mismatches
```

Exit code 4 = number of mismatches, the diff is short, the run continued. So a
0-mismatch result above is meaningful. (The `��` in the markdown lines are the
two non-breaking spaces of the paragraph indent, mangled by the console code
page — the comparison itself is byte exact.)

## D. Findings

1. **No note text was lost** in five 20-cycle runs and four 6-cycle runs (about
   130 Nodi starts, all four close modes, plus the final-check starts): every
   note's `text` and `markdown` came back byte for byte, after `WM_CLOSE` right
   after an edit, after Ctrl+S + close, after a simulated shutdown that ended in
   a kill, and after `Stop-Process -Force` once `Saved` was shown. Nothing to
   fix in the code that was exercised; the harness reports, it does not repair.
2. **Mode 1 closed a genuinely modified buffer** in every one of its cycles
   (`state 'Modified'` is printed just before the `WM_CLOSE`), so the
   “close immediately, do not wait for Saved” path is covered for real, not
   accidentally after an autosave. `maybe_close` flushed everything and never
   showed “close anyway?” in any mode.
3. **`WM_ENDSESSION` does not end the process**: `wnd/mod.rs` flushes on
   `WM_QUERYENDSESSION` and `WM_ENDSESSION(give_up=1)` but returns 0 without
   `DestroyWindow`, so Nodi keeps running after a shutdown message. The data is
   flushed synchronously, which is why every mode-3 cycle passed, and a real
   Windows shutdown kills the process anyway — so this is a behaviour nit, not a
   loss: *after a shutdown request Nodi should exit (or hide) itself instead of
   waiting to be killed*. The harness kills it after 3 s and says so in the
   cycle line.
4. **`Saved` latency** in mode 4: 476–550 ms after the last edit (the 0.5 s
   autosave debounce), then the kill is safe. No `Save Failed` appeared in any
   cycle.
5. **One unexplained tree state, seen once**: the 19:36 20-cycle run (on the
   19:21 build) ended with `C1-x` still nested under `Torture`, although cycle 2
   had moved it out (`move-rel C1-x after Torture` returned `ok`), and the run
   reported 0 mismatches because the “expected” tree is read from `tree.json`
   and therefore accepted whatever stable structure it found. The other run on
   that build and all three runs on the 19:40:49 build ended with `C1-x` at the
   top level, so it did not reproduce — possibly the newer `move-rel` path
   (`app.move_node_relative`) already changed the behaviour. No product fix was
   made (harness-only task). The harness was hardened instead:
   `Wait-TreeWritten` now asserts the *intended* structure (`C<n>-x` under
   `Torture` **and** `C<n-1>-x` at the top level) on every cycle, so a repeat is
   a hard mismatch at the cycle it happens, with the cycle number. If it shows
   up again, look at `Tree::move_relative` / `App::move_node_relative` and at
   whether the `Job::TreeSave` batch coalescing (`pending.tree = Some(json)`)
   can drop the write of the second move of a cycle.
6. Harness caveats worth knowing when reading a failure:
   - the expected state has to be read *before* closing (the task requires it),
     which puts ~50–100 ms between the read and the close; mode 1 is kept
     honest by making the last edit in the log note *after* all other notes are
     recorded (see B.3);
   - `tree.json` is written by a background thread, so the harness waits until
     it shows both moves of the cycle before recording the tree; without that
     wait the “expected” tree could be a state the app never wrote;
   - verification opens every known note (`cmd 171`), so the tab strip grows to
     one tab per note. With 20 cycles ≈ 21 tabs, the run stays at ~61 s.
7. **Not covered by this harness** (out of the agent part's scope, they belong
   to the full §12 campaign): the RTF file hash, a pasted picture, trash /
   restore, the “kill during `Saving…`” case with the slow-write fault hook,
   and the 3 × full 20-cycle campaign. `-Runs 3 -Cycles 20` does the repeat
   part (three fresh data folders, exit code = total mismatches).
