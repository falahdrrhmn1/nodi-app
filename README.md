# Nodi App

Nodi is a fast, **native Windows** note-taking app: Rust + Win32 + Microsoft
Rich Edit. No Electron, Tauri, WebView, browser, Node.js or local server.

The window follows Notepad++ (classic menus, small toolbar, tabs); the Notes
Explorer follows KeepNote (a note can contain other notes). The editor is
WYSIWYG — Ctrl+B makes text bold instead of inserting `**text**`.

## Features

- **Notes Explorer**: folders, notes with child notes, drag & drop with an
  insertion line, Trash with restore, 8 built-in icons, Move to…, Go to Note by
  title, internal note links that survive renames.
- **Editor**: bold/italic/underline/strikethrough, headings 1–4, bulleted and
  numbered lists, alignment, font, size, colour, highlight, inline code, real
  tables, pictures (insert, paste a screenshot, resize, crop), formulas
  (LaTeX rendered in place and editable again), undo/redo, zoom.
- **Markdown-style typing**: `- `, `1. `, `[ ] `, `# `…`#### `, `---` +
  Enter (separator), `----` + Enter (page break); Tab indents the paragraph or
  nests a list item at any depth, Enter keeps the indentation. Numbered lists
  number like an outline (a deeper list starts at 1, Shift+Tab continues the
  outer numbers), with Restart / Continue Numbering. **Checklists** with boxes
  you click.
- **Notes Explorer**: Ctrl+click / Shift+click several notes and folders and
  trash, move, export or lock them together; **Lock** keeps a note or folder
  read-only (no edit, rename, move or trash) until unlocked; **sort** by your
  own order, name (`2` before `10`, `00-…` first), date created or modified;
  F5 refreshes; Trash stays the last row.
- **Find Notes** (Ctrl+Shift+F) searches note and folder titles and the text
  of every note, with the folder path and a bit of text around each hit.
- **Tabs** with a × close button and a + that opens a new note (**Ctrl+T**).
  A new note is titled by its first line until you rename it.
- **Ctrl+Z in the Notes Explorer** undoes a move to the Trash, a move or a
  rename (it asks first).
- **Paste from ChatGPT or a web page** keeps headings, lists, task lists,
  tables, code, links and pictures (embedded ones, local files and web
  pictures, downloaded through Windows); `$…$`, `$$…$$`, `\(…\)`, `\[…\]`
  and KaTeX/MathJax/Wikipedia formulas become rendered formulas. Terminal
  output with a drawn table (`pm2 ls`, `docker ps`, MySQL) becomes a real table.
- **Import** (File > Import, right-click > Import Files Here, or drag files
  onto the window): `.md`, `.txt` (any encoding) and `.rtf` become notes,
  folders keep their structure. Import **copies**: the original files are
  never changed. Each imported note remembers its file (right-click > Show
  Original File); right-click > Save Back to Original File writes your edits
  into it, after keeping a copy of the old file in `data\recovery\originals`.
  Removing an import (Move to Trash) never touches the files on disk.
- **Export / Save As** (File > Export, or right-click): this note as `.md`,
  `.rtf` or `.txt` (Ctrl+Shift+S); the selected notes and folders, or all notes,
  as `.md` / `.rtf` / `.txt` files in folders like the tree; or a **complete
  backup copy** that Tools > Restore From Backup Folder brings back.
- **Autosave** with atomic writes, a `.prev` generation, crash recovery and
  honest status (`Modified` / `Saving…` / `Saved` / `Save Failed`); the note is
  flushed on tab switch, on close and on a Windows shutdown.
- Daily backups (20 kept) plus **Tools > Backup Now** and Restore Backup…
- Sessions: open tabs, caret, scroll position, window and sidebar size come back
  after a restart.
- Search and replace, search in all notes, read-only Markdown view, export a
  note as Markdown with its pictures.
- **Themes**: 15 built in (Light, Paper, GitHub Light, Solarized Light/Dark,
  VS Code Dark+ / Dark Modern, Monokai, Dracula, One Dark, Nord, Gruvbox Dark,
  Tokyo Night, Catppuccin Mocha, High Contrast). The title bar, menus,
  toolbars, tree, tabs and status bar follow the theme. The Theme Editor sets
  every colour and the chrome font, previews live and saves **your own
  themes**; copy a theme as text and paste it on another PC.
