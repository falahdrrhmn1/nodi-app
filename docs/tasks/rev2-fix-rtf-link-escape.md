# Fix — RTF injection through pasted link targets (security)

Same rules/build env as `docs/tasks/rev2-batch1.md`.

`crates/nodi-core/src/markdown_in.rs` builds `{\field{\*\fldinst{HYPERLINK "<target>"}}…}` and only escapes `"` and `\` in the target. A pasted web page (CF_HTML `href`) or Markdown `[x](…)` controls that target, so `}` / `{` close the field group and inject arbitrary RTF (e.g. `\object`, `\pict`, extra fields) into the note.

1. Add one function (e.g. `links::rtf_field_target(target) -> String`) used by **every** place that writes a `HYPERLINK` instruction from data (markdown_in, the Windows link insertion, formula `nodi-math:` links, copy links, file URLs): percent-encode `{`, `}`, `"`, `\`→ keep path semantics by converting local paths to file URLs first, control characters (< 0x20, 0x7F) and every non-ASCII char (UTF-8 bytes as `%XX`). Result must be pure printable ASCII without `{ } \ "`.
2. Allowlist schemes for links created from paste/Markdown: `http`, `https`, `mailto`, `file` (local drive paths only), `nodi`/`nodi-note`. Anything else (`javascript:`, `vbscript:`, `ms-*:`, `search-ms:`, unknown) becomes plain text, not a link. Follow-time safety in `wnd/link.rs` stays as is.
3. Make sure decoding for display/follow (`target_of`, `hyperlink_target`, `file_url_path`, Markdown export) still gives the original target (non-ASCII and `{}` round-trip).
4. Grep the whole repo for other `fldinst` / `HYPERLINK` / raw RTF construction from user-controlled text (html_in, markdown_in, rtf.rs, image/crop patching, mirror) and apply the same escaping; list each site in the report.

Tests: unit tests with targets `https://x/}}{\object\objemb{}}`, `https://x/a"b`, `https://例え.jp/ä`, `javascript:alert(1)`, `C:\Folder {1}\ä.pdf` → generated RTF parses back with exactly one field, correct target, no extra groups; scheme refusal. e2e: paste CF_HTML with such an href → note has exactly one link, `link-target` equals the original. All earlier tests pass; fmt/clippy clean. Write `docs/tasks/rev2-fix-rtf-link-escape-REPORT.md`.
