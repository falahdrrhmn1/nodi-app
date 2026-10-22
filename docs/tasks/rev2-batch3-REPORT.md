# Rev2 Batch 3 — report

Task: `docs/tasks/rev2-batch3.md` (three real-key bugs from batch 2, Markdown/HTML
paste, better code blocks, picture crop and size, local-path links with a folder
picker, and tree drag & drop).

Build environment: same as batches 1–2 — `CARGO_BUILD_JOBS=1`, debug builds, no
commits, nothing deleted, `dist\Nodi\data` untouched.

## What changed

### Section 0 — the three real-key bugs

- **Heading body stayed grey.** `normalise_heading_colour` (in
  `crates/nodi-windows/src/wnd/format.rs`) runs on `EN_SELCHANGE`/`EN_CHANGE`
  for the caret paragraph. If the first character after the `#…` marker
  inherited `HEADING_MARKER_GREY` (typing right after the grey marker does
  that), the body is reset to `CFE_AUTOCOLOR` and the insertion colour at the
  caret is reset too, so new text is normal black from the first keystroke.
- **Tab inside a list item lost the bullet.** Level 2–3 items are the literal
  `◦\t` / `▪\t` prefix. The prefix was inserted before the paragraph format was
  set, so Rich Edit re-applied the level-1 numbering over it. `apply_list_level`
  now sets the paragraph format first and inserts the prefix afterwards; the
  character format is applied to the whole new prefix.
- **Separator was too narrow and left an indent.** `plain_paragraph` resets the
  caret paragraph (`PFM_NUMBERING*`, `PFM_STARTINDENT`, `PFM_OFFSET`) before the
  block is inserted, and the width is converted from twips to HIMETRIC
  (`twips * 2540 / 1440`) before `EM_INSERTIMAGE`, so the bar spans the editor
  and the paragraph after it starts at the left margin.
- e2e **Y** reproduces all three through real keystrokes (`SendKeys`), including
  `{TAB}` inside a bullet and `---{ENTER}`.

### Section 1 — pasting Markdown and HTML

- **`crates/nodi-core/src/markdown_in.rs` (new).** `markdown_to_rtf(markdown,
  width_twips)` converts headings (`\fs40/\fs32/\fs28/\fs25`), bold/italic/
  strike/inline code, bullet and numbered lists (including nesting and the
  literal `◦`/`▪` prefixes used by the editor), fenced code blocks, pipe tables,
  links (web, `nodi-note:`, local paths as file URLs), block quotes, horizontal
  rules and paragraph indentation. `looks_like_markdown` counts Markdown
  signals and requires two (or a fence) so ordinary prose is not converted.
- **`crates/nodi-core/src/html_in.rs` (new).** `cf_html_to_markdown` honours the
  `StartFragment`/`EndFragment` header offsets and tolerates broken offsets by
  scanning for `<!--StartFragment-->`; `html_to_markdown` handles headings,
  paragraphs, `<strong>/<b>`, `<em>/<i>`, `<code>`, `<pre>` (with
  `language-*`), `<ul>/<ol>`, `<table>`, `<a href>` and `<hr>`, then hands the
  result to `markdown_to_rtf`.
- **`crates/nodi-windows/src/wnd/editkeys.rs`.** Paste precedence: native RTF →
  CF_HTML → Markdown heuristic → plain text. Ctrl+Shift+V is still plain text
  only. The HTML flavour is read as raw bytes
  (`image::clipboard_html`), because a .NET `DataObject` serialises `byte[]` as
  the string `System.Byte[]`.
- **`crates/nodi-windows/src/wnd/clipboard.rs`.** `set_text` for the code-block
  Copy link; CF_HTML is put on the clipboard with Win32 `SetClipboardData`.

### Section 2 — better code blocks

- `insert_code_block` now builds a one-column, two-row table: a header row with
  the language label (default `code`) and a `Copy` link (`nodi-copy:`), and a
  code row on the code background. `insert_output_block` does the same with a
  dark `#1E1E1E` background and the label `output`.
