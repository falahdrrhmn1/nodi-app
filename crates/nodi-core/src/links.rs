//! Internal note links: `nodi://note/<id>`.

use crate::id::NoteId;

/// URL scheme prefix for internal note links.
pub const LINK_PREFIX: &str = "nodi://note/";

/// Formats an internal link for a note.
pub fn format_note_link(id: &NoteId) -> String {
    format!("{LINK_PREFIX}{id}")
}

/// Parses an internal link. The scheme and host are matched
/// case-insensitively, surrounding whitespace is ignored, and the id must be a
/// valid [`NoteId`].
pub fn parse_note_link(text: &str) -> Option<NoteId> {
    let trimmed = text.trim();
    if trimmed.len() <= LINK_PREFIX.len() {
        return None;
    }
    // `get` instead of `split_at` so multi-byte input can never panic.
    let scheme = trimmed.get(..LINK_PREFIX.len())?;
    let rest = trimmed.get(LINK_PREFIX.len()..)?;
    if !scheme.eq_ignore_ascii_case(LINK_PREFIX) {
        return None;
    }
    let rest = rest.split(['?', '#']).next().unwrap_or(rest);
    NoteId::parse(rest)
}

/// Scheme of formula links: the formula picture is the link text and the
/// percent-encoded LaTeX is the target. Rich Edit drops custom PNG chunks and
/// picture descriptions on reload, but keeps hyperlink targets.
pub const MATH_PREFIX: &str = "nodi-math:";

