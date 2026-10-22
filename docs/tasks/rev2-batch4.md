# Rev2 Batch 4 — themes, AI-agent-readable notes, extra features

Same rules/build env as `docs/tasks/rev2-batch1.md`. Every UI behaviour that a user triggers with the keyboard must also be tested with REAL keys (see e2e tests R and Y), not only through hooks.

## 0. Fix batch 3 bugs first (orchestrator screenshot after real keys)
Sequence: real keys `## Bagian satu{ENTER}`… (as e2e Y), then Ctrl+V of Markdown containing `### Dari ChatGPT`, `**tebal**`, `` `kode` ``, a ```` ```python ```` fence and a 2×2 pipe table, then real Ctrl+Shift+O and typing `hasil output`.
1. **Pasted table cell text is grey** (looks like the code-label/marker grey leaked into table cells). Table cell text must use the normal text colour (auto). Check the colour of markdown_in tables, and of tables inserted with Insert > Table after a paste.
2. **~200 px empty gap** between the pasted table and the output block inserted right after it, although the raw text has only one empty paragraph there. Find the cause (paragraph spacing/size inherited from the table or the code block, a tall empty row, `\sa`/`\sb`, …) and fix; the gap must be one normal line.
3. **Code text contrast**: the code row text (`print('halo')`) renders light grey on light grey. Use a dark text colour (e.g. #1F1F1F) in code blocks for Light theme; the label may stay grey.
4. Separator: make the line clearly thicker (≥ 3 px at 100 % scaling, user asked for a "thick" line).
Add e2e checks (real keys + paste, `fmt` hook colour of a table cell = auto, `pos` y-distance between the table's last row and the next block ≤ 2 line heights, code text colour dark).

## 1. Themes (dark + custom)
- View > Theme: **Light** (current), **Dark**, **Custom...**. Persist in a new `data/settings.json` (core: `Settings` struct with serde defaults, atomic write like session).
- Dark applies to: editor background/default text colour (`EM_SETBKGNDCOLOR` + default char format colour, without rewriting the colours stored in notes — notes with explicit black text should stay readable: render "auto" colour as light; explicit colours stay as they are), tree + tabs + status bar + toolbar/format bar backgrounds/text, splitter, title bar (`DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE=20)`), scrollbars and tree via `SetWindowTheme(hwnd, "DarkMode_Explorer", null)`; menus may stay light if dark menus need undocumented APIs (document it). Code blocks / output blocks keep readable contrast in both themes.
- Custom...: dialog to pick editor background, text, accent (selection/highlight of tree) colours with ChooseColor; preview by applying live; "Reset".
- Switching theme applies immediately to all open tabs, no restart.

## 2. Notes readable by AI agents
- Keep an always-current Markdown mirror: after every successful note save, the writer thread also writes `data/markdown/<path-of-titles>/<title>.md` (safe file names, collisions get ` (2)`), pictures to `data/markdown/<...>/<title>_files/`, formulas as `$...$`. Rename/move/trash updates or removes the mirror file (trash → move mirror to `data/markdown/.trash/`). The RTF stays the source of truth; the mirror is regenerated, never read back.
- Each mirror file starts with YAML front matter: `id`, `title`, `parent_id`, `path`, `created`, `modified`, `tags: []`.
- `data/markdown/INDEX.md`: tree outline with relative links to every note + a short "How this folder is organised" header for AI agents; `data/markdown/index.json`: machine-readable list (id, title, path, parent, children, modified, md_file).
- Tools > "Rebuild AI Markdown Mirror" regenerates everything; Settings toggle "Keep Markdown mirror for AI agents" (default ON).
- Do it in nodi-core (testable), conversions off the UI thread.

## 3. Extra features (useful, small)
- Status bar: **word and character count** of the note (or selection).
- **Spell check** toggle (View > Spell Check): Rich Edit's built-in `IMF_SPELLCHECKING` via `EM_SETLANGOPTIONS` + `EM_SETEDITSTYLEEX SES_EX_...` as documented; default off; persisted.
- **Tab context menu** (right-click a tab): Close, Close Others, Close All, Copy Internal Link, Reveal in Tree. Middle-click closes a tab.
- **Restore from backup** UI: Tools > Restore Backup... lists snapshots (date, size) → confirm → `backup::restore` (which snapshots the current state first) → reload tree and tabs.
- **Recent notes**: File > Recent (last 10 opened), persisted in session.
- **Outline**: View > Outline (Ctrl+Shift+O conflicts? choose Ctrl+Alt+O): small list window / sidebar section of the note's headings; clicking jumps there.
- **Custom note icons from file**: Change Icon... gets "From file..." (.ico/.png 16–32 px; stored copy in `data/icons/`).

## Tests
e2e for theme switch persistence (+ screenshot check that editor background is dark), mirror files + INDEX/index.json content after create/rename/move/trash, word count, tab context Close Others, restore backup round trip, recent list. Real-key test for at least the shortcuts added here. All earlier e2e (incl. R, Y) pass; fmt/clippy/test clean. Update SHORTCUTS/STATUS/ARCHITECTURE; write `docs/tasks/rev2-batch4-REPORT.md`.
