# Task 2+3 — Notes hierarchy + editor persistence + autosave

Read `docs/ARCHITECTURE.md` (binding), `crates/nodi-core/REPORT.md` and `crates/nodi-windows/REPORT.md` first.
Wire `nodi-windows` to `nodi-core` (path dependency). Stay inside `E:\nodi-app\mark-3`. Do not weaken nodi-core's tests; you may extend nodi-core if needed (add tests for anything new).

## Build env (PowerShell, set before every cargo call)
```
$env:CARGO_HOME='E:\Software\Rust\cargo'; $env:RUSTUP_HOME='E:\Software\Rust\rustup'; $env:PATH="E:\Software\Rust\cargo\bin;$env:PATH"
```
For manual runs use `$env:NODI_DATA_DIR='E:\nodi-app\mark-3\data'`. For tests use a temp data dir.

## Phase 2 — Notes Explorer (KeepNote style)
- Remove the static demo tree items. On startup: `DataDir::resolve()`, `ensure_layout()`, `recover_leftovers()` (if anything recovered, show one MessageBox listing paths), `load_tree()`. Corrupt tree.json → MessageBox offering to restore the newest backup (Yes) or exit without touching anything (No). Never start with an empty tree over a corrupt file.
- If the tree is empty on first run, create one note "Untitled" at root and open it (user can type immediately, no Save As).
- TreeView mirrors `Tree`: each item's lParam maps to the NoteId (keep a HashMap<HTREEITEM, NoteId> + reverse). Folders and notes use different icons from a small image list (build it from shell stock icons via `SHGetStockIconInfo` or `ExtractIconExW` from shell32.dll — no external files). Built-in icon set for "Change Icon": at least 8 choices (folder, note, star, book, flag, code, bulb, warning) chosen from shell32/imageres icons by index; store the icon key string in node.icon.
- Single click/Enter on a note opens it in a tab (reuse existing tab if already open — no duplicates). Folders do not open.
- Context menu (WM_CONTEXTMENU / NM_RCLICK; select item under cursor; keyboard Shift+F10/Apps key works):
  - Empty area: New Note, New Folder, Expand All, Collapse All.
  - Note: Open, New Child Note, New Sibling Note, New Folder (child), Rename (F2), Change Icon..., Duplicate, Move to Trash (Del), Properties (MessageBox: title, id, created, modified, child count).
  - Folder: New Note, New Subfolder, Rename, Change Icon..., Move to Trash.
  - Only show items that actually work. (Move/Copy Internal Link/Paste come later — do not show.)
