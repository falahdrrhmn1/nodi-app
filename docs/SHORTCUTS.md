# Nodi — Keyboard Shortcuts

Also available inside the app: **Help > Keyboard Shortcuts** (F1). The list
in the app comes from `SHORTCUTS` in `crates/nodi-windows/src/wnd/menu.rs`.

## Notes and tabs

| Keys | Action |
|---|---|
| Ctrl+N | New note (next to the selected one) |
| Ctrl+Shift+N | New child note |
| Ctrl+S | Save now (autosave runs anyway, ~0.5 s after you stop typing) |
| Ctrl+W / Ctrl+F4 | Close tab |
| Ctrl+Tab / Ctrl+PgDn | Next tab |
| Ctrl+Shift+Tab / Ctrl+PgUp | Previous tab |
| Ctrl+O | Go to note by title |
| F2 | Rename note |
| Del (in the tree) | Move note to Trash |
| Alt+Up / Alt+Down | Move note up / down among its siblings |
| Enter (in the tree) | Open the selected note |

## Editing (Notepad++ style)

| Keys | Action |
|---|---|
| Enter | New line with the same indentation and paragraph indent |
| Shift+Enter | Line break without indentation |
| Tab / Shift+Tab | Indent / outdent the paragraph (the selected lines when several are selected); in the middle of a line Tab still inserts a tab character |
| Backspace | At the start of a paragraph: remove a bullet (keeping the indentation), turn a heading back into text, or outdent one step |
| Ctrl+Z / Ctrl+Y | Undo / Redo |
| Ctrl+X / Ctrl+C / Ctrl+V | Cut / Copy / Paste (also pastes screenshots and copied pictures) |
| Ctrl+Shift+V | Paste as plain text |
| Ctrl+A | Select all |
| Ctrl+D | Duplicate line (or the selection) |
| Ctrl+G | Go to line |
| F5 | Insert date and time |

## Markdown-style typing

Lists, headings and separators can be typed the Markdown way and turn into
formatting as soon as the marker is finished (the raw marker is removed for
lists/separators; headings keep it and show it in grey while the caret is in
that heading).

| Type | Result |
|---|---|
| `- ` or `* ` at the start of a paragraph | Bulleted list item |
| `1. ` or `1) ` at the start of a paragraph | Numbered list item |
| `# ` … `#### ` at the start of a paragraph | Heading 1–4 |
| `---` alone on a line, then Enter | Horizontal separator |
| Enter in a list item | Next item (same level) |
| Enter on an empty list item | Ends the list, normal paragraph at the same indent |
| Tab / Shift+Tab in a list item | One bullet level deeper / shallower (• ◦ ▪) |
| Backspace at the start of a list item | Removes the bullet, keeps the indent |
| Ctrl+Z right after `- ` / `1. ` | Restores the typed marker as plain text |

## Formatting

The format bar (below the main toolbar) has **style** (Normal, Heading 1–4),
**font name** and **font size** (8–72 pt) lists that show the formatting at the
caret, plus text colour and highlight palettes, the four alignments, lists and
the insert commands.

| Keys | Action |
|---|---|
| Ctrl+B / Ctrl+I / Ctrl+U | Bold / Italic / Underline |
| Alt+Shift+5 | Strikethrough |
| Ctrl+` | Inline code |
| Ctrl+Shift+C | Code block (keeps indentation, Tab inserts a tab) |
| Ctrl+Shift+H | Highlight |
| Ctrl+] / Ctrl+[ | Bigger / smaller text |
| Ctrl+Alt+1 / 2 / 3 / 4 | Heading 1 / 2 / 3 / 4 (Enter after a heading continues with normal text) |
| Ctrl+Alt+0 | Normal text (also "Normal" in the style list) |
| Ctrl+Shift+8 / Ctrl+Shift+7 | Bulleted / numbered list |
| Ctrl+L / E / R / J | Align left / center / right / justify (also aligns pictures) |
| Ctrl+Space | Clear character formatting |

## Insert

| Keys | Action |
|---|---|
| Ctrl+K | Insert or edit link (web, file, or another note) |
| Ctrl+Click | Follow a link |
| Ctrl+Shift+I | Insert image from a file |
| Ctrl+Shift+T | Insert table |
| Alt+= | Insert formula (LaTeX) |
| Ctrl+Shift+- | Insert separator (horizontal rule) |
| Double-click a formula | Edit the formula |
| Double-click a picture | Open Image > Size... |

## Search and view

| Keys | Action |
|---|---|
| Ctrl+F | Find in note |
| F3 / Shift+F3 | Find next / previous |
| Ctrl+H | Replace |
| Ctrl+Shift+F | Search in all notes |
| Ctrl+= / Ctrl+- / Ctrl+0 | Zoom in / out / reset (or Ctrl+mouse wheel) |
| Ctrl+Shift+M | Show the note as Markdown (read-only) |
| F1 | Keyboard shortcut list |

## Mouse

- Right-click a note in the tree: New Child/Sibling Note, Rename, Change Icon,
  Duplicate, Move to Trash, Properties. Moving to the Trash asks first and
  mentions how many sub-notes go with it.
- **Trash (N)** sits at the bottom of the tree: click it (or press Enter) to
  open the Trash; right-click it for Open Trash and Empty Trash... (files are
  still kept in `data/recovery/`).
- Drag a note onto another note to make it a child; drop on empty space to move
  it to the top level.
- Right-click inside a note: clipboard commands plus picture (size, 25–100 %,
  fit to width, align, save), table (insert/delete rows and columns), link and
  formula commands when the click lands on one.
- A selected picture shows resize handles (Rich Edit's own); **Image > Size...**
  sets an exact size.
