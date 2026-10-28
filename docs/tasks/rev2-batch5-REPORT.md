# Rev2 Batch 5 — report

Done by the orchestrator directly (user's request), with a small agent part
(README + `scripts/persist-torture.ps1`, see `rev2-batch5-agent-REPORT.md`).

| § | Item | Result | Evidence |
|---|---|---|---|
| 0 | RTF injection through pasted link targets | `links::field_target` percent-encodes `{ } \ "`, control and non-ASCII characters; `links::pasted_link_allowed` keeps only http(s), mailto, file, drive paths and `nodi` links (also in `html_in`, so no `[x](javascript:…)` text) | unit `hostile_link_targets_cannot_inject_rtf`, `only_safe_schemes_become_links`, `local_paths_with_braces_and_non_ascii_round_trip`; e2e AM |
| 1 | Heading typing "selects" text | `editor::Frozen` (no painting while the selection is moved for probing), marker formats only applied when they differ, body paragraphs return before touching the selection | e2e AN (real keys: selection collapsed after every key, one Ctrl+Z undoes the typing) |
| 2 | Tab × / + / Ctrl+T | owner-drawn × with hover, + after the last tab, Ctrl+T + File > New Tab. Found with the real mouse: the tab control answers `HTTRANSPARENT` outside its tabs (clicks on + went to the frame) → `WM_NCHITTEST` returns `HTCLIENT` there; a fast second click is a `WM_LBUTTONDBLCLK` → handled too | e2e AO (real clicks) |
| 3 | Title follows the first line | `Node::auto_title` (serde default false), `Tree::follow_text` at save time, manual rename clears it | unit tests; e2e AP (real keys) |
| 4 | Ctrl+Z in the Notes Explorer | `TreeUndo` (Trash/Move/Rename) with confirmation question, 20 steps | unit tests; e2e AQ (real click + Ctrl+Z) |
| 5 | Tree drag & drop | **Two real bugs**: the drop hit test accepted only the label (`TVHT_ONITEM`), so dropping right of a title meant "empty space → top level"; and `TVM_GETITEMRECT` was called with the item in wParam, so the before/after/inside zone was never computed for a real drag. Now the whole row counts (`hit_row`), `row_rect` passes the item correctly (context menus too) | e2e AV (real mouse drags: above the first item, inside a folder) |
| 6 | Ctrl+Up/Down scroll toggle | `settings.ctrl_arrow_scroll` (default on), View menu | e2e AR (real keys) |
| 7 | Import / Export | `nodi_core::import` (plan, encoding detection, Markdown with local pictures embedded only from inside the folder, text → RTF), worker thread, "Imported <date>" folder, Explorer drop (pictures on the note are inserted), Export Note As .md/.rtf/.txt, `mirror::export` for a folder | unit tests (4 + 1); e2e AT |
| 8 | AI mirror folder | Tools > AI Mirror Folder… / Use Default / Copy AI Folder Path / Open AI Folder, Help > Using Nodi with AI Agents; local drives only; a used folder gets a `Nodi AI` sub-folder (the mirror moves unknown `.md` files to `.trash`, so it must never take over a user folder) | e2e AU |
| 9 | README | "Nodi App", import/export, mirror folder (agent + orchestrator) | — |
| 10 | Paste formulas | `markdown_in` placeholders (U+E000…U+E001) rendered by `formula::render_pasted`; `html_in` reads KaTeX `<annotation>`, `<math alttext>`, MathJax `<script type="math/tex">`, `data-latex`, skips `aria-hidden` duplicates (depth-aware skipping, void elements fixed) | unit tests; e2e AS |
| 11 | Counts | `Lines · Words · Chars` + `Sel: …`, debounced 120 ms | unit tests; e2e AI, AW |
| 12 | Save torture | `scripts/persist-torture.ps1` (agent), 4 close modes rotating | see below |

Also fixed: the e2e helper `Set-ClipboardHtml` computed the CF_HTML header
length from the format placeholders (12 characters short), so earlier HTML
paste tests ran on shifted offsets.

## Save reliability (§12)

See `rev2-batch5-agent-REPORT.md` for the harness and its first runs (≈130
starts, 0 mismatches).

On the finished build the first 3 × 20 run found **2 mismatches**, both in
cycle 1: a move was done in the app (`parent C1-x` = Torture, state `Saved`)
but `tree.json` on disk never showed it. Cause: the harness polls `tree.json`
every 50 ms; replacing a file that another process has open fails on Windows,
and a failed tree write was only reported, never retried — the change reached
the disk only with the next tree change (a scanner or sync client can do the
same in real use). Fix: `rename` retries for ~1 s on a locked target
(`storage::rename_retrying`), and a failed `tree.json` / `session.json` write
stays queued and is retried every second (`autosave::RETRY_DELAY`, unit test
`a_failed_tree_write_is_retried_without_a_new_submission`). After the fix:
**2 × (3 × 20) cycles, 0 mismatches**.

Not covered end to end: killing the process *during* a slow write. The atomic
write path (`.tmp` → `sync_all` → `.prev` → rename) and leftover recovery are
covered by unit tests (`failure_injection_returns_err_and_keeps_the_original`,
`recover_leftovers_moves_tmp_files_and_never_deletes_them`).

Known nit (from the harness): after `WM_ENDSESSION` Nodi flushes but does not
exit by itself; Windows ends it anyway during a real shutdown.

## Verification

- `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo test`: 152 tests pass.
- `scripts\e2e.ps1`: **47 passed, 0 failed** (A–AW, real keys R/Y/AL/AN/AO/AP/AQ/AR, real mouse AO/AQ/AV).