/// `nodi-math:<percent-encoded LaTeX>`.
pub fn format_math_link(latex: &str) -> String {
    let mut out = String::from(MATH_PREFIX);
    for byte in latex.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Target of a separator picture: the picture is the link text and the target
/// marks it as a horizontal rule (see `Insert > Separator`).
pub const RULE_PREFIX: &str = "nodi-hr:";

/// `nodi-hr:` for the separator's `HYPERLINK` field.
pub fn format_rule_link() -> String {
    RULE_PREFIX.to_owned()
}

/// Target of a code block's Copy link. The block itself is found from the
/// link's position in the document.
pub const COPY_PREFIX: &str = "nodi-copy:";

/// `nodi-copy:` for the code block's Copy link.
pub fn format_copy_link() -> String {
    COPY_PREFIX.to_owned()
}

/// True for a code block's Copy link.
pub fn is_copy_link(target: &str) -> bool {
    target.trim() == COPY_PREFIX
}

/// True for the separator target.
pub fn is_rule_link(target: &str) -> bool {
    target.trim() == RULE_PREFIX
}

/// A link target that is safe inside `{\*\fldinst{HYPERLINK "<target>"}}`:
/// `{`, `}`, `\`, `"`, control characters and every non-ASCII character are
/// percent-encoded (UTF-8), so the target can never close the field group or
/// smuggle RTF into the note. Everything else (URL syntax, spaces, existing
/// `%XX` escapes) is kept as written.
pub fn field_target(target: &str) -> String {
    let mut out = String::with_capacity(target.len());
    for ch in target.trim().chars() {
        if (' '..='~').contains(&ch) && !matches!(ch, '{' | '}' | '\\' | '"') {
            out.push(ch);
        } else {
            let mut buffer = [0u8; 4];
            for byte in ch.encode_utf8(&mut buffer).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

/// Link targets that pasted Markdown/HTML may turn into links: web and mail
/// addresses, `file:` URLs, local drive paths and Nodi's own `nodi` links.
/// Anything else (`javascript:`, `vbscript:`, `ms-…:`, `search-ms:`, `data:`,
/// relative addresses, …) stays plain text.
pub fn pasted_link_allowed(target: &str) -> bool {
    let lower = target.trim().to_ascii_lowercase();
    [
        "http://", "https://", "mailto:", "file:", "nodi://", "nodi-",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || is_drive_path(target.trim())
}

/// Finds a full local path at the end of `text` and returns the byte offset
/// where the replaceable range starts plus the path itself. Quotes around the
/// path are included in the range, so converting removes them.
///
/// Recognises only fully qualified drive paths (`C:\...`, `D:\...`): relative
/// paths, UNC paths and other schemes are left alone. Trailing sentence
/// punctuation is not part of the path.
pub fn local_path_suffix(text: &str) -> Option<(usize, &str)> {
    if let Some(rest) = text.strip_suffix('"') {
        let open = rest.rfind('"')?;
        let path = rest.get(open + 1..)?;
        is_drive_path(path).then_some((open, path))
    } else {
        let mut start = text.len();
        for (index, ch) in text.char_indices().rev() {
            if ch.is_whitespace() || matches!(ch, '"' | '\'' | '(' | '[' | '<' | '{') {
                break;
            }
            start = index;
        }
        let candidate = text.get(start..)?;
        let path = candidate.trim_end_matches(['.', ',', ';', '!', '?', ')', ']', '}']);
        is_drive_path(path).then_some((start, path))
    }
}

/// The whole (trimmed) `text` is one local path, optionally quoted.
pub fn whole_local_path(text: &str) -> Option<&str> {
    let text = text.trim();
    let (offset, path) = local_path_suffix(text)?;
    let quoted = text.starts_with('"') && text.ends_with('"') && text.len() >= 2;
    let covered = if quoted {
        offset == 0 && text.len() == path.len() + 2
    } else {
        offset == 0 && text.len() == path.len()
    };
    covered.then_some(path)
}

/// `X:\` followed by anything except characters Win32 treats as wildcards,
/// quotes, control characters or a second colon (an alternate data stream).
fn is_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return false;
    }
    path.matches(':').count() == 1
        && !path.chars().any(|ch| {
            ch == '"'
                || ch == '*'
                || ch == '?'
                || ch == '<'
                || ch == '>'
                || ch == '|'
                || ch.is_control()
        })
}

/// `file:///C:/Users/a%20b` for a local drive path, or
/// `file://server/share` for a UNC path. Rich Edit keeps a `HYPERLINK` field
/// only when the target looks like a URL, so path links have to be stored as
/// file URLs; [`file_url_path`] turns them back into paths.
pub fn path_file_url(path: &str) -> Option<String> {
    let path = path.trim();
    let bytes = path.as_bytes();
    let drive =
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    let unc = path.starts_with("\\\\");
    if !drive && !unc {
        return None;
    }
    let mut out = String::from(if unc { "file://" } else { "file:///" });
    let mut skipped = 0;
    for ch in path.chars() {
        if unc && skipped < 2 && ch == '\\' {
            skipped += 1;
            continue;
        }
        let ch = if ch == '\\' { '/' } else { ch };
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~' | ':' | '/') {
            out.push(ch);
        } else if ch == '/' {
            out.push('/');
        } else {
            let mut buffer = [0u8; 4];
            for byte in ch.encode_utf8(&mut buffer).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    Some(out)
}

/// Turns a `file:` URL back into the local path it names.
pub fn file_url_path(target: &str) -> Option<String> {
    let target = target.trim();
    let scheme = target.get(..5)?;
    if !scheme.eq_ignore_ascii_case("file:") {
        return None;
    }
    let original = &target[5..];
    let trimmed = original.trim_start_matches('/');
    let unc = original.starts_with("//") && !original.starts_with("///");
    let decoded = percent_decode(trimmed).replace('/', "\\");
    Some(if unc {
        format!("\\\\{decoded}")
    } else {
        decoded
    })
}

/// Percent-decodes a `%XX` escaped target (the Markdown writer escapes spaces
/// and parentheses in link targets).
pub fn percent_decode_text(text: &str) -> String {
    percent_decode(text)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[index + 1..index + 3]) {
                if let Ok(value) = u8::from_str_radix(hex, 16) {
                    out.push(value);
                    index += 3;
                    continue;
                }
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// LaTeX from a `nodi-math:` link target.
pub fn parse_math_link(target: &str) -> Option<String> {
    let encoded = target.trim().strip_prefix(MATH_PREFIX)?;
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_links_round_trip() {
        let latex = r#"L = \frac{1}{n} \sum_{i=1}^{n} (\hat{y}_i - y_i)^2 + "q" ≤ ∞ 100%"#;
        let link = format_math_link(latex);
        assert!(link.starts_with(MATH_PREFIX));
        assert!(!link.contains(' ') && !link.contains('"') && !link.contains('\\'));
        assert_eq!(parse_math_link(&link).as_deref(), Some(latex));
        assert_eq!(
            parse_math_link("nodi-math:E%20%3D%20mc%5E2").as_deref(),
            Some("E = mc^2")
        );
        assert!(parse_math_link("https://example.com").is_none());
    }

    #[test]
    fn rule_links_are_recognised() {
        assert!(is_rule_link(RULE_PREFIX));
        assert!(is_rule_link("  nodi-hr:  "));
        assert!(!is_rule_link("nodi-math:E%20%3D%20mc%5E2"));
        assert_eq!(format_rule_link(), RULE_PREFIX);
        assert!(is_copy_link(COPY_PREFIX));
        assert!(is_copy_link(" nodi-copy: "));
        assert!(!is_copy_link("nodi-hr:"));
        assert_eq!(format_copy_link(), COPY_PREFIX);
    }

    #[test]
    fn finds_local_paths_at_the_end() {
        assert_eq!(
            local_path_suffix(r"See C:\Users\Public"),
            Some((4, r"C:\Users\Public"))
        );
        assert_eq!(
            local_path_suffix(r"Open C:\Users\paper.pdf."),
            Some((5, r"C:\Users\paper.pdf"))
        );
        // Quoted paths keep their spaces; the range starts at the quote.
        assert_eq!(
            local_path_suffix(r#"See "D:\My Data\a b.txt""#),
            Some((4, r"D:\My Data\a b.txt"))
        );
        assert_eq!(whole_local_path(r" C:\Windows "), Some(r"C:\Windows"));
        assert_eq!(
            whole_local_path(r#""C:\Program Files\app.exe""#),
            Some(r"C:\Program Files\app.exe")
        );
        assert_eq!(whole_local_path(r"C:\a b.txt"), None);
        assert_eq!(whole_local_path("https://example.com/a"), None);
        assert_eq!(whole_local_path(r"\\server\share\a.txt"), None);
        assert_eq!(whole_local_path(r"relative\path"), None);
        assert_eq!(whole_local_path(r"C:a.txt"), None);
        assert_eq!(whole_local_path(r"C:\x\a.txt::$DATA"), None);
        // A bare drive root is still a path (the Windows folder).
        assert_eq!(local_path_suffix(r"D:\"), Some((0, r"D:\")));
        // Multi-byte text before the path must not break the offset.
        assert_eq!(
            local_path_suffix("Buka C:\\Users\\Ä"),
            Some((5, r"C:\Users\Ä"))
        );
    }

    #[test]
    fn round_trip() {
        let id = NoteId::generate();
        let link = format_note_link(&id);
        assert!(link.starts_with("nodi://note/"));
        assert_eq!(parse_note_link(&link), Some(id));
    }

    #[test]
    fn accepts_surrounding_noise() {
        let id = NoteId::parse("0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(
            parse_note_link("  NODI://NOTE/0123456789abcdef0123456789abcdef  "),
            Some(id.clone())
        );
        assert_eq!(
            parse_note_link("nodi://note/0123456789abcdef0123456789abcdef/\n"),
            None
        );
    }

    #[test]
    fn rejects_other_links() {
        assert_eq!(parse_note_link(""), None);
        assert_eq!(parse_note_link("nodi://note/"), None);
        assert_eq!(parse_note_link("nodi://note/short"), None);
        assert_eq!(parse_note_link("https://example.com"), None);
        assert_eq!(
            parse_note_link("nodi://note/0123456789abcdef0123456789abcdeZ"),
            None
        );
        // Multi-byte input must never panic on a byte index.
        assert_eq!(parse_note_link("ノートブックのリンク"), None);
    }

    #[test]
    fn ignores_query_and_fragment() {
        let id = NoteId::parse("0123456789abcdef0123456789abcdef").unwrap();
        let link = format_note_link(&id);
        assert_eq!(
            parse_note_link(&format!("{link}?caret=7")),
            Some(id.clone())
        );
        assert_eq!(parse_note_link(&format!("{link}#anchor")), Some(id));
    }
}

#[cfg(test)]
mod file_url_tests {
    use super::*;

    #[test]
    fn local_paths_become_file_urls_and_back() {
        assert_eq!(
            path_file_url(r"C:\Users\Public").as_deref(),
            Some("file:///C:/Users/Public")
        );
        assert_eq!(
            path_file_url(r"C:\My Docs\a b.txt").as_deref(),
            Some("file:///C:/My%20Docs/a%20b.txt")
        );
        assert_eq!(
            path_file_url(r"\\server\share\a b").as_deref(),
            Some("file://server/share/a%20b")
        );
        assert_eq!(path_file_url("https://example.com"), None);
        assert_eq!(path_file_url(r"relative\path"), None);
        for path in [r"C:\Users\Public", r"D:\Ä ö\x#y.txt", r"\\server\share\a b"] {
            let url = path_file_url(path).unwrap();
            assert_eq!(file_url_path(&url).as_deref(), Some(path));
        }
        assert_eq!(
            file_url_path("file:///C:/My%20Docs/a.txt").as_deref(),
            Some(r"C:\My Docs\a.txt")
        );
        assert_eq!(
            file_url_path("file://server/share/a").as_deref(),
            Some(r"\\server\share\a")
        );
        assert_eq!(
            file_url_path("FILE:///C:/A%20B").as_deref(),
            Some(r"C:\A B")
        );
    }
}
