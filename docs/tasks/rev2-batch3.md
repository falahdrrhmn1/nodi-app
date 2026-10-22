# Rev2 Batch 3 — paste Markdown/HTML, better code blocks, crop, folder links, tree drag & drop

Same rules/build env as `docs/tasks/rev2-batch1.md`. Read `docs/ARCHITECTURE.md` (Rich Edit findings, link safety policy in `wnd/link.rs`) first.

## 0. Fix batch 2 bugs first (found by the orchestrator with REAL keyboard input)
Reproduce with real keys (SendKeys after the foreground + `{ESC}` trick used in e2e test R), typing `## Bagian satu{ENTER}`, `- long text...{ENTER}`, `{TAB}sub poin{ENTER}`, `+{TAB}poin kedua{ENTER}{ENTER}`, `---{ENTER}Teks`:
1. The heading text renders **grey**; only the `## ` marker may be grey (and hidden when the caret leaves). The heading itself must use the normal text colour.
2. **Tab in a list item removes the bullet** instead of making a level-2 item (◦). Shift+Tab must go back to level 1 (•) with the bullet kept.
3. The `---` separator is only half the editor width and inherits the list indent. It must span the full editor width at indent 0, and the paragraph after it must be a normal paragraph at indent 0.
Add an e2e test **Y** that types exactly this sequence with real keys (skip as NOT TESTED only if the foreground cannot be obtained) and checks: heading colour = auto/black (CFE_AUTOCOLOR or cr_text_color black) for the heading text, `numbering` and level/indent of the sub-item via a hook, separator width ≥ 90 % of the editor client width, indent 0 after it. The existing hook-based tests passed while these bugs were visible, so the real-key test is mandatory.

## 1. Paste from ChatGPT / websites keeps the formatting
- New `nodi-core` module `markdown_in.rs`: Markdown → RTF (the RTF Nodi itself uses: Segoe UI body, headings sizes from `format::HEADING_SIZES`, bullets/numbered/nested lists, bold/italic/strike/inline code, links (`HYPERLINK` fields), fenced code → the one-cell code-block table, GFM tables → real tables, `---` → separator if batch 2 added one, `$...$`/`$$...$$` → formula if feasible (render via the existing formula code, else keep as text), images `![alt](url)` → a link (no network access ever). Unit tests for each construct.
- New `nodi-core` module `html_in.rs`: clipboard "HTML Format" (CF_HTML: parse StartFragment/EndFragment offsets) → Markdown (h1–h4, p, br, b/strong, i/em, s/del, code, pre, a, ul/ol/li nested, table/tr/td/th, img→link, blockquote → indented paragraph) then reuse `markdown_in`. No external crates; a small tolerant tag scanner is enough. Unit tests with real-world snippets (ChatGPT web copy, a Wikipedia paragraph).
- Paste rules (editor subclass, Ctrl+V / Shift+Insert / menu Paste): clipboard has RTF → native (unchanged); else has HTML Format → html_in; else plain text that *looks like Markdown* (heuristic: heading `#`, list markers, fences, `**`, `[x](y)`, `|` tables — at least two signals or a fence) → markdown_in; else native. Ctrl+Shift+V stays plain text. Edit menu: "Paste Markdown as Plain Text" not needed — Ctrl+Shift+V covers it.

## 2. Code blocks look and work better
- Visual: slightly darker header strip with a language label (optional, from the fence info string or "code") and a **Copy** link at the top-right of the block (a `nodi-copy:` link field; Ctrl+Click or plain click copies the block's code text to the clipboard as plain text and shows "Code copied" in the status bar). Use `Draw` / table features Rich Edit actually renders (two-row table: header row + code row is fine). Keep Enter/Tab behaviour inside the code cell.
- **Output block** variant: Insert > Output Block (Ctrl+Shift+O) — same structure, dark background (#1e1e1e) with light monospace text, label "output". Markdown export: ```` ```output ````.
- Markdown export of a code block uses the language label as the fence info string.

## 3. Pictures: crop and exact numbers
- Image > Crop...: dialog with left/top/right/bottom in pixels (and a "Reset crop"). Implement with RTF `\piccropl/t/r/b` (twips) on the selected picture group if Rich Edit honours it (test it first and screenshot); if it does not, crop the pixels: decode PNG via a small inflate (miniz_oxide is already a dependency) / BMP directly, JPEG via WIC (`IWICImagingFactory` through raw vtables like `image::release`) — re-encode PNG and replace the picture keeping its display scale. Report which approach works.
- Image Size dialog: add width/height % fields and a "Lock aspect ratio" checkbox (default on).

## 4. Links to folders and files
- Link dialog: add **Browse Folder...** (IFileOpenDialog with FOS_PICKFOLDERS, or SHBrowseForFolderW) next to Browse File.
- Typing or pasting a full local path (`C:\...`, `D:\...` with at least one backslash, until whitespace/end, or quoted) auto-converts to a link whose target is that path (display text = the path). Following it uses the existing safety policy (folders open in Explorer after confirmation, documents per allowlist).

## 5. Notes tree: free ordering with a visible drop indicator
- Dragging a node shows where it will land: **insertion line above / below** the item under the cursor (top/bottom quarter of the row → before/after, middle half → **inside** as last child, item highlighted with TVGN_DROPHILITE). Use `TVM_SETINSERTMARK` for the line. Works for notes and folders alike; any node can be reordered among siblings or moved into any other node (cycle check already in core). Esc cancels the drag. Auto-scroll when dragging near the top/bottom edge; hovering a collapsed node for ~700 ms expands it.
- Context menu: **Move to...** opens a picker (tree of possible parents + "Top level") and moves the node as last child; keep Move Up/Down/Top Level.
- Core: add `Tree::move_relative(id, target, Position::{Before, After, Inside})` with tests (cycles, trashed siblings, order density).

## Tests
Unit tests for markdown_in, html_in, move_relative, crop math. e2e: paste Markdown (put text on the clipboard from PowerShell `Set-Clipboard`, then hook `cmd` paste) → headings/list/code/table/link present (check via `markdown` hook round trip); paste HTML (set CF_HTML via a small C# helper) → same; code block Copy link puts code on the clipboard (`Get-Clipboard`); output block exports ```` ```output ````; crop changes the picture and survives restart; folder link target recognised; drag & drop before/after/inside via hook commands (`move-rel <title> before|after|inside <title>`) + tree.json check. All previous e2e still pass (incl. R real keyboard); fmt/clippy/test clean. Update SHORTCUTS/STATUS; write `docs/tasks/rev2-batch3-REPORT.md`.
