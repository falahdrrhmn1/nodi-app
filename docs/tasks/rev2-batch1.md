# Rev2 Batch 1 — bugs + Word-like formatting toolbar

Read first: `docs/ARCHITECTURE.md` (esp. "Rich Edit findings"), `docs/STATUS.md`, `crates/nodi-windows/src/wnd/menu.rs`, `toolbar.rs`, `tree.rs`, `format.rs`. Work only inside `E:\nodi-app\mark-3`. Do not commit (orchestrator commits).

Build env (PowerShell, every cargo call; RAM is tight):
```
$env:CARGO_HOME='E:\Software\Rust\cargo'; $env:RUSTUP_HOME='E:\Software\Rust\rustup'; $env:PATH="E:\Software\Rust\cargo\bin;$env:PATH"; $env:CARGO_BUILD_JOBS='1'
```
Iterate with debug builds only. Never delete files; never touch `dist\Nodi\data`.

## User-reported bugs (must fix)
1. **Rename from the tree context menu fails.** Right-click a note → Rename: the inline label edit does not get keyboard focus, typing beeps (keys go to the TreeView). `TVM_EDITLABELW` requires the tree to have focus: call `SetFocus(tree)` first (in `tree::begin_rename`). Also make sure a right-click while a label edit is open does not show the empty-area menu. Rename must work from: context menu, F2, Notes > Rename. On end-edit keep the existing `rename_node` flow.
2. **Trash is hard to find; deleting has no confirmation.** (a) "Move to Trash" (context menu, Del key, Notes menu) asks "Move "<title>" (and its N sub-notes) to the Trash?" Yes/No. (b) Show a permanent pseudo-item **"Trash (N)"** at the bottom of the Notes Explorer (not a real node, not draggable; its own icon e.g. SIID_RECYCLER). Clicking/Enter opens the existing Trash window; right-click offers "Open Trash" and "Empty Trash..." (empty = purge all with one confirmation; files go to `recovery/` as purge already does).

## Formatting (the user could not find existing features → make them visible)
3. Headings: add **H4** (`HEADING_SIZES` 4 levels, e.g. 400/320/280/250 twips, Ctrl+Alt+4) and Markdown export `####`. Format menu gets Heading 4.
4. Replace the toolbar text buttons for H1/H2 with a **style combobox** (Normal, Heading 1–4) like Word, showing the style at the caret (update on EN_SELCHANGE). "Normal" turns a heading back into body text (the user said there was no way back).
5. Add a **font name combobox** (all installed fonts via `EnumFontFamiliesExW`, sorted, no `@` vertical fonts) and a **font size combobox** (8–72) to the toolbar; both show the selection's current values and apply to the selection. Keep Format > Font... too.
6. Toolbar buttons: **Text colour** and **Highlight** (each with a small palette dropdown of ~10 colours + "More..." for ChooseColor + "None"), **Align left/center/right/justify** (4 buttons, also align pictures), **Word wrap** toggle (checked state mirrors View > Word Wrap), **Insert table/image/link/code/formula** (keep). Use `TBSTYLE_EX_DRAWDDARROWS` / `BTNS_DROPDOWN` for the palettes. Tooltips with shortcuts for every button.
7. **Tab width is too wide — halve it.** Rich Edit's default tab stop is 0.5"; set default tab stops every 360 twips for new and loaded documents (e.g. `PARAFORMAT2` `cTabCount`/`rgxTabs` for the whole document after load and as default for new text, or `EM_SETTABSTOPS`-equivalent; verify by measuring `EM_POSFROMCHAR` of the char after a tab). Code blocks also 4-space-like width.
8. Picture size with numbers: double-clicking a (non-formula) picture opens the existing Image Size dialog; the picture context menu already has it.

## Tests (must add and pass)
- Extend `scripts/e2e.ps1` with a **real keyboard** test `R`: start Nodi, bring it to the foreground with the Alt trick (`keybd_event(VK_MENU)` down, `SetForegroundWindow`, up; assert `GetForegroundWindow()==hwnd`), then `[System.Windows.Forms.SendKeys]::SendWait`: type text, `^a`, `^b` → `fmt 0 5` bold=1; `^n` → tabs has 2; F2 then type "Renamed{ENTER}" → tree.json has a node titled "Renamed". Skip (report NOT TESTED) only if foreground cannot be obtained.
- Hook commands as needed (e.g. `style` returning the combobox text, `tabpos`), new e2e cases for H4 + Normal, Trash confirmation (answer via Close-Dialog), Empty Trash.
- All existing e2e A–O must still pass. `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` clean.

## Done when
Everything above passes; update `docs/SHORTCUTS.md` and `docs/STATUS.md` (only claims you verified). Write a short `docs/tasks/rev2-batch1-REPORT.md`: changes, test output copied verbatim, anything not done.
