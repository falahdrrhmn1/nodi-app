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
