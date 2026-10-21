# Task 1b — nodi-windows native shell (Phase 1)

Read `docs/ARCHITECTURE.md` first. It is binding. Work ONLY inside `crates/nodi-windows/`. Another agent is building `crates/nodi-core` in parallel — do NOT edit it and do NOT depend on it yet (Phase 2 wires them). Do not touch files outside `E:\nodi-app\mark-3`.

## Build env (PowerShell, set before every cargo call)
```
$env:CARGO_HOME='E:\Software\Rust\cargo'; $env:RUSTUP_HOME='E:\Software\Rust\rustup'; $env:PATH="E:\Software\Rust\cargo\bin;$env:PATH"
```
Toolchain is `stable-x86_64-pc-windows-gnu` (MinGW linker bundled). Use `cargo build -p nodi-windows`, `cargo clippy -p nodi-windows --all-targets`, `cargo fmt -p nodi-windows`.

## Stack
Pure Win32 via `windows-sys` (latest 0.60.x; enable only the features you need). No other UI crates. No WebView. Binary name: `nodi` (`[[bin]] name = "nodi" path = "src/main.rs"`). `#![windows_subsystem = "windows"]`.

## Deliver (real, working code — no placeholders that pretend to work)
1. Embedded application manifest enabling Common Controls v6 and `PerMonitorV2` DPI awareness + UTF-8 active code page. With the GNU toolchain: write `res/nodi.rc` + `res/nodi.manifest` + a simple `res/nodi.ico` (generate a valid 16x16/32x32 .ico with a small build-time Rust function in build.rs if needed — no external downloads), compile with `windres` if it is available in the toolchain (`rustc --print sysroot` → `lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained` may contain it) or use the `embed-resource` crate ONLY if windres works. If no resource compiler works: call `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)` at startup and activate comctl v6 via an activation context (`CreateActCtxW` with a manifest file written next to the exe is NOT acceptable — embed it as a byte string and use `CreateActCtxW` from a temp file in the exe directory only if nothing else works; document what you chose in REPORT.md).
2. `LoadLibraryW("msftedit.dll")` at startup; fail with MessageBox if missing.
3. Main window "Nodi" with:
   - Native menu bar: File (New Note Ctrl+N, Save Ctrl+S, Close Tab Ctrl+W, Exit), Edit (Undo Ctrl+Z, Redo Ctrl+Y, Cut, Copy, Paste, Select All), Format (Bold Ctrl+B, Italic Ctrl+I, Underline Ctrl+U, Strikethrough), View (Toolbar toggle, Status bar toggle), Help (About). Other menus (Search, Insert, Notes, Tools) come later — do not add them now.
   - Accelerator table (CreateAcceleratorTableW) + TranslateAcceleratorW in the message loop; Ctrl+Tab / Ctrl+Shift+Tab next/prev tab.
   - Toolbar (TOOLBARCLASSNAMEW, TBSTYLE_FLAT | TBSTYLE_TOOLTIPS, standard system image list IDB_STD_SMALL_COLOR) with New, Save, Cut, Copy, Paste, Undo, Redo, then B / I / U as text buttons.
   - Left TreeView (WC_TREEVIEWW, TVS_HASLINES|TVS_LINESATROOT|TVS_HASBUTTONS|TVS_SHOWSELALWAYS|TVS_EDITLABELS) titled area "Notes Explorer" — for Phase 1 populate with a few static demo items so layout is visible.
   - Vertical splitter between sidebar and editor: drag with mouse (SetCapture), cursor IDC_SIZEWE, min 120 px, max 60% width. Width stored in app state.
   - Right: Tab control (WC_TABCONTROLW) above the editor area. One Rich Edit (`MSFTEDIT_CLASS` = "RICHEDIT50W", ES_MULTILINE|ES_AUTOVSCROLL|WS_VSCROLL|ES_NOHIDESEL|ES_WANTRETURN) PER TAB; only active one visible. `EM_SETEVENTMASK` ENM_CHANGE|ENM_SELCHANGE|ENM_LINK. `EM_SETTEXTMODE` TM_RICHTEXT. Set default font Segoe UI 11pt via CHARFORMAT2W. `EM_EXLIMITTEXT` large (e.g. 0x7FFFFFFF).
   - Status bar (STATUSCLASSNAMEW) with 5 parts: `Ready` | `Saved`/`Modified` | `Ln x, Col y` (update on EN_SELCHANGE via EM_EXLINEFROMCHAR / EM_LINEINDEX) | `UTF-8` | `Rich Text`.
   - New Note (Ctrl+N) opens a new tab "Untitled" with a fresh editor and focuses it. Close Tab (Ctrl+W). Typing marks tab dirty with " *" suffix in tab text and status `Modified` (no saving yet — Phase 3).
   - Format commands work on the selection in the active editor: toggle CFE_BOLD/ITALIC/UNDERLINE/STRIKEOUT via EM_GETCHARFORMAT(SCF_SELECTION) + EM_SETCHARFORMAT. Ctrl+B/I/U must work (Rich Edit's own Ctrl+B etc. handling may conflict — ensure one toggle per keypress).
   - Undo/Redo via EM_UNDO / EM_REDO; set `EM_SETTYPOGRAPHYOPTIONS`/`EM_SETUNDOLIMIT` 100.
   - DPI: scale sizes with `GetDpiForWindow`; handle WM_DPICHANGED (SetWindowPos to suggested rect, recreate fonts).
   - Fonts: UI font from `SystemParametersInfoW(SPI_GETNONCLIENTMETRICS)` lfMessageFont applied to tree/tab/status via WM_SETFONT.
   - WM_SIZE layout: toolbar top, status bottom, sidebar left (splitter), tabs + editor right.
4. Code structure exactly as in ARCHITECTURE.md (`src/main.rs`, `src/app.rs`, `src/win.rs`, `src/wnd/{mod,menu,toolbar,tree,tabs,editor,status}.rs`). App state stored in a struct reachable from the window proc via `GWLP_USERDATA` (Box::into_raw on WM_NCCREATE, freed on WM_NCDESTROY). Every `unsafe` block gets a `// SAFETY:` comment. No panics across FFI: window proc bodies must not panic (avoid unwrap in wndproc paths).

## Verify yourself before finishing
- `cargo build -p nodi-windows` and `cargo build -p nodi-windows --release` succeed.
- `cargo clippy -p nodi-windows --all-targets -- -D warnings` clean; `cargo fmt -p nodi-windows --check` clean.
- Launch smoke test (PowerShell): start `target\release\nodi.exe`, wait 2 s, check `(Get-Process nodi).MainWindowTitle` contains "Nodi" and process is responding, then `Stop-Process -Name nodi`. Report the result.
Write `crates/nodi-windows/REPORT.md`: what works, how you verified, exe size, what is NOT done. Do NOT claim anything you did not run.
