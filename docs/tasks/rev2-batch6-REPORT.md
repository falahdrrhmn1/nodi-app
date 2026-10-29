# Rev2 Batch 6 — report

Done by the orchestrator directly (user: "sisanya kamu yang mengerjakan").
Requests: `rev2-batch6.md` (from `saran dan revisi v2.docx` + the `----` page
idea from chat).

| § | Item | Result | Evidence |
|---|---|---|---|
| 1 | Smoother tree drag | the drag image is hidden while the insertion mark / drop highlight repaint (it left trails), auto-scroll repaints before the image comes back, a rebuild keeps the scroll position and paints once (`WM_SETREDRAW`); the splitter moves tree, tabs, line numbers and editor in one `DeferWindowPos` batch | e2e AF, AV, BE |
| 2 | Title from the first line | also for notes still called "New Note"/"Untitled" from before (once they get text) | e2e AP |
| 3 | Sort | View > Sort Notes: manual / name (natural: `2` < `10`, `01-` first) / created / modified, again = descending, folders first; Move Up/Down explain that a sorted tree is not arranged by hand | unit `natural_cmp`, e2e BG |
| 4 | Multi-select | Ctrl+click / Shift+click (real mouse), drawn in the accent colour; Delete, Move to…, Move to Top Level, Export…, Lock/Unlock and drag act on all; Ctrl+Z undoes them as one step (`TreeUndo::Batch`) | e2e BE |
| 5–8 | Lists | any depth; Backspace right after a deeper bullet/box removes it and keeps the indent; outline numbering (`format::renumber`, nested lists start at 1, Shift+Tab continues); Restart / Continue Numbering (kept in RTF as `\pnindent361`); a list typed in an indented paragraph starts at that indent. Found while testing: a theme change reloaded the note and Rich Edit renumbered nested lists 3, 4… — every `load_rtf` now renumbers | e2e BA, Y, T, U |
| 9 | Checklist | `[ ] ` / `[x] `, toolbar-less Format > Checklist (Ctrl+Shift+9), click on the box or Ctrl+Enter ticks; Markdown `- [ ]` / `- [x]` both ways, HTML checkboxes | e2e BB, BR |
| 10 | Trash last | a new top-level note is inserted above it | e2e BG |
| 11 | Refresh | View > Refresh Notes, F5 in the tree (F5 in a note still inserts the date) | — |
| 12 | Find | Find Notes (Ctrl+Shift+F, also right-click): titles of notes and folders plus the text, with the folder path and a snippet; a folder hit is shown in the tree | — |
| 13 | Ctrl+Backspace in dialogs | every message loop gives Ctrl+Backspace / Ctrl+Delete to `wnd/editword.rs`. Found with real keys: a Nodi dialog kept the keyboard focus on itself after opening, so the first keys went nowhere → `WM_ACTIVATE` hands it to the field | unit `word_boundaries`, e2e BD |
| 14 | Link dialog buttons | OK/Cancel get their own row when the extra buttons do not fit | screenshot |
| 15 | Themes | 15 presets, full `ThemeSpec` (12 colours + chrome font), Theme Editor with Pick Colour / Preview / Copy / Paste, my themes saved in `settings.json`, copy/paste as JSON; dark title bar, menu bar (`WM_UAHDRAWMENU*`), dark pop-up menus, icon toolbar strip | e2e BL, BQ, AG; screenshots |
| 16 | Export/import from the tree | right-click: Export…, Import Files/Folder Here…, Show Original File, Save Back to Original File… | e2e BJ, BK |
| 17 | Shift+click | already worked in every case tried with the real mouse (plain, after a double-click, after a margin click, after wheel scrolling) — test added | e2e BP |
| 18 | Terminal tables | `markdown_in::text_table_markdown`: `│`/`|` rows with `─┼`/`+--+` borders become a table, the lines around stay text; not inside code blocks or tables | unit test, e2e BI |
| 19 | Splitter glitch | see §1 | — |
| 20 | "Saving..." | autosave unchanged (atomic write); new torture mode kills right after Ctrl+S | see below |
| 21–22 | Import / export | import copies and never writes the originals; notes remember `source`; Save Back keeps the old file in `data\recovery\originals`; Remove = Move to Trash (files on disk untouched). Export: this note (Ctrl+Shift+S), selection, all notes — `.md`/`.rtf`/`.txt` in folders like the tree; Complete Backup Copy + Tools > Restore From Backup Folder | e2e AT, BJ, BK |
| 23 | Sounds | Preferences > Sounds (default off): message boxes become Nodi's silent dialog (`NodiForm`), Rich Edit `SES_ALLOWBEEPS` off, no `WM_MENUCHAR` beep | e2e BM |
| 24 | Colour emoji | **not possible** with the windowed Rich Edit: `TO_DISPLAYFONTCOLOR` is refused (needs Direct2D mode), all 32 `EM_SETEDITSTYLEEX` bits tried | probe |
| 25 | Lock | `Node::locked`, inherited by everything inside; read-only editor, no rename / move / trash / new child / icon | e2e BF |
| 26 | Two spaces → ". " | within 400 ms, only after a letter or digit | e2e BC |
| 27 | Preferences | default font + size (new notes and pasted text), sounds, double space, line numbers, Ctrl+Up/Down, spell check, AI mirror | e2e BO |
| 28 | Font box | typed (`CBS_DROPDOWN`) with completion; applied on Enter or a pick, never while typing; Esc goes back | e2e BO |
| 29 | Line numbers | `wnd/gutter.rs`: paragraph numbers on the first row of each paragraph, click selects the line; status `Ln` now counts paragraphs too | e2e BN; screenshots |
| 30 | Background | lines / grid / dots via `wnd/pattern.rs` (key colour + `MaskBlt`, anchored to the document) | screenshots, e2e BQ |
| 31 | Counts hidden | the counts part is sized to its text | — |
| 32 | GIF | inserted and kept as a still picture (first frame) | probe (insert, restart) |
| 33 | Web paste | pictures: `data:` URIs, local files and web addresses (WinHTTP, ≤ 12 × 15 MB); task lists | e2e BR |
| 34 | `----` page break | `----` + Enter / Ctrl+Enter / Insert > Page Break; a 600×24 picture recognised by size; Markdown `<div style="page-break-after: always;"></div>` both ways | unit test, e2e BH, BS |

## Verification

- `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo test`: all pass.
- `scripts\e2e.ps1`: **66 passed, 0 failed** (A–AW + BA–BS).
- Save torture: 2 × 20 cycles with the new content and the new "kill right
  after Ctrl+S" mode — **0 mismatches** (the first run found the startup
  dialog after an interrupted save; fixed).
- Heavy note (`scripts/stress-heavy.ps1`): 18 kills, 8 of them with only
  ≈ 970 MB of RAM free — every reopen intact (new or previous version).
  Found a real slowness first: Enter in a numbered list of a 3000-paragraph
  note renumbered the whole note with a status update per paragraph (> 10 s);
  now only the surrounding list is renumbered, without selection events.
- Security review: pictures in pasted web content are never read from UNC
  paths (NTLM leak) or downloaded from local/LAN addresses, no redirects
  (`links::local_path`, `net::get`, unit tests).
- Locked notes: Rich Edit refuses typing in a read-only note but not
  programmatic changes, so every Nodi editing path checks the lock
  (`editor::refuse_if_locked`); e2e BF types, presses Enter, Bold and Paste
  into a locked note and checks it is unchanged.
