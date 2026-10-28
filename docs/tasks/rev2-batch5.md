# Rev2 Batch 5 — user feedback round 3, import/export, security fix

Same rules/build env as `docs/tasks/rev2-batch1.md`. Every keyboard/mouse behaviour must also be tested with REAL input (SendKeys after the foreground + `{ESC}` trick of e2e R/Y; real mouse via `SendInput`/`SetCursorPos` + `mouse_event` for clicks and drags), plus a screenshot (`scripts/probe-lib.ps1` `Save-Shot`) for anything visual.

## 0. Security: RTF injection through link targets (do first)
Execute `docs/tasks/rev2-fix-rtf-link-escape.md` completely (its tests are part of this batch).

## 1. Heading typing bug
User report: with the caret inside a heading (H1–H4), typing makes something look **selected/highlighted** (flicker of a selection) while typing. Reproduce with real keys (`# Judul{ENTER}`, then click/arrow back into the heading and type), capture screenshots during typing (several in a row). Likely cause: the marker colour/visibility code (`normalise_heading_colour`, marker show/hide on `EN_SELCHANGE`) changes the selection with `EM_EXSETSEL` and redraws. Fix so formatting is applied without visible selection changes: prefer TOM (`ITextDocument`/`ITextRange` via `EM_GETOLEINTERFACE`, raw vtables like elsewhere) to format a range without moving the selection; otherwise wrap in `WM_SETREDRAW` false/true + `EM_HIDESELECTION` and restore the exact selection and scroll position. Also make sure no change runs on every keystroke when nothing needs changing (check the state first). Undo stack must not fill with formatting steps. e2e: real-key typing in a heading, then screenshot pixel check that no selection-highlight colour appears in the heading line, and `sel` hook start==end after each key.

## 2. Tabs: × and + buttons, Ctrl+T
- Each tab shows a small **×** close button on its right (owner-drawn tab or a hit-tested region; hover highlight; DPI-aware). Clicking it closes that tab (same path as Ctrl+W, flush save first).
- A **+** button right after the last tab creates a new note and opens it in a new tab.
- **Ctrl+T** = new note in a new tab (same as +). Add to accelerators, menu (File > New Tab), SHORTCUTS.md, F1 list.
- Dark theme must style both buttons.
- e2e: real mouse click on × of the 2nd tab closes it; real click on + adds a tab; real Ctrl+T adds a tab.

## 3. Note title follows the first line
New notes are created as "New Note" (keep). While a note still has its automatic title (new field `auto_title: bool` in the tree node, serde default false for old notes; true when created by New Note / + / Ctrl+T), its title follows the **first non-empty paragraph** of the text (heading or normal; strip `#` markers, list bullets, trim, max 60 chars, empty → "New Note"). Update tree label + tab caption + window title when the first line changes (debounced with autosave, not on every key). Manual rename sets `auto_title=false` forever. Mirror file name follows the title. e2e: real keys `Rapat mingguan{ENTER}isi` → title "Rapat mingguan"; after F2 rename to "X", typing a new first line does not change it.

## 4. Notes Explorer undo (Ctrl+Z in the tree)
- Undo stack (session only, last 20) for tree operations: Move to Trash, move/drag/reorder, rename, delete from Trash is NOT undoable (permanent, already asks).
- Ctrl+Z while the tree has focus (and Edit > Undo when the tree is focused) shows a confirmation: "Undo: restore 'Judul' from the Trash?" / "Undo: move 'Judul' back to 'Folder'?" / "Undo: rename 'B' back to 'A'?" → Yes applies it (core functions, saved immediately), No keeps it. Ctrl+Y redo optional.
- Core: operations recorded as data (`TreeUndo` enum) with unit tests (trash→restore puts the node back at the same parent and position with its children).
- e2e: real keys: select note, Del (confirm), Ctrl+Z (confirm Yes) → note is back at the same position with its sub-notes.

## 5. Tree drag & drop must feel right
The user still reports that moving notes is awkward ("only via the right side"). Verify with a REAL mouse drag (SendInput) on the current build: dragging a note above the first item, between two items, into a folder, and to the end. The insertion line must be visible across the full row width (screenshot), the drop must work anywhere in the row height (not only when the pointer is over the label text: use `TVM_HITTEST` with the row rect, `TVHT_ONITEMRIGHT`/`ONITEMINDENT`/`ONITEMBUTTON` count as the row), and above the first item / below the last item must work. Add e2e AG with real mouse drags + tree.json check.

## 6. Ctrl+Up / Ctrl+Down scroll (toggle)
View > "Ctrl+Up/Down Scrolls the View" (checkable, default ON, persisted in `settings.json`). When ON, Ctrl+Up/Down scroll the editor by one line without moving the caret (Notepad++ behaviour, `EM_LINESCROLL`); when OFF, Rich Edit's default (paragraph jump). e2e real keys: first visible line changes, caret unchanged; toggle OFF → caret moves.

