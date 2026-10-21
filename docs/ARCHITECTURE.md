# Nodi Mark III — Architecture (binding spec)

Native Windows note app. Rust + Win32 + Rich Edit (`msftedit.dll`, class `RICHEDIT50W`).
NO Electron/Tauri/WebView/HTML/JS/localhost/npm. Ever.

## Toolchain
- `stable-x86_64-pc-windows-gnu` (pinned in `rust-toolchain.toml`). MSVC is not installed on this machine.
- `CARGO_HOME=E:\Software\Rust\cargo`, `RUSTUP_HOME=E:\Software\Rust\rustup`, target dir = `E:\nodi-app\mark-3\target`.
- Commands: `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test`, `cargo build --release`.

## Workspace layout
```
mark-3/
  Cargo.toml                 # [workspace] members = crates/*
  crates/nodi-core/          # pure Rust, NO windows-sys. Reusable for Android later.
    src/lib.rs
    src/id.rs                # NoteId: 128-bit random hex string (from time + counter + RtlGenRandom-free: use std hash of time/pid/counter). Never reused.
    src/tree.rs              # Tree model: nodes, kinds, move/reorder, cycle prevention, trash/restore
    src/storage.rs           # Data dir layout, atomic write, load/save tree + note content
    src/autosave.rs          # Writer thread + request/result types (no Win32; result delivered via callback)
    src/backup.rs            # Snapshots + retention + restore
    src/search.rs            # Title search, text search over plain-text sidecars
    src/links.rs             # nodi://note/<id> parse/format
    src/markdown.rs          # (Phase 5) export
  crates/nodi-windows/       # Win32 UI, binary name `nodi` (nodi.exe)
    src/main.rs              # entry, #![windows_subsystem = "windows"]
    src/app.rs               # App state (open tabs, active note, dirty flags)
    src/wnd/mod.rs           # main window proc, layout, splitter
    src/wnd/menu.rs          # menu bar + accelerators
    src/wnd/toolbar.rs
    src/wnd/tree.rs          # TreeView wrapper + context menus
    src/wnd/tabs.rs          # Tab control wrapper
    src/wnd/editor.rs        # Rich Edit wrapper: stream in/out RTF, formatting, EN_CHANGE
    src/wnd/status.rs        # Status bar
    src/wnd/dialogs.rs       # rename/insert-table/link/formula dialogs (DLGTEMPLATE in memory or simple popup windows)
    src/win.rs               # small safe helpers: wide strings, HWND wrappers. unsafe kept tiny + `// SAFETY:` comments
    build.rs / res/          # app icon + manifest (Common Controls v6, PerMonitorV2 DPI aware) via embedded .rc compiled with windres? -> use `embed-resource` only if windres from the GNU toolchain works; otherwise embed manifest via linker args.
```

## Dependencies (keep minimal)
- `windows-sys` (UI crate only).
- `serde` + `serde_json` allowed for metadata (core).
- Nothing else without a written reason in `docs/DEPENDENCIES.md`.

## Data layout (portable)
Data dir resolution order: env `NODI_DATA_DIR` → `<exe_dir>\data`. Never AppData. Dev script sets `NODI_DATA_DIR=E:\nodi-app\mark-3\data`.
```
data/
  tree.json            # all node metadata (id, kind, title, icon, parent, order, trashed, created, modified)
  session.json         # open tabs, active tab, cursor/scroll per tab, window placement, sidebar width
  notes/<id>.rtf       # note content (RTF from Rich Edit EM_STREAMOUT SF_RTF). Images/tables live inside RTF.
  notes/<id>.txt       # plain-text sidecar for search (written in same save transaction)
  notes/<id>.rtf.prev  # previous good version (1 generation)
  attachments/<id>/    # source files kept for re-edit (e.g. formula source, original images)
  backups/<yyyymmdd-hhmmss>/   # full snapshot copies, retention: keep last 20
  logs/nodi.log        # append-only, small, rotate at 1 MB
```

## Save pipeline (data-loss is priority #1)
1. EN_CHANGE → mark tab dirty, status `Modified`, (re)start debounce timer (SetTimer, 500 ms).
2. Timer fires OR flush trigger (tab switch, note switch, close tab, WM_CLOSE, WM_QUERYENDSESSION/WM_ENDSESSION, Ctrl+S):
   UI thread streams RTF + plain text out of Rich Edit (fast, in-memory), increments `seq`, sends `SaveRequest{id, seq, rtf, txt}` to the single writer thread (std mpsc). Status `Saving...`.
3. Writer thread: write `<file>.tmp` → `sync_all()` → copy current to `.prev` → `std::fs::rename(tmp, file)` (MoveFileEx REPLACE_EXISTING) → result.
4. Writer posts `WM_APP_SAVE_RESULT` (PostMessageW with boxed result) to main window. UI marks `Saved` ONLY if result Ok AND `seq` == latest seq for that note; else stays Modified/Saving. Error → `Save Failed` + MessageBox once per failure streak; content stays in memory; retry on next change or every 5 s.
5. Shutdown flush: synchronous — send all pending, then wait on writer with timeout (5 s) before exiting. WM_ENDSESSION same.
6. tree.json/session.json use the same atomic write path.
7. Startup recovery: if `<id>.rtf` fails to load or is empty while `.prev` exists → offer `.prev`. Leftover `.tmp` files → keep as `recovered-<id>-<ts>.rtf` in `data/recovery/`, never silently delete.

## UI (Notepad++-style, classic, light)
Menu: File Edit Search View Insert Format Notes Tools Help. Toolbar (TB_ standard small icons/text). Left: TreeView (resizable splitter). Right: Tab control above one Rich Edit per open tab (hide/show on switch, preserves undo stack/caret). Bottom: status bar parts: `Ready` | save state | `Ln x, Col y` | `UTF-8` | `Rich Text`.
Menu items that are not implemented must NOT be shown (or shown grayed with "(not yet)").

## Rich features plan (Phase 4)
- Images: `EM_INSERTIMAGE` (RICHEDIT_IMAGE_PARAMETERS, Win8+) from IStream (SHCreateMemStream). Persist via RTF `\pict`.
- Tables: `EM_INSERTTABLE` (TABLEROWPARMS/TABLECELLPARMS). Row/col ops via EM_GETTABLEPARMS/EM_SETTABLEPARMS.
- Links: CFE_LINK + `EM_SETTEXTEX` with RTF `{\field{\*\fldinst HYPERLINK "url"}{\fldrslt text}}`, EN_LINK handling. Internal = `nodi://note/<id>`.
- Formula: research in Phase 4 (RichEdit math zones vs render-to-image with LaTeX source kept in attachments).