- Rename inline via TVN_BEGINLABELEDITW/TVN_ENDLABELEDITW; update tab title and window title.
- Drag-and-drop in the tree (TVN_BEGINDRAGW + ImageList_BeginDrag or simple capture-based): drop onto a node = make it the last child; must call `Tree::move_node`, which rejects cycles (show status-bar message on rejection).
- Trash: "Notes > Trash..." menu opens a simple modal list window (ListBox) of trashed roots with Restore and Delete Permanently (confirm dialog; permanent delete moves the note files to `data/recovery/purged-<id>-<ts>` rather than deleting them). Trashing a node closes its open tabs (after flushing their saves).
- Expand/collapse state persists in session.json (`expanded: Vec<NoteId>`). Order persists via `order`.
- Every tree mutation → submit `Job::TreeSave` immediately (it's small).
- Menus: add `Notes` menu (New Note, New Child Note, New Folder, Rename, Change Icon, Trash...) and `Search` menu with "Find Note by Title..." (Ctrl+Shift+F): modal with an edit box + ListBox filtered live via `search_titles`; Enter opens the note.

## Phase 3 — Editor persistence + autosave
- Opening a note: `load_note(id)` → stream RTF into a new Rich Edit via `EM_STREAMIN` (SF_RTF) with an EDITSTREAM callback over a byte slice. Then `EM_SETMODIFY FALSE`, `EM_EMPTYUNDOBUFFER`. Missing file = empty note. If load fails but `.prev` exists → MessageBox offer to open `.prev`.
- Saving: exactly the pipeline in ARCHITECTURE.md "Save pipeline". EM_STREAMOUT SF_RTF for rtf, and `WM_GETTEXTLENGTH`/`EM_GETTEXTEX` (GT_DEFAULT, 1200 codepage) for the plain-text sidecar. Debounce 500 ms via SetTimer per tab (timer id = tab key). `Writer::spawn` notify callback = `PostMessageW(main_hwnd, WM_APP_SAVE_RESULT, 0, Box::into_raw(Box::new(result)) as isize)`; the window proc reclaims the Box. If PostMessageW fails, reclaim and drop to avoid leaks (store hwnd as isize so the closure is Send).
- Per-tab state: note id, latest submitted seq, latest acknowledged-ok seq, dirty flag, last error. Status bar part 2 shows for the ACTIVE tab exactly one of: `Modified`, `Saving...`, `Saved`, `Save Failed`. A freshly opened unmodified note shows `Saved` only if its file exists or it is empty-and-new-but-already-written; for a brand new note write an empty file immediately on creation so `Saved` is true. Tab text gets ` *` while not saved.
- On `Save Failed`: status + one MessageBox per failure streak (not per retry), keep content in memory, retry every 5 s via timer while failing.
- Flush triggers: tab switch, opening another note, closing a tab, Ctrl+S, WM_CLOSE, WM_QUERYENDSESSION (return TRUE after flushing), WM_ENDSESSION. On WM_CLOSE: flush all → `flush_blocking(5s)` → if any tab's latest seq is not acknowledged Ok, MessageBox "Some notes could not be saved: … Close anyway?" (Yes/No). No = stay open.
- Session: on close and on tab changes, save session.json (open tabs, active index, caret pos via EM_EXGETSEL, first visible line via EM_GETFIRSTVISIBLELINE, sidebar width, window placement via GetWindowPlacement, expanded nodes). On startup restore them (skip ids that no longer exist / are trashed).
- Formatting persistence: Bold/Italic/Underline/Strike made with toolbar or Ctrl+B/I/U must survive save → restart (it will, via RTF — verify).
- Clipboard paste: keep Rich Edit default but ensure Unicode text paste works (EM_PASTESPECIAL not needed).
- Backup: on startup, if newest snapshot is older than 24 h (or none) → `backup::snapshot` + `prune(20)` on the writer thread or a short-lived thread (must not block UI). Add `Tools > Backup Now` and `Tools > Open Data Folder` (ShellExecuteW "explore" on the data dir) menu items.
- Title bar: `<note title> - Nodi`.

## Automated verification you must run (write them as scripts in `scripts/`, PowerShell, using a temp NODI_DATA_DIR, never the real data dir)
Write `scripts/e2e.ps1` that drives the real exe through Win32 messages (Add-Type C# with FindWindowEx/SendMessage; find the Rich Edit child of class `RICHEDIT50W`):
- Test A (quick note): launch with empty data dir → a note exists → send text with `EM_REPLACESEL` (UTF-16, include Indonesian + emoji, e.g. "Halo dunia — catatan 😀") → wait until status bar part 2 (`SB_GETTEXTW` on the `msctls_statusbar32` child) reads `Saved` (poll ≤ 3 s) → close with WM_CLOSE → assert process exited → assert `notes\<id>.txt` contains the text → relaunch → assert Rich Edit text equals the text.
- Test C (rich text): select a range with EM_EXSETSEL, send the Bold command via WM_COMMAND with the menu id, wait Saved, restart, read CHARFORMAT2 of that range via EM_GETCHARFORMAT(SCF_SELECTION) → CFE_BOLD set.
- Test E (save failure): add a debug-only hook: env `NODI_FAIL_WRITES=1` sets `fail_writes` at startup AND a hidden accelerator-free menu command `ID_DEBUG_TOGGLE_FAIL` (only in debug builds) — use the env var; type text → status becomes `Save Failed` and never `Saved`; the error MessageBox appears (close it by finding the #32770 dialog and sending IDOK).
- Test B (nested): needs tree commands — use WM_COMMAND with the Notes menu ids on the selected item (select via TVM_SELECTITEM): create A, child B under A, child C under B, rename B via TVM_EDITLABEL... (or a debug-only command that renames the selected node) → restart → read tree.json and assert C.parent == B.id and B.parent == A.id and B.title is new.
Print PASS/FAIL per test. All four must PASS.

## Done when
- `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` (whole workspace) all clean.
- `cargo build --release` ok and `scripts/e2e.ps1` prints PASS for A, B, C, E against the release exe.
- `crates/nodi-windows/REPORT.md` updated with a "Phase 2+3" section: what works, e2e output copied verbatim, known gaps. Do NOT claim anything you did not run.