- **View**: line numbers next to the text (as in Notepad++), a background
  pattern (lines, grid, dots — drawn on screen only), sort order.
- **Tools > Preferences**: default font and size for new notes, sounds (off
  by default: no beeps, silent message boxes), two quick spaces → `. `.
- Lines, words and characters (and the selection's) in the status bar;
  Ctrl+Up/Down scroll the view.
- **AI Markdown mirror** — see below.
- Every shortcut: [`docs/SHORTCUTS.md`](docs/SHORTCUTS.md), or **F1** in the app.

## Run (portable)

```powershell
.\dist\Nodi\nodi.exe
```

Double-click `nodi.exe` or make a shortcut to it. Everything lives next to the
exe, so `dist\Nodi` can be moved to another drive or folder as a whole; the
notes are in `dist\Nodi\data`.

A build from `target\debug` or `target\release` keeps its data in
`E:\nodi-app\mark-3\data` (not inside `target`, so `cargo clean` cannot delete
notes). `NODI_DATA_DIR` forces another folder.

## Build

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build.ps1
```

Result: `dist\Nodi\nodi.exe` (single file — no DLLs to ship). During
development `cargo run -p nodi-windows` builds and starts the debug exe.

Prerequisites:

- Rust in `E:\Software\Rust` (`CARGO_HOME` and `RUSTUP_HOME` are set by the
  build script).
- The `stable-x86_64-pc-windows-gnu` toolchain, pinned in `rust-toolchain.toml`;
  the MinGW linker comes with it, so Visual Studio Build Tools are **not**
  needed.
- `windres` for the icon and the manifest: looked up on `PATH`, then
  `E:\Software\C++ Compiler\bin\windres.exe`.

## Data

```
data/
  tree.json            the tree (id, title, icon, parent, order, trash)
  session.json         open tabs, caret, window and sidebar size
  settings.json        theme, AI mirror, spell check
  notes/<id>.rtf       the note itself (formatting, pictures and tables inside)
  notes/<id>.txt       plain text, used by the search
  notes/<id>.rtf.prev  the previous version (one generation)
  markdown/            AI mirror of every note (see below)
  backups/<time>/      automatic daily snapshots, the last 20 are kept
  recovery/            files left by a crash, and permanently deleted notes
```

Every write is atomic: `*.tmp` → flush → the old version becomes `.prev` →
rename. The status bar shows **Saved** only after the write really succeeded;
on a failure it shows **Save Failed**, the text stays in the editor and Nodi
retries every 5 seconds.

## AI agents: the Markdown mirror

Nodi mirrors the whole library as plain Markdown in `data\markdown` (with front
matter, plus `index.json`). Point an agent at that folder and let it start with
`INDEX.md`; the files are rewritten after every save. Edits made in the mirror
are **not** read back into Nodi — the app's own data stays the source of truth.
Tools > Rebuild AI Markdown Mirror regenerates it on demand.

- **Tools > AI Mirror Folder…** puts the mirror anywhere on a local drive (for
  example `E:\catatan-ai`). A folder that already holds other files gets a
  `Nodi AI` sub-folder instead, so nothing of yours is ever moved.
- **Tools > Copy AI Folder Path** copies the path to give to an agent;
  **Open AI Folder** shows it; **Help > Using Nodi with AI Agents** explains it.

## Tests

```powershell
cargo test                              # unit + integration tests (nodi-core)
pwsh -File scripts\e2e.ps1              # end-to-end against the real nodi.exe
pwsh -File scripts\persist-torture.ps1  # save-reliability torture (restarts + kills)
pwsh -File scripts\stress-heavy.ps1     # big note, killed while saving (-HogMB: low memory)
pwsh -File scripts\smoke.ps1            # startup time, RAM, screenshot
```

`docs/STATUS.md` lists what is done, what is measured and what is still
missing. Code layout: `crates/nodi-core` (pure Rust: tree, storage, autosave,
backup, Markdown, search, links, formulas) and `crates/nodi-windows` (the Win32
UI).
