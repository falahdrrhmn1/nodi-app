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
| extra — format bar | Heading 4 and Normal via the style list, `####` export; the style list is measured for its longest entry | **PASS** (e2e P) |
| extra — tab width | tab stops 360 twips, code blocks 480 twips (measured with `EM_POSFROMCHAR`) | **PASS** (e2e Q) |
| extra — Trash | confirmation with sub-note count, permanent `Trash (N)` item, Empty Trash → `recovery/` | **PASS** (e2e S) |
| extra — real keyboard | foreground window, typed text, Ctrl+A/B, Ctrl+N, F2 rename | **PASS** (e2e R) |
| extra — lists | `- ` → bullet, Enter → item, Tab/Shift+Tab nesting, Backspace keeps indent, markdown | **PASS** (e2e T, U) |
| extra — paragraph indent | Tab indents the paragraph, Enter keeps it, wrapped lines align, restart | **PASS** (e2e V) |
| extra — headings | `## ` → H2, grey marker visible while editing and hidden after leaving, level changes, restart, markdown | **PASS** (e2e W) |
| extra — separator | `---` + Enter, Insert > Separator (Ctrl+Shift+-), restart, markdown `---` | **PASS** (e2e X) |
| extra — heading colour / list numbering | typed `#` heading gets automatic colour, Tab nests a list item correctly, separator width in twips | **PASS** (e2e Y) |
| extra — paste Markdown | Markdown on the clipboard becomes headings, lists, a code table, a GFM table and links | **PASS** (e2e Z) |
| extra — paste HTML | browser CF_HTML becomes the same structure (no network) | **PASS** (e2e AA) |
| extra — code Copy link | plain click on “Copy” copies exactly the code block text | **PASS** (e2e AB) |
| extra — output block | dark block inserts, persists and exports as ```` ```output ```` | **PASS** (e2e AC) |
| extra — picture crop | crop left/top/right/bottom survives save and restart | **PASS** (e2e AD) |
| extra — local path links | typed, quoted and pasted `C:\…` paths become links and export back as paths | **PASS** (e2e AE) |
| extra — tree drag & drop | `before` / `after` / `inside`, cycle refusal, dense order, Move Up/Top Level, restart | **PASS** (e2e AF) |
| extra — dark theme | persisted in `settings.json`, editor pixel is dark, text stays *automatic* in the stored RTF and after switching back | **PASS** (e2e AG + screenshots) |
| extra — AI Markdown mirror | front matter, rename + move follow, old file kept in `.trash/`, `INDEX.md`, `index.json` | **PASS** (e2e AH + 5 unit tests) |
| extra — word/char counts | note and selection counts in the status bar | **PASS** (e2e AI) |
| extra — tab menu | Close Others keeps one tab, notes untouched | **PASS** (e2e AJ) |
| extra — restore backup | Backup Now, rename, restore → old title back, current state snapshotted first | **PASS** (e2e AK) |
| extra — recent notes, outline shortcut | recent list stored in the session; Ctrl+Alt+O with real keys | **PASS** (e2e AL) |
| security — pasted links | a hostile `href` (`}}{\object…`) cannot inject RTF; `javascript:` etc. stay text | **PASS** (e2e AM + unit tests) |
| extra — typing in a heading | real keys: no selection left behind, Ctrl+Z undoes the typing (not a formatting step) | **PASS** (e2e AN) |
| extra — tab × and + | real mouse clicks on × and +, real Ctrl+T | **PASS** (e2e AO) |
| extra — automatic titles | real keys: first line becomes the title; a manual rename sticks | **PASS** (e2e AP) |
| extra — Notes Explorer undo | Trash a note with a sub-note, real click in the tree + Ctrl+Z, confirmation, same place | **PASS** (e2e AQ) |
| extra — Ctrl+Up/Down | real keys scroll without moving the caret; toggle off → caret moves; saved | **PASS** (e2e AR) |
| extra — pasted formulas | `$…$` and `\[…\]` rendered, unsupported LaTeX kept as text, `$5` stays text, restart | **PASS** (e2e AS + unit tests) |
| extra — import / export | folder with `.md`, ANSI `.txt`, sub-folder → notes; Export Folder round trip; original untouched | **PASS** (e2e AT + unit tests) |
| extra — AI mirror folder | moved to an empty folder; a used folder gets `Nodi AI`; user files untouched; back to default | **PASS** (e2e AU) |
| extra — tree drag & drop (real mouse) | drop on the right side of a row above the first item; drop inside a folder | **PASS** (e2e AV) |
| extra — status counts | `Lines · Words · Chars` and the selection's counts | **PASS** (e2e AW) |
| **save torture** | `scripts/persist-torture.ps1 -Cycles 20 -Runs 3`: every cycle restarts Nodi and compares every note's text and Markdown and the tree with what was there before closing; closing rotates over WM_CLOSE right after typing (buffer still `Modified`), Ctrl+S + close, Windows shutdown messages, and `Stop-Process -Force` after `Saved` | **PASS**: 2 × 60 cycles, 0 mismatches (plus ≈130 earlier starts by the harness author) |

| batch 6 — lists | any depth, Backspace after a deeper bullet removes it, outline numbering (nested starts at 1, Shift+Tab continues), Restart Numbering kept after restart | **PASS** (e2e BA) |
| batch 6 — checklist | `[ ] ` / `[x] `, Enter continues, Ctrl+Enter ticks, Markdown task list | **PASS** (e2e BB, BR) |
| batch 6 — typing helpers | two quick spaces → `. `, pause keeps two; Ctrl+Backspace in the Link dialog (real keys) | **PASS** (e2e BC, BD) |
| batch 6 — Notes Explorer | Ctrl+click (real mouse) several notes, trash all, Ctrl+Z restores all; lock (read-only, no rename/trash); sort by name with numbers; Trash last | **PASS** (e2e BE, BF, BG) |
| batch 6 — page break | `----` + Enter, survives restart, `page-break-after` in Markdown | **PASS** (e2e BH) |
| batch 6 — paste | terminal box table → table; HTML task list and embedded picture | **PASS** (e2e BI, BR) |
| batch 6 — import / export | export all as md/rtf/txt, complete backup copy, restore from it; import into a folder keeps the source, Save Back keeps the old file | **PASS** (e2e BJ, BK) |
| batch 6 — themes, sounds, view | preset + copy/paste own theme; silent dialog; line numbers & paragraph `Ln`; default font, font box applies on Enter only; Shift+click (real mouse); background persists | **PASS** (e2e BL–BQ) |
| batch 6 — save torture | `persist-torture.ps1 -Cycles 20 -Runs 2` with new content (outline numbers, checklist, page break, picture, pasted terminal table) and a fifth close mode: Ctrl+S then `Stop-Process -Force` at once (often while `Saving...`) — the last edit may be lost, nothing else | **PASS**: 40 cycles, 0 mismatches. Found on the way: after a kill during a save the next start stopped at a "leftover files" dialog before the window appeared → now a status-bar line |
| batch 6 — heavy note | `stress-heavy.ps1`: 3000 paragraphs, 15 pictures, 15 page breaks, numbered lists, checklists, tables (150 000 characters); killed 0–40 ms after Ctrl+S, mostly during `Saving...` | **PASS**: 10 + 8 kills, every reopen the new or the previous complete version, 0 damaged. Found on the way: Enter in a numbered list of such a note took > 10 s (renumbering the whole note, a status-bar update per paragraph) → only the list around the caret is renumbered, without selection notifications (note built in 8–11 s instead of timing out) |
| batch 6 — low memory | the heavy-note run with a second process holding 1.5 GB (≈ 970 MB of 7.7 GB left) | **PASS**: 8 kills, 0 damaged |
| batch 6 — mixed content | heading, nested numbers, checklist, page break, picture, table: identical Markdown and text after close (no Ctrl+S) and reopen | **PASS** (e2e BS) |

Unit and integration tests: `cargo test` — all pass (nodi-core: tree,
storage, autosave, backup, Markdown mirror, search, links, image, RTF and theme
colours, Markdown import/export, HTML import, LaTeX parser, math layout, PNG; nodi-windows: table structure,
link parsing/safety). `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` are clean.

Real mouse input (cursor moved and clicked by the tests, in the window's own
DPI mode) covers the tab × / + buttons, a click in the tree and tree drag &
drop (e2e AO, AQ, AV).

**NOT TESTED automatically:** Still to be tried by hand: dragging a picture's resize handles,
the visual drag feedback (insertion line / row highlight / hover-expand /
auto-scroll are driven by `WM_MOUSEMOVE`, while e2e AF tests the same model
move through a hook), right-click menus, the common dialogs
(open/save/folder/colour/font/find) and the “Move to...” picker. Real keyboard
input is covered by e2e R, which sends keystrokes through `SendKeys` to the
foreground window.

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
  drag & drop with a visible drop indicator (insertion line before/after,
  highlighted row to drop inside as the last child), auto-scroll near the
  edges, hover-expand after ~0.7 s, Esc to cancel, Move Up/Down, **Move to...**
  picker (any parent or Top level), Move to Top Level, Copy Internal Link,
  Move to Trash with a confirmation that names the
  sub-note count, a permanent `Trash (N)` item at the bottom (click/Enter opens
  the Trash, right-click → Open Trash / Empty Trash…), restore and permanent
  delete (files moved to `recovery/`), properties, Go to Note by title.
- Editor: WYSIWYG rich text (bold, italic, underline, strikethrough, headings
  1–4, bulleted/numbered lists, alignment, font, size, colour, highlight,
  inline code, clear formatting), undo/redo, Unicode, zoom, word wrap,
  duplicate line, go to line, date/time.
- Markdown-style typing (Obsidian-like): `- `/`* `/`1. `/`1) ` make list items,
  `# `…`#### ` make headings (the marker stays in the text but is grey while
  the caret is in that heading and hidden otherwise), `---` + Enter (or
  Ctrl+Shift+-) makes a horizontal separator; Undo right after a list marker
  restores the typed text.
