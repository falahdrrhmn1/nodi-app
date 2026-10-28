# Batch 5 — agent part (small, isolated)

Rules: only touch the two files named below (`README.md`, new
`scripts/persist-torture.ps1`) — the orchestrator is editing the Rust code at
the same time, so do NOT edit any other file, do not run `cargo fmt`, do not
commit. Never delete files. Stay inside `E:\nodi-app\mark-3`. Build env:
`CARGO_HOME=E:\Software\Rust\cargo`, `RUSTUP_HOME=E:\Software\Rust\rustup`,
`CARGO_BUILD_JOBS=2` (only `cargo build` if you need a binary; e2e helpers use
`target\debug\nodi.exe`).

## A. README.md (spec `docs/tasks/rev2-batch5.md` section 9)
Title **Nodi App**; remove "Mark III" from the title, headings and intro (a
path like `mark-3` may stay). Short intro, features list (read
`docs/STATUS.md` "Done"), how to run (`dist\Nodi\nodi.exe`, portable, data in
`dist\Nodi\data`), how to build (`scripts\build.ps1`), where the AI Markdown
mirror is (`data\markdown`, start with `INDEX.md`), link to
`docs/SHORTCUTS.md`. Keep it concise, English.

## B. scripts/persist-torture.ps1 (spec section 12, harness only)
Write the save-reliability torture script described in
`docs/tasks/rev2-batch5.md` section 12, reusing `scripts\e2e.ps1` helpers by
dot-sourcing it (`. .\scripts\e2e.ps1 -Only none`; see `scripts\probe-lib.ps1`
for how). Parameters: `-Cycles 20`, `-Runs 1`. Each cycle: start Nodi on one
fresh data dir (kept across cycles), verify the previous cycle's expected state
exactly (per note: `text` hook output + `markdown` hook output; tree titles and
parents from `tree.json`), make varied edits through hooks (`type`, `enter`,
`tab`, `cmd <id>` for heading/list/table/code block/separator — ids in
`crates\nodi-windows\src\wnd\menu.rs`, `formula <latex>`, `rename-selected`,
`cmd 170` new note, `move-rel`), then close in rotation: (1) `Stop-Nodi`
immediately after the last edit without waiting for Saved, (2) `cmd` Ctrl+S
(find the Save id in menu.rs) then Stop-Nodi, (3) send
`WM_QUERYENDSESSION`(0x11)+`WM_ENDSESSION`(0x16, wparam 1) to the window then
wait for exit, (4) wait for `Saved` (Wait-State) then `Stop-Process -Force`.
Record the expected state after each cycle from the hooks *before* closing
(after waiting for Saved in mode 4 only; in modes 1–3 Nodi must flush on close,
so the state read before closing must survive). Print one line per cycle and a
final `N cycles, M mismatches`; on a mismatch print a short diff and continue.
Exit code = mismatches. Run it once with `-Cycles 6` to make sure the script
itself works and paste the output into
`docs/tasks/rev2-batch5-agent-REPORT.md` (do not fix product bugs — report
them).
