# nodi-windows — Phase 1b native shell

Status: **done for the Phase 1b scope**. `crates/nodi-windows` builds a real
`nodi.exe` (pure Win32 + `windows-sys`, no WebView, no other UI crates) and every
check listed in `docs/tasks/phase1b-shell.md` was run on this machine.

- Crate: `crates/nodi-windows` (`[[bin]] name = "nodi"`, `#![windows_subsystem = "windows"]`)
- Release binary: `target\release\nodi.exe` — **280,576 bytes (274 KiB)**
- Debug binary: `target\debug\nodi.exe` — 6,252,126 bytes
- Toolchain used: `cargo 1.99.1`, `rustc 1.99.1 (x86_64-pc-windows-gnu)`,
  `windows-sys 0.60.2`, `windres` 2.28 (MinGW32), Windows 11 26H2 (10.0.26300), 125 % DPI

---

## 1. Verification (commands exactly as run)

Environment for every cargo call:

```powershell
$env:CARGO_HOME='E:\Software\Rust\cargo'; $env:RUSTUP_HOME='E:\Software\Rust\rustup'
$env:PATH="E:\Software\Rust\cargo\bin;$env:PATH"
```

| Command | Result |
| --- | --- |
| `cargo build -p nodi-windows` | `Finished dev profile`, no warnings |
| `cargo build -p nodi-windows --release` | `Finished release profile`, no warnings |
| `cargo clippy -p nodi-windows --all-targets -- -D warnings` | `Finished`, no lints |
| `cargo fmt -p nodi-windows --check` | clean (exit code 0) |
| Launch smoke test | see below |

Smoke test, verbatim outcome:

```
MainWindowTitle : 'Untitled 1 - Nodi'
contains Nodi   : True
Responding      : True
leftover nodi   : False        (Stop-Process -Name nodi succeeded)
```

The launch was repeated several times during the session; no crash, no
non-responding window, no leftover process (`Get-WinEvent` application log stays
clean during normal use — the only crash entries in the log come from my own
invalid cross-process probes, see §6).

Resource verification on `target\release\nodi.exe`:

```
(Get-Item ...\nodi.exe).VersionInfo
  FileDescription  : Nodi Mark III
  FileVersion      : 0.3.0.0
  ProductName      : Nodi Mark III
  ProductVersion   : 0.3.0.0
  OriginalFilename : nodi.exe
  InternalName     : nodi

byte search of the exe : PerMonitorV2 = True, Microsoft.Windows.Common-Controls = True
[System.Drawing.Icon]::ExtractAssociatedIcon(exe).Width/Height = 32 / 32
compiled resource object machine = 0x8664 (x86-64)
```

## 2. What works

**Crate layout** — exactly the files the spec asks for: `src/main.rs`,
`src/app.rs`, `src/win.rs`, `src/wnd/{mod,menu,toolbar,tree,tabs,editor,status}.rs`,
plus `src/wnd/dialogs.rs` (the two `MessageBoxW` wrappers, kept out of the
window procedure). `build.rs` + `res/` carry the resources. App state is a
`Box<App>` stored in `GWLP_USERDATA` on `WM_NCCREATE` and dropped on
`WM_NCDESTROY`; every `unsafe` block has a `// SAFETY:` comment and the window
procedure contains no panicking calls.

**Window chrome** — menu bar with exactly File / Edit / Format / View / Help;
accelerator table (`CreateAcceleratorTableW` + `TranslateAcceleratorW` in the
message loop) with Ctrl+N, Ctrl+S, Ctrl+W, Ctrl+Z, Ctrl+Y, Ctrl+B, Ctrl+I,
Ctrl+U, Ctrl+Tab and Ctrl+Shift+Tab; toolbar built from the system image list
(`IDB_STD_SMALL_COLOR`) with New, Save, Cut, Copy, Paste, Undo, Redo followed by
B / I / U as text buttons (`TBSTYLE_EX_MIXEDBUTTONS` + `BTNS_SHOWTEXT`).
Save stays disabled (greyed in both menu and toolbar) because nothing is stored
yet; Ctrl+S reports `Save is not implemented yet (Phase 3)` in the status bar.

