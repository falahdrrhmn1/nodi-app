//! Search: titles from the tree, full text from the `.txt` sidecars.

use crate::id::NoteId;
use crate::storage::DataDir;
use crate::tree::{Node, NodeKind, Tree};

/// Case-insensitive substring search over visible node titles, in tree order
/// (folders and notes alike).
pub fn search_titles<'a>(tree: &'a Tree, query: &str) -> Vec<&'a Node> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    tree.iter_ordered()
        .into_iter()
        .filter(|node| !node.trashed && node.title.to_lowercase().contains(&needle))
        .collect()
}

/// Case-insensitive substring search over the plain-text sidecars of every
/// visible note. Returns `(note, number of hits)`, sorted by note tree order.
pub fn search_text(data: &DataDir, tree: &Tree, query: &str) -> Vec<(NoteId, usize)> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut results = Vec::new();
    for node in tree.iter_ordered() {
        if node.trashed || node.kind != NodeKind::Note {
            continue;
        }
        let Some(text) = data.load_note_text(&node.id) else {
            continue;
        };
        let hits = count_hits(&text, &needle);
        if hits > 0 {
            results.push((node.id.clone(), hits));
        }
    }
    results
}

/// Counts non-overlapping matches of an already lowercased needle.
fn count_hits(haystack: &str, needle_lower: &str) -> usize {
    if needle_lower.is_empty() {
        return 0;
    }
    haystack.to_lowercase().match_indices(needle_lower).count()
}