## 7. Open existing notes: Import (non-destructive) and Export / Save As
Concept (decided by the orchestrator): Nodi keeps ONE library (its data folder, always autosaved). Outside files are never edited in place.
- **File > Import Files...** (multi-select: `.md`, `.markdown`, `.txt`, `.rtf`) and **File > Import Folder...**: each file becomes a note (title = file name without extension), a folder becomes a folder node keeping the sub-folder structure. Markdown via `markdown_in` (local images referenced relatively are embedded; remote images become links, no network), `.txt` as plain text (UTF-8/UTF-16 BOM/ANSI detection), `.rtf` as-is (pictures kept). Originals are only read. Import runs off the UI thread with a progress line in the status bar and a summary ("12 notes imported, 1 skipped: reason"). Imported notes go under a new folder "Imported <date>" (or the selected node).
- **File > Export Note As...** (`.md` with picture folder, `.rtf`, `.txt`) and **File > Export Folder...** (whole subtree as folders of `.md`). This is the "Save As" the user is missing.
- Drag files/folders from Explorer onto the tree = import (WM_DROPFILES / `DragAcceptFiles`).
- Unit tests for txt encoding detection, folder walk → tree plan; e2e: import a temp folder with 2 `.md` + 1 `.txt` + a sub-folder → tree structure + markdown content round trip; export `.md` file content.

## 8. AI agents: where to point them
- Settings dialog (Tools > Settings...): "Markdown mirror folder for AI agents" (default `<data>\markdown`, Browse... to choose any local folder, e.g. `E:\catatan-ai`; changing it rebuilds the mirror there; never deletes the old folder, just stops updating it). Network paths refused.
- Tools > **Copy AI Folder Path** (puts the mirror folder path on the clipboard) and Tools > **Open AI Folder** (Explorer).
- Help > "Using Nodi with AI agents": short dialog text: point the agent at the mirror folder, start with `INDEX.md`; files are regenerated after each save; edits made there are not read back.

## 9. README
`README.md` on GitHub: title **Nodi App**, remove "Mark III" from title/headings/intro (the folder name `mark-3` may stay where it is a path). Keep how-to-run/build instructions current (Ctrl+T, import/export, AI mirror folder).

## 10. Paste formulas
Pasting math must become Nodi formulas (the existing `nodi-math:` LaTeX pictures):
- Plain text / Markdown: `$...$`, `$$...$$`, `\(...\)`, `\[...\]` (ChatGPT "copy" button output) → formula; a selection that is only LaTeX (e.g. `\frac{a}{b} + \sqrt{x}`) pasted with Ctrl+Shift+M or Insert > Formula from Clipboard → formula.
- HTML from ChatGPT/Wikipedia/KaTeX/MathJax: take the TeX from `<annotation encoding="application/x-tex">`, `<math alttext="...">`, `<script type="math/tex">`, or MathJax `data-latex`; drop the visual MathML/span duplicates so the formula is not pasted twice.
- Unsupported LaTeX: keep it as text in inline-code style and list it in the status bar ("2 formulas kept as text: \\foo unsupported"); never lose content.
- Unit tests (KaTeX HTML snippet from ChatGPT, Wikipedia `alttext`, `\[ \]`), e2e paste → `formulas` count and LaTeX via hook, survives restart.

## 11. Counts in the status bar
Status bar shows: `Ln x, Col y | Lines N | Words N | Chars N` for the note, and when there is a selection `Sel: L lines, W words, C chars`. Updates debounced (≤ 150 ms after typing, cheap on a 1 MB note: count off the UI thread or incrementally; measure with perf). Counts ignore hidden field instructions and table marks. If batch 4 already added word/char counts, extend them. e2e via `status` hook text with known content and a known selection.

## 12. Save reliability torture test (user lost notes in KeepNote — top priority)
Add `scripts/persist-torture.ps1` (and an e2e case that runs a short version) which, on a fresh data dir, for **at least 20 cycles**:
1. start Nodi, verify the content from the previous cycle exactly (text via `text` hook + RTF file hash + `markdown` export + tree.json titles/order), 
2. make varied edits with REAL keys and commands: typing in several notes/tabs, headings, lists, bold/colour, a table, a code block, a formula, a pasted picture, rename, move, new child note, trash/restore,
3. close in a rotating way: normal close (WM_CLOSE) immediately after the last key (no waiting for `Saved`), Ctrl+S then close, `WM_QUERYENDSESSION/WM_ENDSESSION` (Windows shutdown), and **kill the process** (`Stop-Process -Force`) only after the status showed `Saved`,
4. record the expected state after each cycle.
Any difference = failure with a diff printed. Also: kill during `Saving...` (inject a slow write via the existing fault hook) → on next start the note is either the new or the previous version (`.prev`), never empty/corrupt, and the app tells the user if it recovered. Run the full 20+ cycles 3 times; report the numbers in the batch report and STATUS. Fix every loss found.

## Tests / done
All earlier e2e (incl. R, Y and batch 4 tests) pass; new e2e for every section; fmt/clippy/test clean. Update SHORTCUTS/STATUS/ARCHITECTURE; write `docs/tasks/rev2-batch5-REPORT.md` listing each section with evidence (test name + screenshot path).