**Layout** — `WM_SIZE` places toolbar at the top, status bar at the bottom,
sidebar on the left, and the tab control + editor on the right. The vertical
splitter is drawn by the frame, uses `SetCapture` / `ReleaseCapture`, an
`IDC_SIZEWE` cursor and clamps the sidebar to 120 px … 60 % of the client width
(and always leaves ≥ 160 px for the editor).

**Tabs and editors** — one `RICHEDIT50W` per tab (`ES_MULTILINE |
ES_AUTOVSCROLL | WS_VSCROLL | ES_NOHIDESEL | ES_WANTRETURN`), only the active one
visible, so caret/selection/undo stack survive tab switches. `EM_SETEVENTMASK`
with `ENM_CHANGE|ENM_SELCHANGE|ENM_LINK`, `EM_SETTEXTMODE` rich text,
`EM_EXLIMITTEXT` 0x7FFFFFFF, `EM_SETUNDOLIMIT` 100, `EM_SETTYPOGRAPHYOPTIONS`,
Segoe UI 11 pt via `CHARFORMAT2W` (`SCF_ALL|SCF_DEFAULT`). Tabs are titled
`Untitled 1`, `Untitled 2`, … (counter, so two untitled notes are
distinguishable); Ctrl+N / Ctrl+W / Ctrl+Tab / Ctrl+Shift+Tab work, closing the
last tab leaves an empty frame with `No note open`.

**Editing** — typing flips the tab label to `<title> *`, the status part to
`Modified` and the frame caption to `<title> * - Nodi`. Format commands toggle
Bold/Italic/Underline/Strikethrough through
`EM_GETCHARFORMAT(SCF_SELECTION)` + `EM_SETCHARFORMAT`; one command (one
keypress) toggles exactly once — verified visually, see §4. Undo/redo via
`EM_UNDO` / `EM_REDO`, Cut/Copy/Paste via `WM_CUT`/`WM_COPY`/`WM_PASTE`,
Select All via `EM_EXSETSEL`.

**Status bar** — five parts: `Ready` | `Saved`/`Modified` | `Ln x, Col y` |
`UTF-8` | `Rich Text`. Caret text is recomputed on `EN_SELCHANGE` with
`EM_EXGETSEL` + `EM_EXLINEFROMCHAR` + `EM_LINEINDEX`; part widths are recomputed
in `WM_SIZE` so the message part absorbs the remaining space.

**Resources / DPI** — embedded manifest (Common Controls v6, `PerMonitorV2`,
`activeCodePage UTF-8`, `longPathAware`), icon and `VERSIONINFO` (see §3).
Sizes are scaled from `GetDpiForWindow`, fonts are rebuilt from
`SPI_GETNONCLIENTMETRICS` `lfMessageFont` on `WM_DPICHANGED`, and the suggested
rect is applied. `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` is also
called at startup as a fallback for resource-less builds.

**Dialogs** — Help ▸ About (`About Nodi`, shows `Nodi Mark III — 0.3.0`) and a
discard confirmation on `WM_CLOSE` when any tab is dirty (`OK` closes, `Cancel`
keeps the app running).

Deliberate small addition beyond the spec: the frame caption mirrors the active
tab and its dirty marker. The spec only requires "Nodi" in the title, which still
holds.

## 3. Resource embedding — what I chose and why

The spec suggested checking `rustc --print sysroot` →
`lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained` for `windres`. It is
**not** there: that directory only contains `dlltool.exe`, `ld.exe` and
`x86_64-w64-mingw32-gcc.exe`. The only `windres` on this machine is
`E:\Software\C++ Compiler\bin\windres.exe` (binutils 2.28), and it reports:

