# Rev2 Batch 6 — spec (from `saran dan revisi v2.docx` + chat)

Done by the orchestrator directly (user: "sisanya kamu yang mengerjakan").

| § | Request (user's words, short) | Plan |
|---|---|---|
| 1 | Moving notes "kurang smooth" | insertion line while dragging, auto-scroll near the edges, hover-expand folders |
| 2 | Title follows the first text | exists; also for untouched "New Note"/"Untitled" notes, and the tab/tree update while typing |
| 3 | Sort the tree: original, created date, name (`00-…`, `01-…`) | View > Sort Notes: Manual / Name (natural) / Created / Modified, ascending/descending, folders first |
| 4 | Ctrl+click / Shift+click several notes, then delete/rename/move… | multi-selection in the Notes Explorer; trash, move, export, lock work on all selected |
| 5 | Bullet Backspace like numbers | Backspace at the text start removes any marker, keeps the indent |
| 6 | Tab only 3 levels | any depth |
| 7 | Tab → new numbering, Shift+Tab → continue; restart option | outline numbering (`format::renumber`), Restart/Continue Numbering in the context menu, kept on reload |
| 8 | New list item at the current indent, not the left margin | `- ` / `1. ` start at the paragraph's own level |
| 9 | Checklist | `[ ] ` / `[x] `, toolbar/Format menu, click or Ctrl+Enter ticks |
| 10 | Trash always last | also after adding a note at the top level |
| 11 | Refresh | F5: reload the tree from disk, re-read settings |
| 12 | Find files/folders, by content (priority) | Search panel: title + full-text, results list, open at the hit |
| 13 | Ctrl+Backspace in Find/Link/dialogs types a box | handled in every message loop (`wnd/editword.rs`), Ctrl+Delete too |
| 14 | Link dialog buttons overlap | OK/Cancel get their own row when the extras do not fit |
| 15 | More dark themes; full theme settings; dark title/menu/toolbar; save/copy/paste own themes; VS Code-like presets | theme presets + theme editor (all colours + UI/editor font), export/import as text |
| 16 | Export/import from the tree context menu | yes, also for a multi-selection |
| 17 | Shift+click to extend a selection | verify with the real mouse |
| 18 | Paste of box/pipe tables → readable table | plain text that looks like a table becomes a Nodi table |
| 19 | Splitter drag glitches | redraw without flicker |
| 20 | "Saving..." — is it safe? | explain + full tests |
| 21 | Imported folder: remove without deleting files; does editing change the files? | import = copy (originals never touched); Remove Import; Save back to original (explicit) |
| 22 | Import/export must be complete and clear | Export dialog: note / folder / selection / everything, Markdown/RTF/TXT, full backup |
| 23 | Alert sounds annoying | Preferences > Sounds off (default), no beeps from Nodi |
| 24 | Modern emoji | colour emoji (Segoe UI Emoji) |
| 25 | Lock/unlock notes and folders | read-only until unlocked; no rename/move/delete |
| 26 | Two spaces → period (not after a pause) | 400 ms window |
| 27 | Preferences (default font, theme...) | Tools > Preferences |
| 28 | Typing a font name is buggy | typed combo, applied on Enter / pick |
| 29 | Line numbers between tree and text | gutter like Notepad++ (per paragraph) |
| 30 | Background lines/grid/dots, light | optional editor background pattern |
| 31 | Selection counts hidden by "Rich Text" | counts part sized to its text |
| 32 | GIF paste? | picture (first frame) — explain |
| 33 | Web paste keeps formatting like Notion/Obsidian | better HTML paste (headings, lists, tables, code, quotes, links) |
| 34 | `----` = page separator ("halaman baru") | page break block, Markdown export |
| 35 | Push; test with text, pictures, tables… under stress | e2e + torture with rich content |
