# Rev2 Batch 1 — report

Task: `docs/tasks/rev2-batch1.md` (rename/trash bug fixes + Word-like format
toolbar). Debug builds only; nothing committed.

## Changes

### Rename bug
- `wnd/tree.rs`: `begin_rename` calls `SetFocus(tree_view)` before
  `TVM_EDITLABELW`, so the inline label edit receives the keystrokes when the
  rename comes from the context menu, F2 or Notes > Rename.
- `show_context_menu` does nothing while a label edit is open, so a right-click
  during renaming can no longer pop the empty-area menu.
- The end-edit path still funnels through `App::rename_node`, so tree.json and
  the tab title stay in sync.

### Trash
- `App::trash_node` asks `Move "<title>" to the Trash?` or, with children,
  `Move "<title>" (and its N sub-notes) to the Trash?`. All entry points
  (context menu, Del, Notes menu) go through it.
- `wnd/tree.rs` appends a permanent `Trash (N)` pseudo-item at the bottom of
  the Notes Explorer with the `SIID_RECYCLER` icon. It has no entry in
  `tree_items`, so every node command ignores it and it cannot be dragged.
  Clicking it or pressing Enter posts `IDM_NOTES_TRASH_VIEW`; its context menu
  offers **Open Trash** and **Empty Trash...**.
- `App::empty_trash` confirms once, purges every trashed root and moves the
  files to `data/recovery/` (same as the existing purge path).
- `IDM_NOTES_TRASH_EMPTY` is also in the Notes menu and the shortcut list was
  updated.

### Formatting
- `wnd/format.rs`
  - `HEADING_SIZES = [400, 320, 280, 250]` (H1–H4), `heading()` accepts 0..=4.
  - `TAB_STOP_TWIPS = 360`, `CODE_TAB_TWIPS = 480`,
    `apply_document_tabs()` (whole document after load, new paragraphs inherit),
    `set_tab_stops()`, `code_tab_stops_rtf()` for code blocks.
  - `set_text_color`, `set_highlight`, `choose_text_color`,
    `choose_highlight_color`, `editor_dpi`.
- `nodi-core/src/markdown.rs`: four heading levels (`####`), threshold changed
  so the 250-twip H4 is recognised.
- `wnd/menu.rs`: `IDM_FORMAT_H4`, `IDM_TOOLBAR_COLOR`, `IDM_TOOLBAR_HIGHLIGHT`,
  `IDM_NOTES_TRASH_EMPTY`; Heading 4 menu item and Ctrl+Alt+4; palette and
  Empty Trash dispatch; `with_editor` now also refreshes the format lists.
- `wnd/toolbar.rs`: two toolbars.
  - `app.toolbar` keeps New/Save, cut/copy/paste, undo/redo, the **Wrap** check
    button and Find.
  - `app.formatbar` holds three comboboxes (style: Normal + Heading 1–4; font
    names from `EnumFontFamiliesExW`, sorted, no `@` faces; size 8–72 pt),
    text-colour and highlight dropdowns (`BTNS_DROPDOWN` /
    `TBSTYLE_EX_DRAWDDARROWS`, 10 colours + More… + None), four alignment
    buttons, lists, and insert image/table/link/code/formula. Tooltips name the
    shortcut of every button.
  - The combos sit over a width-reserving separator and are positioned with
    `TB_GETRECT`; a `SetWindowSubclass` shim forwards the bar's
    `WM_COMMAND`/`WM_NOTIFY` to the frame.
  - `sync_format` mirrors the caret's style/font/size on `EN_SELCHANGE` and
    after formatting commands.
- `wnd/editkeys.rs`: double-clicking a non-formula picture posts
  `IDM_IMAGE_SIZE` (the formula branch is unchanged).
- `wnd/mod.rs`: both bars are laid out and the combo/dropdown notifications are
  handled.

### Test support
- `testhook.rs` gained `style`, `style-set N`, `fontname`, `fontsize`,
  `tabpos N` (pixels + twips via `EM_POSFROMCHAR`) and `trashitem`.
