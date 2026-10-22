# Rev2 Batch 2 — report

Task: `docs/tasks/rev2-batch2.md` (Word/Obsidian-like lists, wrapping-safe
paragraph indentation, Markdown-style headings with a hidden marker,
`---` separators and the style-combo width fix).

Build environment: same as batch 1 — `CARGO_BUILD_JOBS=1`, debug builds,
no commits, nothing deleted, `dist\Nodi\data` untouched.

## What changed

### `crates/nodi-windows/src/wnd/format.rs` (main new code)

- **Lists.** `start_list(numbered)`, `toggle_list`, `remove_lists`,
  `list_kind`, `list_level`, `apply_list_level`, `new_literal_item`,
  `remove_list_marker`, `end_list`.
  - Level 1 uses Rich Edit's native numbering (`PFN_BULLET`/`PFN_ARABIC`,
    `start = 0`, `offset = 360`, `numberingTab = 360`) so wrapped lines get a
    hanging indent and align with the item text.
  - Rich Edit has only one native bullet glyph, so levels 2–3 are stored as the
    literal `◦\t` / `▪\t` prefix with `start = (level-1) * 360`, `offset = 360`
    and native numbering off. The Markdown reader recognises the same literal
    prefixes.
  - Enter starts the next item; Enter on an empty item clears the list but
    keeps the indent (normal paragraph, same `start`); Backspace at the start
    of an item removes the bullet and keeps the indentation.
- **Paragraph indentation.** `change_indent(editor, delta)` changes
  `PFM_STARTINDENT` for every paragraph in the selection (360 twips per step).
  Because the indent is a paragraph property, wrapped lines stay aligned with
  the indented text; no tab characters are added for new text.
- **Headings.** `heading()`, `heading_level_at`, `maintain_heading`,
  `update_heading_marker`, `normalize_heading_markers`, `heading_marker`,
  `set_marker_visibility`.
  - The heading keeps its literal `#{1,4} ` prefix in the text. While the
    caret is in that paragraph the prefix is grey and visible
    (`CFE_HIDDEN` off); everywhere else it is `CFE_HIDDEN`.
  - Toggling is cheap: `EN_SELCHANGE` only looks at the paragraph that was
    entered and the one that was left (per-editor map of the last paragraph).
    `EN_CHANGE` keeps the marker level in sync with the number of `#`s and
    re-applies the heading style; deleting a `#` returns the paragraph to the
    level implied by the remaining `#`s, zero `#`s makes it body text.
  - `normalize_heading_markers` runs when a note is opened (before change
    events are attached), so markers stored visible are hidden again and RTF
    round-trips keep the prefix.
  - Headings created from the style list / Ctrl+Alt+1–4 get the same prefix, so
    both routes behave identically.
- **Separator.** `insert_separator` / `separator_from_marker`: a generated grey
  PNG bar (`separator_png`, 600×3, editor width) is inserted as a picture
  wrapped in a `HYPERLINK "nodi-hr:"` field (the same technique as formulas),
  so it survives save/reload. `---` alone on a line plus Enter replaces the
  marker; Insert > Separator / Ctrl+Shift+- inserts one anywhere.
- `format_at` / `hidden_at` plus a `Busy` re-entrancy guard: the temporary
  selection used by the test hook (and by our own queries) cannot make a
  hidden heading marker visible.
- `forget_editor` drops the per-tab bookkeeping when a tab closes.

### `crates/nodi-windows/src/wnd/editkeys.rs`

- `space_marker`: `- `/`* ` → bullet, `1. `/`1) ` → numbered list, `# `…`#### `
  → heading, all applied before Rich Edit inserts the space.
- `MarkerUndo` remembers `(editor, paragraph start, typed marker)`; Ctrl+Z
  (`IDM_EDIT_UNDO`, intercepted before `EM_UNDO`) restores the typed text
  instead of undoing an unrelated edit. Typing any later printable character
  clears the memory so a normal undo is not hijacked.
- Enter (`newline_keep_indent`): separator, list items, end-of-heading → body
  text, then the indent-carrying rule. Code blocks (one-cell tables) insert a
  paragraph inside the cell (Rich Edit would otherwise add a table row).