```
supported targets: pe-i386 pei-i386 elf32-i386 elf32-i386 ... 
```

i.e. **no 64-bit COFF writer**, so linking its object into the x86-64 image failed
with `ld: i386 architecture of input file ... is incompatible with i386:x86-64
output`.

Chosen approach (`build.rs`):

1. `res/nodi.rc` (checked in) pulls in `res/nodi.ico`, `res/nodi.manifest` and a
   `VERSIONINFO` block; numeric resource types (1/16/24) are used so no MinGW
   headers are needed.
2. `build.rs` looks for `windres` (`$WINDRES`, `windres`,
   `x86_64-w64-mingw32-windres`, then the sysroot self-contained path), compiles
   `res/nodi.rc` to `OUT_DIR/nodi_res.o` and passes it to the link through
   `cargo:rustc-link-arg-bins`.
3. `retarget_object` then rewrites that object for the target architecture: the
   COFF machine field (`0x014c` → `0x8664`) and the relocation types
   (`DIR32→ADDR32`, `DIR32NB→ADDR32NB`, `SECTION`, `SECREL`). This is exact for a
   resource object — it holds data plus section-relative fixups only. The
   function refuses to touch an object that contains a code/executable section
   (`IMAGE_SCN_CNT_CODE|MEM_EXECUTE`) or an unknown relocation type, and panics
   with a clear message instead of producing a broken binary.
4. If no `windres` is found at all, the build still succeeds with
   `cargo:warning=...` and the binary simply has no manifest/icon; the runtime
   `SetProcessDpiAwarenessContext` call keeps it DPI-aware (Common Controls v6
   styling is then unavailable). A `windres` *failure* is a hard build error.

The icon itself is a 16/32/48 px, 32-bit BGRA `.ico` drawn by a small Rust
routine during development and then committed as a normal asset in `res/`
(no downloads). `nodi.rc` embeds it as resource id 1 (RT_GROUP_ICON).

## 4. Behaviour checks actually performed

Besides the smoke test, the running binary was driven with posted Win32
messages and inspected with screenshots (`PrintWindow`, saved under `target/`
during the session, not committed). All of these were observed, not assumed:

| Check | How | Result |
| --- | --- | --- |
| Caption | `WM_GETTEXT` on the frame | `Untitled 1 - Nodi` |
| Typing marks dirty | `WM_CHAR` ×5 into the editor | caption `Untitled 1 * - Nodi`, status `Modified` |
| Caret tracking | type `Note` | status `Ln 1, Col 5` |
| Bold toggle, once per command | Select All + `WM_COMMAND IDM_FORMAT_BOLD` | text bold; second command → regular (screenshots) |
| Undo / Redo | `WM_COMMAND IDM_EDIT_UNDO` / `IDM_EDIT_REDO` | text cleared, then restored |
| Save notice | `WM_COMMAND IDM_FILE_SAVE` | status `Save is not implemented yet (Phase 3)` |
| New tab | `WM_COMMAND IDM_FILE_NEW` | tabs 1 → 2, selection 1, caption `Untitled 2 - Nodi` |
| Tab cycling | `WM_COMMAND IDM_TAB_NEXT` / `IDM_TAB_PREV` | selection 1 → 0 → 1, caption follows, dirty marker preserved |
| Close tab | `WM_COMMAND IDM_FILE_CLOSE_TAB` | tabs 2 → 1, caption `Untitled 1 * - Nodi` |
| Splitter drag | real cursor moved with `SetCursorPos` + `WM_LBUTTONDOWN`/`WM_MOUSEMOVE`/`WM_LBUTTONUP` | tree width 240 px → 420 px (follows the mouse, respects the clamp) |
| View toggles | `IDM_VIEW_TOOLBAR`, `IDM_VIEW_STATUS` | chrome hidden and restored, editor re-laid out both times |
| About | `IDM_HELP_ABOUT` | `About Nodi` dialog found, dismissed, app still responsive |
| Discard prompt | type text then `WM_CLOSE` | dialog with OK/Cancel; Cancel keeps the app alive, OK exits cleanly |
| Status bar parts | screenshots | `Ready \| Modified \| Ln 1, Col 1 \| UTF-8 \| Rich Text` |