- Lists nest with Tab/Shift+Tab (• ◦ ▪) and wrap with a hanging indent;
  Enter on an empty item ends the list; Backspace at the start of an item drops
  the bullet but keeps the indent.
- Paragraph indentation: Tab/Shift+Tab indent the paragraph itself (360 twips),
  so wrapped lines stay aligned; Enter keeps the indent and legacy notes with
  leading tab characters keep working.
- Word-like format bar: style list (Normal, Heading 1–4), installed-font list,
  size list 8–72 pt, text-colour and highlight palettes (10 colours + More… +
  None), four alignments, word-wrap toggle, lists and the insert commands;
  every button has a tooltip with its shortcut. The lists follow the caret.
- Tab stops every 360 twips (half of Rich Edit's 0.5") in new and loaded notes;
  code blocks use 480 twips. Double-clicking a picture opens Image > Size….
- Code blocks keep Notepad++-style Enter/Tab: Enter copies the indentation,
  Tab inserts a tab, Shift+Tab removes one indent level.
- Code blocks: two-row table with a language header and a **Copy** link (plain
  click copies the code), grey bordered body, Consolas, code-editor Enter/Tab;
  the header label becomes the Markdown fence info string. **Insert > Output
  Block** (Ctrl+Shift+O) makes the same block with a dark body and the
  `output` label.
- Paste: RTF keeps its native formatting; HTML from a browser (CF_HTML) is
  converted through a small tag scanner; plain text that looks like Markdown
  (two signals or a fence) becomes headings, lists, code tables, GFM tables,
  quotes and links; Ctrl+Shift+V always pastes plain text.
- Pictures: insert from file, paste screenshots/copied images, Rich Edit's
  own resize handles, exact size dialog (pixels or percent, lock aspect ratio),
  **crop** (left/top/right/bottom in pixels, stored as RTF crop amounts),
  25/50/75/100 %, fit to width, alignment, save as file. Stored inside the note
  (no external paths).
- Tables: insert, add rows above/below, add columns left/right, delete rows
  and columns. Real Rich Edit tables, not ASCII.
- Links: web, local file and folder (Browse File / **Browse Folder...**),
  internal `nodi://note/<id>` (survive renames), Ctrl+Click to follow,
  automatic URL detection. A typed quoted/unquoted local path or a pasted path
  becomes a path link; Rich Edit only persists such fields as `file:` URLs, so
  targets are stored that way and shown/exported as the real path. Link safety
  policy below.
- Tabs: × on every tab, + after the last one, Ctrl+T, right-click menu,
  middle-click. A new note is titled by its first line (headings and bullets
  stripped) until it is renamed by hand.
- Notes Explorer undo: Ctrl+Z with the tree focused takes back the last move to
  the Trash (with its sub-notes, at the same place), move or rename, after a
  confirmation (20 steps, this session).
- Ctrl+Up/Down scroll the view without moving the caret (View menu toggle).
- Status bar: `Lines · Words · Chars` of the note and of the selection
  (updated 120 ms after typing pauses).
- Import: File > Import Files / Import Folder and files dropped from Explorer
  (`.md`, `.markdown`, `.txt` in UTF-8/UTF-16/ANSI, `.rtf`; folders keep their
  structure; local pictures of Markdown files are embedded; originals are only
  read). Export: File > Export Note As (`.md` / `.rtf` / `.txt`) and Export
  Folder (Markdown). Pictures dropped on the note are inserted.
- Formulas pasted from ChatGPT/web pages: `$…$`, `$$…$$`, `\(…\)`, `\[…\]`,
  KaTeX/MathJax TeX sources and Wikipedia `alttext` become rendered formulas;
  LaTeX the renderer does not know stays as inline code and is named in the
  status bar.
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
- **Themes**: View > Theme > Light / Dark / Custom... (background, text and
  accent colours). Applies to all tabs immediately and persists. The editor,
  tree, tabs, format bar, status bar and title bar follow the theme; menus and
  the icon toolbar stay light (Windows has no supported dark menus, and the
  standard toolbar glyphs are black). Notes keep their real colours on disk.
- **AI Markdown mirror** (Tools > Keep Markdown Mirror for AI Agents, on by
  default; Tools > Rebuild AI Markdown Mirror): `data\markdown\` holds every
  note as `.md` with front matter, plus `INDEX.md` and `index.json`. Point an
  AI agent at that folder and start with `INDEX.md`.
- Status bar word and character counts (note or selection); View > Spell
  Check (Rich Edit's own); tab right-click menu (Close, Close Others, Close
  All, Copy Internal Link, Reveal in Tree) and middle-click to close; File >
  Recent (last 10); View > Outline (Ctrl+Alt+O) lists the headings and jumps
  to one; Tools > Restore Backup... (snapshots the current state first);
  Change Icon... > From file... (`.ico`/`.png`, copied to `data\icons`).
- Portable build (`scripts/build.ps1` → `dist\Nodi`), data next to the exe.

### Batch 6 (2026-10-29)
See `docs/tasks/rev2-batch6.md` (requests) and `rev2-batch6-REPORT.md`.

### Not done yet / limitations
- **Colour emoji**: Windows' Rich Edit control draws emoji in colour only in
  its Direct2D mode, which a normal (windowed) control cannot switch on; every
  `EM_SETEDITSTYLEEX` flag was tried. Emoji show as Segoe UI Emoji outlines.
- An animated GIF is pasted / dropped as a still picture (its first frame).
- Pictures of pasted web pages are fetched only from public internet
  addresses (never from the LAN, `localhost` or `\\server\share`, which
  could leak the Windows login); those stay links.
- **Typing** inline Markdown (`**bold**`, `` `code` ``) still does not convert;
  pasting Markdown does (see above). The Markdown view stays read-only and the
  note itself is the source of truth.
- Picture rotate and editing in an external program are not implemented
  (crop and exact size are).
- Formula evaluation (calculating results) is not implemented — rendering only.
- Older small dialogs (icon picker, Trash, note picker) do not scale with DPI.
- All tabs of the previous session are loaded at startup (150 tabs ≈ 2.2 s).
- No installer (portable by design); no Android app yet (`nodi-core` is ready
  to be reused).

## Security notes
- Pasted HTML cannot make Nodi read network (UNC) paths or contact local
  addresses: picture paths must be on a local drive, downloads go only to
  public IP addresses, and HTTP redirects are not followed.
  Known residual risk: the address WinHTTP actually connected to is checked
  after the request is answered, so with DNS rebinding one GET (no login —
  autologon policy HIGH —, no cookies, no proxy, no body) can reach a local
  address before Nodi drops the answer. Pinning the IP would need a custom
  TLS client.
- Import sources stored in `tree.json` are only used as canonical local-drive
  `.md`/`.markdown`/`.rtf`/`.txt` paths without commas, quotes or wildcards
  (Show Original File, Save Back).
- Pasted HTML/Markdown can only create web, mail, `file:`/drive-path and Nodi
  links; every link target is percent-encoded before it goes into the RTF, so
  a crafted address cannot add RTF (objects, fields) to a note.
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