- Tab (`tab_key`): list levels 1–3 with Tab/Shift+Tab; normal paragraphs change
  the paragraph start indent (whole selection when several lines are selected);
  a caret in the middle of a line still inserts a tab character; legacy leading
  tab characters are preserved and Shift+Tab removes them first.
- Backspace at a paragraph start: remove a bullet, turn a heading into body
  text, or outdent one step, in that order.

### `crates/nodi-windows/src/wnd/editor.rs`

- `CFM_HIDDEN` / `CFE_HIDDEN` constants (defined locally; the windows-sys
  version we use does not export them).
- `IDM_EDIT_UNDO` first offers the change to the marker undo.

### `crates/nodi-windows/src/app.rs`, `wnd/mod.rs`

- `EN_CHANGE` → `maintain_heading` + `update_heading_marker`;
  `EN_SELCHANGE` → `update_heading_marker`; note open →
  `normalize_heading_markers`; tab activation → marker refresh; tab close →
  `forget_editor`.

### `crates/nodi-windows/src/wnd/menu.rs`

- `IDM_INSERT_SEPARATOR = 207`, Insert menu item, `Ctrl+Shift+-` accelerator,
  dispatch, and new entries in the in-app shortcut list.

### `crates/nodi-windows/src/wnd/toolbar.rs`

- The style combo is measured with the format bar's font for every entry
  (`Normal`, `Heading 1`…`Heading 4`) plus the drop-down arrow and padding,
  converted back to 96-DPI, so "Heading 2" is no longer clipped. `relayout`
  and `combo_area` use the measured width, so the other combos move along.

### `crates/nodi-core/src/links.rs`

- `RULE_PREFIX`, `format_rule_link()`, `is_rule_link()` and a unit test.

### `crates/nodi-core/src/math/mod.rs`

- The empty-`latex` PNG used for separators omits the `iTXt` chunk (an empty
  LaTeX chunk would make the Markdown reader treat the bar as a formula).

### `crates/nodi-core/src/markdown.rs`

- `Format.hidden`, `Inline::Rule`, `Paragraph.indent`.
- Hidden runs (`\v`) are skipped, so `# ` markers never reach the export.
- A picture inside a `nodi-hr:` field becomes `Inline::Rule`; no image asset is
  written for it.
- Paragraph `\li` (twips / 360) drives list levels and plain indentation;
  literal `◦`/`▪` + tab bullets become nested `- ` items; plain indentation is
  exported as two NBSP per level (leading spaces would turn into a code block).
- Paragraph properties are inherited across `\par` until `\pard` resets them,
  matching RTF (Rich Edit writes `\pard` once and relies on inheritance).
- A leading `#{1,4} ` is stripped from heading text, so the export is
  `## Title`, never `## ## Title`.
- New unit tests: paragraph indent / literal bullet levels, RTF inheritance,
  separator picture → `---`.

### `crates/nodi-windows/src/testhook.rs`

- New hooks: `keys` (types text through `WM_CHAR`, so spaces and tabs behave
  like a keyboard), `backspace`, `para` (start indent, first-line offset,
  numbering, level, text) and `hidden`. `fmt` now reports `hidden=`. `tab`
  sends `WM_KEYDOWN`+`WM_CHAR`+`WM_KEYUP`.

### `scripts/e2e.ps1`

- F now checks the paragraph indent (`start=360`) instead of doubled tab
  characters; L selects after the `# ` marker; new cases T–X.

## Behaviour notes / decisions

- **Indent vs. tab characters.** Tab at the start of a paragraph now changes
  the paragraph start indent; a middle-of-line Tab still inserts a tab. Old
  notes with leading tabs are not rewritten; Enter keeps whatever whitespace
  the paragraph already has and Rich Edit inherits the paragraph indent.
- **One native bullet glyph.** Levels 2 and 3 are literal `◦\t`/`▪\t` text so
  each level can look different; the export turns them back into nested `- `
  items.
- **Field length.** After a separator is inserted the field is wider than one
  character, so the "continue in the next paragraph" break is placed at the
  paragraph end, not at `start + 1` (the latter landed inside the field and
  left an empty paragraph before the rule).
