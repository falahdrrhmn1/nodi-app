# Rev2 Batch 4 — report

Started by opencode/agy agents (interrupted three times: memory pressure, free
quota, laptop crash); finished and verified by the orchestrator directly.

## Section 0 — batch 3 bugs (screenshots after real keys + Markdown paste)

| Bug | Fix | Evidence |
|---|---|---|
| Pasted table cells grey | table cells use the automatic colour (agent run) | light/dark screenshots, cells black/white |
| ~200 px gap after a pasted table | fixed in `markdown_in` paragraph spacing (agent run) | screenshot: one line between table and output block |
| Code text light grey | code row text `#1F1F1F` (`\cf12`) | screenshot |
| Separator too thin | 6 px PNG at 60 twips height | e2e X, Y; screenshot |
| (new) second thin line under the separator | Rich Edit forces `\ul` + link colour on a field result, so the rule is no longer a `nodi-hr:` link; `image::is_rule_picture` recognises it by shape (old notes still work) | screenshot, unit test, e2e X/Y |
| (new) Insert > Separator after text put the rule on the same line and later typing before it | `insert_separator_impl` puts the rule in its own paragraph and the caret in a new paragraph after it | RTF probe: `atas\par` / rule / `bawah` |

## Section 1 — themes

- Light / Dark / Custom (`settings.json`), title bar via DWM, tree/tabs/format
  bar/status/sidebar caption themed; menus and the icon toolbar stay light
  (no supported dark menus; the standard glyphs are black).
- Fixed after the agent run: editor text stayed black in dark mode (typed
  text, headings, pasted Markdown). Now `editor.rs` maps automatic/black to the
  display colours in every `set_selection_format`, `insert_rtf` and the default
  format, and folds them back in `stream_out_rtf` / `selection_rtf`;
  `apply_theme_colors` adds an explicit `\cf` to text without one.
- Fixed: format-bar labels were cut off ("Colo…") in both themes —
  `CDRF_NEWFONT` without selecting the toolbar's font; light theme now skips
  custom draw; the checked Wrap button uses the accent colour.
- Fixed: `WM_CTLCOLORSTATIC` had `wparam`/`lparam` swapped (sidebar caption).
- Status bar: the counts part is wider.

## Section 2 — AI Markdown mirror

- Agent implementation kept (`mirror.rs`: front matter, `INDEX.md`,
  `index.json`, `.trash/`). Fixed: every tree save ran a full rebuild (all
  notes converted again) — closing after creating 150 notes took 5.2 s. Now the
  writer collects mirror work and runs it after 300 ms idle and once on exit;
  tree changes use `mirror::sync` (only moved/new notes are converted), startup
  trusts mirror files newer than their RTF. Close after 150 notes: 0.66 s.
- 5 new integration tests (`tests/mirror.rs`).

## Section 3 — extra features

Word/char counts (note + selection), spell check toggle, tab context menu +
middle-click, Restore Backup…, Recent, Outline (Ctrl+Alt+O), icons from file —
all from the agent run, now covered by e2e AI–AL.

## Verification

- `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`: clean.
- `cargo test`: all pass (core lib 84, integration 6+6+5+3+10+12, windows 6).
- `scripts\e2e.ps1`: **36 passed, 0 failed** (A–AL incl. real-key R, Y, AL).

## Not done here (moved to batch 5)

Dark code blocks (the light code block is readable on dark but not themed);
everything listed in `rev2-batch5.md`.
