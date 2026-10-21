# Nodi Mark III — Status

Measured on the development laptop (Windows 11, 8 GB RAM, 125 % display
scaling), release build, 2026-10-22.

## Test results

Automated end-to-end tests drive the real `nodi.exe` (`scripts/e2e.ps1`,
throw-away data folders under `target\e2e`). Keys are sent as the same
`WM_KEYDOWN`/`WM_CHAR` messages a keyboard produces; commands go through the
same handlers as the menus.

| Test (spec §16) | Scenario | Result |
|---|---|---|
| A — Quick notes | type, no Ctrl+S, wait for `Saved`, close, reopen | **PASS** (Saved ≈ 0.52 s after typing) |
| B — Nested notes | A > B > C, rename B, restart, hierarchy intact | **PASS** |
| C — Rich text | bold + italic ranges survive restart | **PASS** |
| D — Image persistence | insert picture, resize to 50 %, restart | **PASS** (e2e G) |
| E — Autosave failure | injected write failure → `Save Failed`, never `Saved`, nothing written | **PASS** |
| F — Many notes | 150 notes created, restart, still responsive | **PASS** (150 notes in 6.3 s incl. test IPC; restart 2.2 s with 151 tabs) |
| G — Markdown conversion | headings, lists, code blocks, tables, links, formulas exported; lossy features reported, note untouched | **PASS** (e2e H, I, J, K, L + unit tests) |
| H — Long editing session | 60 edits across 3 tabs with tab switching; 1 MB note | **PASS** (e2e O + perf) |
| extra — auto-indent | Enter keeps indent, Tab/Shift+Tab on selected lines | **PASS** (e2e F) |
| extra — tables | insert, add/delete rows/columns, persist | **PASS** (e2e H) |
| extra — code block | indentation kept, exports as fenced code | **PASS** (e2e I) |
| extra — formulas | render, persist, LaTeX editable, `$…$` export, bad LaTeX rejected | **PASS** (e2e J) |
| extra — links | internal + web links survive restart | **PASS** (e2e K) |
| extra — format bar | Heading 4 and Normal via the style list, `####` export | **PASS** (e2e P) |
| extra — tab width | tab stops 360 twips, code blocks 480 twips (measured with `EM_POSFROMCHAR`) | **PASS** (e2e Q) |
| extra — Trash | confirmation with sub-note count, permanent `Trash (N)` item, Empty Trash → `recovery/` | **PASS** (e2e S) |
| extra — real keyboard | foreground window, typed text, Ctrl+A/B, Ctrl+N, F2 rename | **PASS** (e2e R) |

Unit and integration tests: `cargo test` — 86 tests pass (nodi-core: tree,
storage, autosave, backup, search, links, image, RTF, Markdown, LaTeX parser,
math layout, PNG; nodi-windows: table structure, link parsing/safety).
`cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` are clean.

**NOT TESTED automatically:** physical mouse input (the tests never move the
real pointer). Still to be tried by hand: dragging a picture's resize handles,
drag-and-drop of notes in the tree, right-click menus, the common dialogs
(open/save/colour/font/find). Real keyboard input is covered by e2e R, which
sends keystrokes through `SendKeys` to the foreground window.

## Measurements (release build)

| Metric | Result |
|---|---|
| `nodi.exe` size | 800 KB (single file, no DLLs to ship) |
| Startup to idle window | 117 ms best, ~150–370 ms typical |
| RAM idle (one empty note) | 23.5 MB working set, 4.8 MB private |
| RAM with 10 open notes (text + 640×360 picture + formula each) | 40.9 MB working set, 20.8 MB private |
| Restart with those 10 tabs | ~0.3–0.4 s |
| 1 MB note (12,500 lines): load into editor | 0.75 s |
| Keystroke round trip in the 1 MB note | 7 ms average, 9 ms worst (includes test IPC) |
| Autosave of the 1 MB note | `Saved` 0.55 s after the last key (0.5 s debounce included) |
| RAM with the 1 MB note | 26.3 MB working set |
| Restart with the 1 MB note open | ~1.05 s |

Targets from the spec: startup ≈ 1 s → met; idle RAM < 100 MB → met.

## Features

### Done
- Native Win32 window: classic menus, toolbar with tooltips, Notes Explorer
  tree, resizable sidebar, tabs, status bar, Per-Monitor DPI.
- Notes tree: folders and notes, notes with child notes, context menus,
  inline rename (F2, context menu, Notes > Rename), 8 built-in icons, duplicate,
  drag & drop (drop on empty space → top level), Move Up/Down, Move to Top
  Level, Copy Internal Link, Move to Trash with a confirmation that names the
  sub-note count, a permanent `Trash (N)` item at the bottom (click/Enter opens
  the Trash, right-click → Open Trash / Empty Trash…), restore and permanent
  delete (files moved to `recovery/`), properties, Go to Note by title.