- Clicking the Copy link copies the code row as plain text
  (`whole_local_path`-style copy handling in `wnd/link.rs` +
  `clipboard::set_text`); the header is not included.
- The Markdown writer (`crates/nodi-core/src/markdown.rs`) reads the header
  label as the fence info string: `code` exports a bare fence, `output` exports
  ```` ```output ````; the header row itself is not exported as a table.
- Insert > Code Block keeps `Ctrl+Shift+C`; Insert > Output Block is new with
  `Ctrl+Shift+O` (`IDM_INSERT_OUTPUT_BLOCK = 208`).

### Section 3 — picture crop and Image Size

- `crates/nodi-core/src/rtf.rs`: `picture_crop` / `set_picture_crop` read and
  write `\piccropl/\piccropt/\piccropr/\piccropb` (units of 1/15 twip, i.e.
  `pixels * 15`) inside the picture group.
- `crates/nodi-core/src/image.rs`: `cropped_size` computes the visible pixel
  size from the natural size and the crop.
- `crates/nodi-windows/src/wnd/image.rs`: **Image > Crop...** (left/top/right/
  bottom in pixels with a Reset crop button, validated against the natural
  size) and a reworked **Image > Size...** with width/height in pixels *or*
  percent and a "lock aspect ratio" checkbox. `resize_percent` and `fit_width`
  keep using the same path. A crop is refused (with a message) when it would
  remove the whole picture or Rich Edit rejects the patched RTF.
- Crops are stored as RTF picture properties, so they survive save/restart
  without pixel-level rewriting (e2e AD asserts both the RTF values and the
  size after a restart).

### Section 4 — local paths, links and folder picking

- `crates/nodi-core/src/links.rs`:
  - `local_path_suffix` / `whole_local_path` recognise `C:\…`, `D:\…` and other
    drive paths, including quoted paths with spaces.
  - `path_file_url` / `file_url_path` convert between a path and a
    `file:///C:/…` URL (percent-encoding the characters Rich Edit would
    otherwise split on); `percent_decode_text` is used on display text.
  - New unit tests cover detection, quoting, encoding and decoding.
- **Why file URLs.** Rich Edit silently drops a `HYPERLINK` field whose target
  is a bare local path, so path links are stored as `file:///C:/…` with the raw
  path as the display text. `target_of`/`hyperlink_target` and the Markdown
  writer decode the URL back to `C:\…`, so `link-target` and the export both
  report the path.
- Auto-linking (`wnd/link.rs`) runs on space, Enter, a closing quote and paste.
  It skips tables and code blocks, ignores text when a quote is still open and
  never relinks text that is already inside a field. The caret after the
  conversion is computed from the document-length delta, because Rich Edit
  counts the hidden field instruction characters.
- `crates/nodi-windows/src/wnd/dialogs.rs`: `open_folder` uses
  `SHBrowseForFolderW` (`Win32_UI_Shell_Common` feature added to
  `crates/nodi-windows/Cargo.toml`). Insert > Link... has **Browse Folder...**
  next to Browse File; the chosen folder becomes a `file:///` link.
- The old "add a trailing space so text != target" workaround was removed
  entirely; it caused caret/raw-offset mismatches and is unnecessary now that
  the target is a URL.

### Section 5 — tree drag & drop and `Tree::move_relative`

- `crates/nodi-core/src/tree.rs`: `Position { Before, After, Inside }` and
  `Tree::move_relative(id, target, position)`.
  - `Before`/`After` place the node next to the target; `Inside` makes it the
    last child (same as `move_node(id, target, usize::MAX)`).
  - Trashed targets are treated as missing (`NotFound`); moving a node into its
    own subtree is `Cycle`.
  - The whole sibling strip, trashed siblings included, is renumbered densely
    to `0..n-1`, so no holes or duplicate orders are left behind.
  - A self-relative `Before`/`After` is a no-op.
  - Six unit tests: before/after/inside, cycle refusal, trashed target, trashed
    siblings and the self no-op.
