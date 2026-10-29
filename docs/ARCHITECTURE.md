# Nodi Mark III — Architecture

Native Windows note app: **Rust + Win32 + Microsoft Rich Edit** (`msftedit.dll`,
class `RICHEDIT50W`). No Electron, Tauri, WebView, HTML/JS, localhost or npm.

## Crates

```
crates/nodi-core      pure Rust, no Win32 — reusable for a future Android app
  id.rs               NoteId (32 hex chars, never reused)
  tree.rs             notes tree: folders and notes (notes may have children),
                      move with cycle checks, sibling order, trash/restore/purge
  storage.rs          data-dir layout, atomic writes, note/tree/session files,
                      leftover recovery
  autosave.rs         single writer thread: coalesces saves per note, reports
                      results back (the UI shows Saved only after an Ok)
  backup.rs           timestamped snapshots (never overwritten), prune, restore
  search.rs           title search, full-text search over .txt sidecars
  links.rs            nodi://note/<id> and nodi-math:<LaTeX> links
  image.rs            PNG/JPEG/GIF/BMP header sniffing, clipboard DIB → BMP
  rtf.rs              targeted RTF patches (picture size, picture blob)
  markdown.rs         note RTF → Markdown (+ list of what could not be kept)
  math/parse.rs       LaTeX subset → syntax tree (with error positions)
  math/layout.rs      TeX-like box layout; font metrics injected via a trait
  math/mod.rs         grey+alpha PNG encoder (miniz_oxide)

crates/nodi-windows   Win32 shell, binary nodi.exe
  main.rs, app.rs     App state: tabs, save state machine, session, commands
  testhook.rs         WM_COPYDATA remote control for scripts/e2e.ps1
                      (only when NODI_TEST_HOOKS=1 at startup)
  wnd/mod.rs          frame window, layout, splitter, message loop
  wnd/menu.rs         menus, accelerators, SHORTCUTS list, command dispatch
  wnd/toolbar.rs      toolbar + tooltips
  wnd/tree.rs         Notes Explorer TreeView, icons, drag & drop, context menu
  wnd/tabs.rs         tab control
  wnd/status.rs       status bar (message | save state | Ln/Col | UTF-8 | mode)
  wnd/editor.rs       Rich Edit wrapper (streams, raw text, formats, zoom…)
  wnd/editkeys.rs     editor subclass: auto-indent, Tab/Shift+Tab, paste
  wnd/format.rs       headings, lists, alignment, code, highlight, colour, font
  wnd/table.rs        tables: insert, rows, columns
  wnd/image.rs        pictures: insert, paste, resize, save
  wnd/formula.rs      formula rendering (GDI, Cambria Math) and editing
  wnd/link.rs         hyperlinks, link safety policy
  wnd/find.rs         Find/Replace (common dialogs), search all notes
  wnd/context.rs      right-click menu inside a note
  wnd/form.rs         small DPI-aware modal form used by several dialogs
  wnd/dialogs.rs      message boxes, icon picker, trash, note picker, file dialogs
  wnd/clipboard.rs    clipboard writing
  wnd/editword.rs     Ctrl+Backspace / Ctrl+Delete in every Edit control
  wnd/gutter.rs       line-number strip (paragraph numbers, cached starts)
  wnd/pattern.rs      background lines / grid / dots (key-colour MaskBlt)
  wnd/net.rs          WinHTTP download for pictures in pasted web pages
  explorer.rs         multi-selection, lock, sort, refresh, import source
  prefs.rs            Tools > Preferences
  themes.rs           theme presets menu, Theme Editor, copy/paste themes
```

## Data (portable, never AppData)

Resolution: `NODI_DATA_DIR` → `<exe dir>\data`, except an exe in a Cargo
`target\<profile>` folder uses `<workspace>\data` (so `cargo clean` cannot
delete notes).

```
data/tree.json            node metadata
data/session.json         open tabs, caret/scroll, window, sidebar, zoom, wrap
data/notes/<id>.rtf       content (formatting, pictures, tables, links inside)
data/notes/<id>.txt       plain-text sidecar for search
data/notes/<id>.rtf.prev  previous good version
data/backups/<ts>/        snapshots (daily at startup + Tools > Backup Now), keep 20
data/recovery/            leftover .tmp files after a crash, purged notes
data/settings.json        theme (light/dark/custom colours), spell check, mirror on/off
data/icons/               note icons chosen from files
data/markdown/            Markdown mirror for AI agents (derived, never read back)
  INDEX.md, index.json    outline + machine-readable list of every note
  <folders>/<title>.md    front matter (id, title, parent, path, dates) + Markdown
  .trash/<ts>/            mirror files of renamed/moved/trashed notes
```

The Markdown mirror is written by the writer thread after the writer has been
idle for 300 ms (and once on shutdown), so it never delays `Saved` or exit:
saved notes are converted from the RTF just written, and a tree change only
re-plans paths (`mirror::sync` converts just the notes whose place changed).
At startup a mirror file newer than its note is trusted as it is.

## Save pipeline (no data loss is priority #1)

