# Rev2 Batch 2 — writing behaviour (lists, indentation, Markdown-style typing)

Same rules/build env as `docs/tasks/rev2-batch1.md` (CARGO_BUILD_JOBS=1, debug builds, no commits, never delete files). Read `docs/ARCHITECTURE.md` "Rich Edit findings" first: Enter is handled on WM_KEYDOWN, Tab on WM_CHAR; tabs inside table cells must go in as RTF; use `editor::raw_text` (GT_RAWTEXT) for positions.

## 1. Lists like Word/Obsidian
- Typing `- ` or `* ` at the start of a paragraph turns it into a **bulleted list item** (remove the typed marker, apply native Rich Edit bullets). `1. ` / `1) ` → numbered list. Undo (Ctrl+Z) right after the conversion restores the typed text.
- In a list item: **Enter** = new item (native); **Enter on an empty item** = end the list (normal paragraph at the same indent). **Tab** (caret anywhere in the item) = one level deeper (sub-point: bigger start indent, different bullet glyph per level e.g. • ◦ ▪); **Shift+Tab** = one level up; **Backspace at the start of an item** = remove the bullet but **keep the indentation** (paragraph stays indented, text aligned where it was).
- Wrapped lines of a list item align with the item text (hanging indent), not with the bullet — see the user's screenshot problem.

## 2. Indentation that survives wrapping
The user indents normal text with Tab and long lines wrap back to the left margin, which looks broken. Change Tab/Shift+Tab/Enter for **normal paragraphs** (not code blocks, not tables):
- Tab with the caret at the start of a paragraph (or inside its leading indentation), or with a multi-line selection → increase the **paragraph start indent** by one step (360 twips, the same step as the new tab width) instead of inserting a tab character. Shift+Tab → decrease. Backspace at the very start of an indented paragraph → decrease one step.
- Enter keeps the paragraph indent (Rich Edit inherits paragraph format; make sure our auto-indent code does not add tab characters on top).
- Tab in the middle of a line still inserts a tab character. Existing notes with leading tab characters keep working (do not rewrite them).
- Wrapped lines now align with the indented text.
- Markdown export: indent levels → leading spaces/nested lists as appropriate.

## 3. Markdown-style headings with a visible `#` while editing (Obsidian live preview)
- Typing `# `, `## `, `### `, `#### ` at the start of a paragraph makes it Heading 1–4.
- The heading keeps a marker prefix `# `/`## `… in the text: **visible (grey) while the caret is in that heading paragraph, hidden (CFE_HIDDEN) when the caret is elsewhere.** Toggle on EN_SELCHANGE for the paragraph being left/entered only (cheap). Deleting the `#`s returns the paragraph to normal text; adding/removing a `#` changes the level.
- Headings made from the style combobox/shortcuts get the same prefix so behaviour is consistent.
- Plain text/search sidecar and Markdown export must not duplicate the prefix (`# Title`, not `# # Title`); `text` hook / Find should still work.
- Enter at the end of a heading → normal paragraph (already exists; keep).

## 4. Separator `---`
- Typing `---` alone on a line and pressing Enter replaces it with a **horizontal separator**: a full-width thick grey line (like a page break between sections). Implement as a picture (generated PNG bar, editor width) wrapped in a `HYPERLINK "nodi-hr:"` field like formulas, so it survives reload; Markdown export writes `---`; Insert menu gets "Separator" (Ctrl+Shift+-).

## 5. Small fix from batch 1
The style combobox is too narrow ("Heading 2" is cut off as "Heading :"); size it to its longest entry (DPI-aware).

## Tests
Add e2e cases (real `enter`/`tab` hooks send KEYDOWN/CHAR like a keyboard): `- ` → bullet; Enter → second item; Tab → level 2; Shift+Tab → level 1; Backspace at start → no bullet but indent kept; Enter on empty item ends list; indented paragraph wraps aligned (check paragraph `dx_start_indent` via a hook); `## ` → H2 with prefix visible when caret inside, hidden after moving away (check CFE_HIDDEN via hook), survives restart; Markdown export of all of these; `---`+Enter → separator, restart, export `---`. All previous e2e (A–O, R, batch 1 cases) still pass; fmt/clippy/test clean. Update docs/SHORTCUTS.md + STATUS.md; write `docs/tasks/rev2-batch2-REPORT.md`.