- `crates/nodi-windows/src/wnd/tree.rs`: drag & drop rewritten.
  - Drop zones: top/bottom quarter of a row = `Before`/`After` with the
    insertion line (`TVM_SETINSERTMARK`); the middle half = `Inside` with the
    row highlighted (`TVGN_DROPHILITE`). Empty space below the tree = top level.
  - Auto-scroll near the top/bottom edge, 700 ms hover-expand of a collapsed
    node (`DRAG_TIMER`), Esc cancels, drops on the Trash are ignored.
  - Context menu: Move Up, Move Down, **Move to...** (`IDM_NOTES_MOVE_TO = 186`)
    and Move to Top Level, all sharing `App::move_node_relative`.
- `crates/nodi-windows/src/wnd/dialogs.rs`: the Move to... picker is a
  generalised `FindState` list (`move_to`) with "Top level" as the first entry
  and the clicked node preselected.
- `crates/nodi-windows/src/app.rs`: `move_node_relative` and
  `move_selected_to_picker` keep the selection, save the tree and rebuild the
  tree view.
- Test hooks: `move-rel <source> before|after|inside <target>` drives the same
  model call as a drag, plus `cancel-rename` for the folder-create flow.

### Docs and tests

- `docs/SHORTCUTS.md`: new paste rows, Output Block, Move to..., drag & drop
  zones/auto-scroll/hover-expand/Esc, path links, crop and Size in px/%.
- `docs/STATUS.md`: new test rows, 115-test count, feature and limitation
  updates.
- `scripts/e2e.ps1`: new cases Y, Z, AA, AB, AC, AD, AE, AF. AF now polls
  `tree.json` (`Wait-Tree`/`Root-Titles`) because the tree writer is
  asynchronous and a read straight after a move could still see the previous
  state (this showed up as one flaky failure during the final verification and
  is a test-only fix).

## Verification (verbatim)

Note: `cargo` writes its progress lines to stderr, which PowerShell shows as
`NativeCommandError`; the exit codes are 0.

### `cargo fmt --all --check`

```text
(no output; exit code 0)
```

### `cargo clippy --all-targets -- -D warnings`

```text
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.14s
```

### `cargo test`

```text
     Running unittests src\lib.rs (target\debug\deps\nodi_core-ee76c7db700215ef.exe)
running 72 tests
test result: ok. 72 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
     Running tests\autosave.rs (target\debug\deps\autosave-7fa3e4599844173e.exe)
running 6 tests
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
     Running tests\backup.rs (target\debug\deps\backup-1d137f52115ca5a1.exe)
running 6 tests
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
     Running tests\search_links.rs (target\debug\deps\search_links-f05d9db489843648.exe)
running 3 tests
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s
     Running tests\storage.rs (target\debug\deps\storage-cd05104a9d3b853a.exe)
running 10 tests
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
     Running tests\tree.rs (target\debug\deps\tree-e4e446c95299af6c.exe)
running 12 tests
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
     Running unittests src\main.rs (target\debug\deps\nodi-48e810ec38b17574.exe)
running 6 tests
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
   Doc-tests nodi_core
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

Total: **115 tests, 0 failed**.

### `pwsh -NoProfile -File scripts\e2e.ps1` (full run)

```text
A quick note survives restart without Ctrl+S                 PASS (Saved after 569 ms)
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
M many notes stay responsive                                 PASS (150 notes created in 4236 ms, restart to window 1289 ms)
O long session: switching tabs while typing loses nothing    PASS (60 edits across 3 tabs)
P Heading 4 and Normal via the format bar and markdown       PASS (combos 94x23+420+241 118x23+520+241 50x23+644+241)
Q tab stops are 360 twips (480 in code blocks)               PASS (body 360 twips, code 480 twips)
S Trash asks for confirmation, counts sub-notes, Empty Trash purges PASS 
R real keyboard: foreground window, Ctrl+A/B, Ctrl+N, F2 rename PASS 
T Markdown lists: typing, nesting, Backspace keeps indent    PASS 
U empty item ends the list, Ctrl+Z restores a typed marker   PASS 
V paragraph indentation survives wrapping and restart        PASS 
W Markdown headings: hidden marker, level changes, restart   PASS 
X separator: --- + Enter, Insert menu, restart and markdown  PASS 
Y real keys: heading colour, list Tab nesting, separator width PASS (heading bold=1 italic=0 underline=0 strike=0 face=Segoe UI size=320 hidden=0 colour=auto; sub start=360 offset=360 numbering=0 level=2 text="?\tsub poin"; separator 12269/12570 twips)
Z paste Markdown keeps headings, lists, code, table and links PASS 
AA paste HTML (CF_HTML) keeps structure                      PASS 
AB code block Copy link copies the code as plain text        PASS 
AC output block inserts, persists and exports ```output      PASS 
AD image crop survives save and restart                      PASS 
AE typed and pasted local paths become links                 PASS 
AF tree drag and drop before / after / inside                PASS 
--- 30 passed, 0 failed
```