- **Marker undo is explicit** rather than relying on Rich Edit's undo stack,
  because applying list/heading formatting adds its own undo steps.

## Verification (verbatim)

Note: `cargo` writes its progress lines to stderr, which PowerShell shows as
`NativeCommandError`; the exit codes are 0.

### `cargo fmt --all --check`

```text
(no output; exit code 0)
```

### `cargo clippy --all-targets -- -D warnings`

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.23s
```

### `cargo test`

```text
     Running unittests src\lib.rs (target\debug\deps\nodi_core-ee76c7db700215ef.exe)
running 48 tests
test result: ok. 48 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
     Running tests\autosave.rs (target\debug\deps\autosave-7fa3e4599844173e.exe)
running 6 tests
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
     Running tests\backup.rs (target\debug\deps\backup-1d137f52115ca5a1.exe)
running 6 tests
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s
     Running tests\search_links.rs (target\debug\deps\search_links-f05d9db489843648.exe)
running 3 tests
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
     Running tests\storage.rs (target\debug\deps\storage-cd05104a9d3b853a.exe)
running 10 tests
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
     Running tests\tree.rs (target\debug\deps\tree-e4e446c95299af6c.exe)
running 12 tests
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running unittests src\main.rs (target\debug\deps\nodi-e35250c4d1a75bf1.exe)
running 6 tests
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
   Doc-tests nodi_core
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

Total: **91 tests, 0 failed**.

### `pwsh -NoProfile -File scripts\e2e.ps1` (full run)

```text
A quick note survives restart without Ctrl+S                 PASS (Saved after 520 ms)
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
M many notes stay responsive                                 PASS (150 notes created in 4627 ms, restart to window 1418 ms)
O long session: switching tabs while typing loses nothing    PASS (60 edits across 3 tabs)
P Heading 4 and Normal via the format bar and markdown       PASS (combos 103x28+282+151 118x28+391+151 50x28+515+151)
Q tab stops are 360 twips (480 in code blocks)               PASS (body 360 twips, code 480 twips)
S Trash asks for confirmation, counts sub-notes, Empty Trash purges PASS 
R real keyboard: foreground window, Ctrl+A/B, Ctrl+N, F2 rename PASS 
T Markdown lists: typing, nesting, Backspace keeps indent    PASS 
U empty item ends the list, Ctrl+Z restores a typed marker   PASS 
V paragraph indentation survives wrapping and restart        PASS 
W Markdown headings: hidden marker, level changes, restart   PASS 
X separator: --- + Enter, Insert menu, restart and markdown  PASS 
--- 22 passed, 0 failed
```

Case D is intentionally absent from `e2e.ps1`; it is covered by G and by the
`cargo test` storage/RTF tests.

## New/updated e2e cases

| Case | What it checks |
|---|---|
| F (updated) | Tab at a paragraph start sets `dx_start_indent = 360` instead of inserting tab characters; Shift+Tab undoes it |
| L (updated) | The caret is placed after the `# ` marker when editing a heading |
| T | `- ` → bullet, Enter → second item, Tab → level 2 (`start=360`, literal `◦`), Shift+Tab → level 1, Backspace → bullet gone but indent kept, markdown `- satu` |
| U | Enter on an empty item ends the list, Ctrl+Z restores the typed `- `, list survives restart, markdown `- alpha`/`- beta` |
| V | Tab indents the paragraph, Enter keeps the indent, both paragraphs export with two NBSP, restart keeps `start=360` |
| W | `## ` → H2 (`size=320`), marker visible inside / hidden outside, adding and deleting a `#` changes the level, survives restart, markdown `## Judul` |
| X | `---` + Enter → one picture, Insert > Separator adds a second, both survive restart, markdown contains `---` between the texts |

## Not done / follow-ups

- Inline Markdown typed as text (`**bold**`, backtick code, `>`, fenced code)
  is still not converted; only lists, headings, separators and indentation.
- The separator is a picture: selecting it and pressing Delete removes it, but
  there is no "edit separator" dialog (which is fine for a rule).
- Word-style multi-level numbering (1.1, 1.2) is not implemented; levels 2–3
  of a numbered list continue Rich Edit's single-level numbering and are
  exported as nested `1.` items.
