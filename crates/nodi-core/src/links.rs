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

#[cfg(test)]
mod tests {
    use super::*;

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