1. Typing → `Modified`, 500 ms debounce timer.
2. Timer / tab switch / closing a tab / Ctrl+S / closing the app /
   `WM_QUERYENDSESSION` → the UI thread streams RTF + plain text out of Rich
   Edit (in memory) and hands them to the writer thread → `Saving...`.
3. Writer: `<file>.tmp` → `sync_all` → old file copied to `.prev` → rename.
4. The result is posted back; the tab shows `Saved` only when that exact save
   succeeded. Failure → `Save Failed`, one message box per failure streak,
   text stays in the editor, retry every 5 s.
5. Replacing a file retries for about a second while another program
   holds it open (scanners, sync clients, readers); a `tree.json` /
   `session.json` write that still fails stays queued and is retried every
   second instead of waiting for the next change. Found by the save torture
   test: a reader polling `tree.json` made a move vanish from disk.
6. Closing flushes everything and waits for the writer; unsaved notes prompt
   "Close anyway?". The session file and new-note files also go through the
   writer, so the UI thread never waits for the disk.

## Rich Edit findings that shaped the code

- `richedit.h` uses `#pragma pack(4)`: `EDITSTREAM`, `GETTEXTEX`, `ENLINK`,
  `TABLEROWPARMS` are declared `packed(4)`; natural alignment made
  `EM_STREAMOUT` call a garbage pointer and hang.
- `EM_GETTEXTRANGE` drops table delimiters (U+FFF9/U+FFFB/U+0007) and shifts
  positions; `EM_GETTEXTEX` with `GT_RAWTEXT` keeps positions aligned.
- Pictures report as U+FFFC in raw text and as a space in plain text.
- Paragraph shading/borders are not rendered; table cell shading is — a code
  block is therefore a one-cell grey, bordered Consolas table.
- Enter is handled by Rich Edit on `WM_KEYDOWN` (the `WM_CHAR` CR is ignored);
  Tab on `WM_CHAR`. Auto-indent hooks Enter at `WM_KEYDOWN`.
- In a table cell, a tab inserted through `EM_REPLACESEL` acts like the Tab key
  (next cell / new row); code blocks insert tabs as RTF `\tab`.
- `EM_REPLACESEL("")` on a middle table row leaves a paragraph that splits the
  table; `WM_CLEAR` deletes rows cleanly.
- `EM_INSERTTABLE` leaves the caret after the table (not in the first cell).
- Rich Edit re-encodes inserted pictures and drops custom PNG chunks and the
  picture description on reload, but keeps hyperlink targets. Formulas are
  therefore stored as a picture inside a `HYPERLINK "nodi-math:<LaTeX>"` field.
- Without an OLE callback Rich Edit only pastes text/RTF; clipboard pictures
  (PNG, DIB) are inserted by Nodi.
- Automatic text colour is always painted with the system window-text colour
  (black); `EM_SETBKGNDCOLOR` only changes the background. Dark themes
  therefore *display* automatic and pure-black text through two near-white
  colour-table entries: `editor::set_selection_format` / `insert_rtf` /
  `apply_default_colour` map to them, and `stream_out_rtf` / `selection_rtf`
  fold them back, so every consumer (autosave, Markdown, mirror, links) sees
  the stored colours. Text without any `\cf` gets an explicit one when themed.
- Rich Edit forces `\ul` and the link colour on a field result, so the
  separator is a plain, very flat picture recognised by its shape
  (`image::is_rule_picture`), not a `nodi-hr:` link (older notes still work).

## Lists (batch 6)

Rich Edit numbers every run of consecutive numbered paragraphs 1, 2, 3…
whatever their indent, and its RTF does not keep a restart at 1. Nodi
therefore numbers like an outline itself (`format::renumber`): a list
continues at its own indent across deeper items and indented continuation
paragraphs, a deeper list starts at 1, plain text or a bullet at the same
indent ends it; items that do not simply follow the paragraph before get
`PFNS_NEWNUMBER` + an explicit start. A user's "Restart Numbering" is stored
as a number-to-text distance one twip larger than normal (`\pnindent361`),
which Rich Edit keeps. `editor::load_rtf` renumbers every loaded note that has
numbered paragraphs. Deeper bullets and checklist boxes (`☐`/`☑`) are a
literal glyph + tab with a hanging indent.

## Themes (batch 6)

A theme is a full colour set (`ThemeSpec`): note, window, tree, tabs,
accent, borders, line numbers, background pattern, plus the chrome font.
The menu bar is painted through the `WM_UAHDRAWMENU*` messages (as Notepad++
does), pop-up menus use uxtheme's `SetPreferredAppMode`, the title bar DWM's
dark-mode attribute. The note's own colours never change (see the display
sentinels above).

## Markdown

Visual editing is the source of truth. Markdown is generated on demand
(View > Markdown Source, File > Export Note as Markdown, the AI mirror) and
never written back into the note, so nothing is lost silently. Features
Markdown cannot express are listed in the export dialog. Markdown → RTF
(`markdown_in`, `html_in`) is used for pasting from ChatGPT and web pages.

## Android later

`nodi-core` has no Windows code: tree, storage format, autosave writer,
backups, search, links, formula parser/layout and the Markdown converter can
be reused. An Android UI would need its own editor and an RTF subset reader or
a migration of the content format.
