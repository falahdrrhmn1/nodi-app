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
| Enter | New line **with the same indentation** as the current line |
| Shift+Enter | Line break without indentation |
| Tab / Shift+Tab | Indent / outdent — the whole selected lines when several are selected |
| Backspace | Removes one indent level when the indent is a tab |
| Ctrl+Z / Ctrl+Y | Undo / Redo |
| Ctrl+X / Ctrl+C / Ctrl+V | Cut / Copy / Paste (also pastes screenshots and copied pictures) |
| Ctrl+Shift+V | Paste as plain text |
| Ctrl+A | Select all |
| Ctrl+D | Duplicate line (or the selection) |
| Ctrl+G | Go to line |
| F5 | Insert date and time |

## Formatting

| Keys | Action |
|---|---|
| Ctrl+B / Ctrl+I / Ctrl+U | Bold / Italic / Underline |
| Alt+Shift+5 | Strikethrough |
| Ctrl+` | Inline code |
| Ctrl+Shift+C | Code block (keeps indentation, Tab inserts a tab) |
| Ctrl+Shift+H | Highlight |
| Ctrl+] / Ctrl+[ | Bigger / smaller text |
| Ctrl+Alt+1 / 2 / 3 | Heading 1 / 2 / 3 (Enter after a heading continues with normal text) |
| Ctrl+Alt+0 | Normal text |
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
| Double-click a formula | Edit the formula |

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
  Duplicate, Move to Trash, Properties.
- Drag a note onto another note to make it a child; drop on empty space to move
  it to the top level.
- Right-click inside a note: clipboard commands plus picture (size, 25–100 %,
  fit to width, align, save), table (insert/delete rows and columns), link and
  formula commands when the click lands on one.
- A selected picture shows resize handles (Rich Edit's own); **Image > Size...**
  sets an exact size.