- `scripts/e2e.ps1`:
  - `NodiHook.Foreground` (Alt trick + `SetForegroundWindow` +
    `GetForegroundWindow` check), `SendAsync` (command on a worker thread so
    the main thread can answer a modal dialog).
  - **P** — H4 via the menu, the three format combos are laid out (style
    88×28 at +282+151, font 118×28, size 50×28), style list shows
    Heading 4/Normal, markdown `#### …` and back to body text.
  - **Q** — tab stops measured with `EM_POSFROMCHAR`: 360 twips in the body,
    480 twips in a code block.
  - **S** — Move to Trash confirmation (answered via `Close-Dialog`),
    `Trash (1)` count, `tree.json` flag, Empty Trash confirmation and
    `data/recovery/`.
  - **R** — real keyboard: foreground assertion, typed text, `^a`, `^b`
    (`fmt 0 5` bold=1), `^n` (two tabs), F2 + `Renamed{ENTER}`
    (a `Renamed` node in `tree.json`). It reports NOT TESTED only when the
    window cannot be foregrounded; in this run it passed.

### Docs
- `docs/SHORTCUTS.md`: H4, Normal, double-click picture, Trash item, rename.
- `docs/STATUS.md`: e2e P/Q/S/R rows, feature list, note that real keyboard is
  now covered.

## Verification (verbatim)

Build environment for every command:

```
$env:CARGO_HOME='E:\Software\Rust\cargo'; $env:RUSTUP_HOME='E:\Software\Rust\rustup'; $env:PATH="E:\Software\Rust\cargo\bin;$env:PATH"; $env:CARGO_BUILD_JOBS='1'
```

`cargo fmt --all --check` — no output (clean).

`cargo clippy --all-targets -- -D warnings`:

```
    Checking nodi-windows v0.3.0 (E:\nodi-app\mark-3\crates\nodi-windows)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.09s
```

(PowerShell prints its `NativeCommandError` wrapper around cargo's stderr; the
cargo output itself is the two lines above and the exit code is 0.)

`cargo test` (result lines):

```
test result: ok. 43 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.21s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

86 tests, 0 failed.

`pwsh -NoProfile -File scripts\e2e.ps1`:

```
A quick note survives restart without Ctrl+S                 PASS (Saved after 522 ms)
B nested notes keep hierarchy after rename + restart         PASS 
C bold survives restart                                      PASS 
E write failure shows Save Failed, never Saved               PASS 
F Enter keeps indentation, Tab/Shift+Tab indent lines        PASS 
G image persists and resizes                                 PASS 
H table: insert, add/delete rows and columns, persist        PASS 
I code block keeps indentation and exports as fenced code    PASS 
J formulas render, persist and keep their LaTeX              PASS 
K links: web and internal note links survive restart         PASS 
L headings, lists and Enter-after-heading                    PASS 
M many notes stay responsive                                 PASS (150 notes created in 4909 ms, restart to window 1418 ms)
O long session: switching tabs while typing loses nothing    PASS (60 edits across 3 tabs)
P Heading 4 and Normal via the format bar and markdown       PASS (combos 88x28+282+151 118x28+374+151 50x28+496+151)
Q tab stops are 360 twips (480 in code blocks)               PASS (body 360 twips, code 480 twips)
S Trash asks for confirmation, counts sub-notes, Empty Trash purges PASS 
R real keyboard: foreground window, Ctrl+A/B, Ctrl+N, F2 rename PASS 
--- 17 passed, 0 failed
```

(The script has no D or N case; that was already true before this batch.)

## Not done / limitations

- The automated tests read the style/font/size lists and drive the commands,
  but choosing a font or size in a combobox and picking a colour from a palette
  need a mouse; those paths are implemented but not covered automatically.
- The context-menu rename and the Trash pseudo-item's right-click menu are not
  automated either (they need real mouse clicks); the rename focus fix is what
  the F2 path in test R exercises.
- Double-clicking a picture is implemented but not part of e2e (picture size is
  covered through the menu in test G).
- The pixel-perfect layout of the two toolbars (button clipping at the minimum
  window width) was not asserted automatically.