Not exercised, and therefore not claimed: **physical key presses**. This agent
session cannot take the foreground window (`SetForegroundWindow`/`AppActivate`
are refused by the desktop), so `SendInput`-based Ctrl+B/Ctrl+Tab tests were
skipped. The accelerator table itself is built from the listed keys, and every
command it dispatches was exercised through `WM_COMMAND` as described above.

## 5. `windows-sys` gaps worth knowing

- `windows-sys` 0.60.2 has **no Rich Edit bindings** (no `CHARFORMAT2W`, no
  `CHARRANGE`, no `EM_*` beyond the plain Edit-control values). `editor.rs`
  therefore declares the ABI locally: `CharFormat2W` (116 bytes, checked with a
  compile-time `assert!`) and `CharRange`, plus the Rich Edit message numbers.
- Windows' own `richedit.h` mixes the two families: `EM_UNDO` keeps the
  **Edit-control value `0x00C7`** (there is no `WM_USER` variant), while
  `EM_REDO` is `WM_USER+84`. Using `WM_USER+23` for undo compiles and silently
  does nothing — I hit exactly that during testing and fixed it.
- `HINST_COMMCTRL` (needed by `TB_LOADIMAGES`) is not bound; a local
  `const HINST_COMMCTRL: isize = -1` is used.

Two Win32 details that cost time and are worth remembering:

- `WM_NCCREATE` must fall through to `DefWindowProcW`: the default handler is
  what stores the creation title. Returning `TRUE` directly leaves the caption
  empty (the window existed but reported `MainWindowTitle = ""`).
- `SB_SETPARTS` is what actually creates the status-bar panes; texts sent before
  it are dropped. The static labels are therefore written after the first
  `layout()` pass, not during status-bar creation.

## 6. Not done (intentionally, later phases)

- **No persistence at all.** Save is disabled, Ctrl+S only prints a status
  message, nothing is written to disk; there is no RTF stream in/out
  (`EM_STREAMIN`/`EM_STREAMOUT`), no data directory, no autosave, no backup.
  Closing a dirty *tab* does not prompt (only closing the app does).
- **No `nodi-core` dependency.** Per the task, Phase 2 wires the core crate; the
  tree is static demo data (`All Notes`, `Welcome to Nodi`, `Project Ideas`,
  `Meeting Notes`, `Trash`) and label editing is only enabled as a style
  (`TVS_EDITLABELS`), not handled.
- Undo/redo toolbar buttons are always enabled (they do not mirror the control's
  undo/redo availability), and the Format menu does not show check marks for the
  current selection.
- Encoding/mode status parts are static labels (`UTF-8`, `Rich Text`), not
  computed from the document.
- No search, no hyperlink handling (`ENM_LINK` is enabled but no `EN_LINK`
  handler), no images/tables in notes, no printing, no tab reordering or
  drag & drop, no per-tab close buttons, no context menus, no settings
  (window geometry/splitter width are not remembered).
- DPI: the app was run at 125 % only. The `WM_DPICHANGED` path is implemented
  (suggested rect, font recreation, sidebar rescale) but could not be exercised —
  this machine has one monitor at a fixed scale.
- No unit tests in this crate: the surface is Win32 message handling, so
  verification is black-box (window messages + screenshots) as listed in §4.

## 7. Scope hygiene

- Only `crates/nodi-windows/` was created/modified. `git status --short` shows a
  single entry, `?? crates/nodi-windows/`; `git diff --stat` is empty, so
  `crates/nodi-core`, `docs/`, `Cargo.toml` and `Cargo.lock` are untouched.
- No files were deleted, nothing outside `E:\nodi-app\mark-3` was written.