Case D is intentionally absent from `e2e.ps1`; it is covered by G and by the
`cargo test` storage/RTF tests.

## New/updated e2e cases

| Case | What it checks |
|---|---|
| Y | Real `SendKeys`: `## ` heading body is `colour=auto`/black (not the marker grey), `{TAB}` inside a `- ` item makes a level-2 `◦` item with `start=360`, Shift+Tab returns to a level-1 bullet, `---` + Enter gives one picture at least 90 % of the editor width and an unindented paragraph after it |
| Z | Pasting Markdown text produces `# Judul`, `**tebal**`, `` `kode` ``, nested `- ` lists, a ```` ```rust ```` block that survives restart, a pipe table and a link |
| AA | Pasting a CF_HTML stream (real `StartFragment`/`EndFragment` offsets) keeps `<h2>`, bold, a link, a list and `<pre><code class="language-rust">` |
| AB | The code block's `Copy` link has target `nodi-copy:`; clicking it puts the code row (not the header) on the clipboard |
| AC | Ctrl+Shift+O inserts an output block, it survives restart and exports as ```` ```output ```` |
| AD | `crop 10 5 20 15` writes `\piccrop*` `150 75 300 225`, keeps the goal size and still shows the cropped size after save/restart |
| AE | Typed `C:\Windows` (space), quoted `C:\Program Files` (closing quote) and pasted `D:\Data\notes` become links; `link-target` is the raw path and the Markdown export has `[C:\\Windows](C:\Windows)` (and the encoded/plain variants as appropriate) |
| AF | `move-rel` before/after/inside, cycle refusal leaves the tree unchanged, Move Down / Move to Top Level, and everything survives a restart |

## Not done / follow-ups

- The drag & drop **visuals** (insertion line, drop highlight, auto-scroll,
  hover-expand) were verified from code and through the model-level hook
  `move-rel`; physically dragging with the mouse in a real session was not
  scripted, so the pixel-level look of the indicators is unverified by tests.
- The Move to... picker and the native common dialogs (Browse Folder, Crop,
  Size) were exercised only through their model/test-hook paths, not by
  clicking through every dialog control.
- Markdown paste covers the common subset (headings, emphasis, code, lists,
  tables, links, rules, quotes); inline HTML inside Markdown, footnotes,
  task-list checkboxes and reference-style links are not interpreted.
- HTML paste is a small tag-driven converter, not a full HTML parser
  (no CSS, nested tables or `<figure>`).
- Cropping is stored as RTF `\piccrop*`; the embedded image bytes are not
  rewritten, so Export/round-trips keep the full picture and only the display
  is cropped.
- A path link is stored as a `file:///` URL; other applications that paste the
  copied link will see that URL rather than the raw path.