- Editor: WYSIWYG rich text (bold, italic, underline, strikethrough, headings
  1–4, bulleted/numbered lists, alignment, font, size, colour, highlight,
  inline code, clear formatting), undo/redo, Unicode, zoom, word wrap,
  duplicate line, go to line, date/time.
- Word-like format bar: style list (Normal, Heading 1–4), installed-font list,
  size list 8–72 pt, text-colour and highlight palettes (10 colours + More… +
  None), four alignments, word-wrap toggle, lists and the insert commands;
  every button has a tooltip with its shortcut. The lists follow the caret.
- Tab stops every 360 twips (half of Rich Edit's 0.5") in new and loaded notes;
  code blocks use 480 twips. Double-clicking a picture opens Image > Size….
- Notepad++-style indentation: Enter keeps indentation, Tab/Shift+Tab on
  selections, Backspace removes a tab indent.
- Code blocks: grey bordered box, Consolas, code-editor Enter/Tab.
- Pictures: insert from file, paste screenshots/copied images, Rich Edit's
  own resize handles, exact size dialog, 25/50/75/100 %, fit to width,
  alignment, save as file. Stored inside the note (no external paths).
- Tables: insert, add rows above/below, add columns left/right, delete rows
  and columns. Real Rich Edit tables, not ASCII.
- Links: web, local file, internal `nodi://note/<id>` (survive renames),
  Ctrl+Click to follow, automatic URL detection, link safety policy (below).
- Formulas: LaTeX dialog (Alt+=), rendered with a TeX-like layout in Cambria
  Math: fractions, roots, scripts, ∑/∏/∫ with limits, `\left…\right`,
  matrices, cases, accents, Greek, ℝ/ℕ/ℤ, operators. Editable again by
  double-click; errors reported with the position.
- Search: Find/Replace in the note, F3/Shift+F3, Search in All Notes.
- Autosave with atomic writes, `.prev` generation, crash leftovers moved to
  `recovery/`, honest status (`Modified` / `Saving...` / `Saved` /
  `Save Failed`), retry, flush on tab switch / close / Windows shutdown.
- Backups: daily snapshot at startup + Tools > Backup Now, 20 kept, never
  overwritten; restore offered when `tree.json` is corrupt.
- Session restore: open tabs, active tab, caret, scroll, window, sidebar,
  expanded folders, zoom, word wrap.
- Markdown: read-only Markdown view and export (`.md` + picture folder),
  lossy features listed.
- Portable build (`scripts/build.ps1` → `dist\Nodi`), data next to the exe.

### Not done yet / limitations
- **Markdown source editing** (typing Markdown that becomes rich text) is not
  implemented. The Markdown view is read-only; the note itself stays the
  source of truth.
- Picture rotate/crop and editing in an external program are not implemented.
- Dragging picture files from Explorer into a note is not implemented
  (Insert > Image and paste work).
- Custom icons from local files are not implemented (8 built-in icons).
- No "Move to…" picker dialog (use drag & drop / Move Up/Down / Top Level).
- No restore-from-backup menu yet (backups are plain folders in
  `data\backups`; the app offers a restore only when `tree.json` is corrupt).
- Formula evaluation (calculating results) is not implemented — rendering only.
- Older small dialogs (icon picker, Trash, note picker) do not scale with DPI.
- All tabs of the previous session are loaded at startup (150 tabs ≈ 2.2 s).
- No installer (portable by design); no Android app yet (`nodi-core` is ready
  to be reused).

## Security notes
- Links never run programs: `http(s)`/`mailto` open in the browser/mail
  client. File links must be fully qualified drive paths (`X:\...`);
  relative paths, `\\?\`, `\\.\`, `\??\` device/NT paths, alternate data
  streams, wildcards, quotes and reserved device names (`CON`, `NUL`, `COM1`,
  …) are refused. Paths are normalised the way Win32 resolves them (trailing
  dots/spaces dropped, so `calc.exe.` is still `calc.exe`). Only file types on
  a **document allowlist** (pdf, docx, png, txt, …) that Windows' own
  `AssocIsDangerous` does not flag are opened, after a confirmation that shows
  the real path; programs, scripts, shortcuts and unknown types are only shown
  in Explorer on request. Other URL schemes are refused.
- Network locations (`\\server\share`, mapped network drives) are never
  opened or touched from a link, so a link cannot make Windows contact another
  computer (no SMB/NTLM exposure). Hovering a link is side-effect free: the
  status bar shows the real target without accessing the file system; the
  disk is only read after a deliberate Ctrl+Click, and only on local drives
  (checked with `GetDriveTypeW`).
- The test remote control (`testhook.rs`) is only active when `nodi.exe` is
  started with `NODI_TEST_HOOKS=1`.
- No network access, no telemetry, no background processes.

## Disk usage on C:
Notes, backups, logs, the Rust toolchain (`E:\Software\Rust`, GNU target),
the Cargo cache and build output (`E:\nodi-app\mark-3\target`) are all on E:.
Windows itself (Rich Edit, fonts) lives on C: as always.
