# Dependencies

| Crate | Used by | Why |
|---|---|---|
| `windows-sys` 0.60 | nodi-windows | Raw Win32 bindings (no COM wrappers, no runtime). Only the needed features are enabled. |
| `serde` + `serde_json` | nodi-core | `tree.json` / `session.json` (de)serialisation. |
| `miniz_oxide` 0.8 | nodi-core | zlib compression for formula PNGs. Pure Rust, no C code, no further dependencies of note. |

Everything else is Windows itself: Rich Edit (`msftedit.dll`), Common Controls,
common dialogs (open/save, colour, font, find/replace), GDI (formula drawing
with the Cambria Math font that ships with Windows), Shell (`ShellExecuteW`,
`SHCreateMemStream`), OLE (Rich Edit drag & drop).

Build: Rust stable `x86_64-pc-windows-gnu` (MinGW linker bundled with the
toolchain); `windres` for the icon/manifest resource.
