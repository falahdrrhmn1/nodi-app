mod support;

use nodi_core::links::{format_note_link, parse_note_link, LINK_PREFIX};
use nodi_core::search::{search_text, search_titles};
use nodi_core::{DataDir, NodeKind, NoteId, Tree};
use support::TempDir;

fn fresh(label: &str) -> (TempDir, DataDir) {
    let tmp = TempDir::new(label);
    let data = tmp.data_dir();
    data.ensure_layout().expect("layout");
    (tmp, data)
}

#[test]
fn title_search_is_case_insensitive_and_skips_the_trash() {
    let mut tree = Tree::new();
    let folder = tree.new_node(NodeKind::Folder, "Shopping List", None);
    let note = tree.new_node(NodeKind::Note, "shopping receipt", Some(folder.clone()));
    let other = tree.new_node(NodeKind::Note, "Unrelated", None);
    let trashed = tree.new_node(NodeKind::Note, "SHOPPING in the trash", None);
    assert!(tree.trash(&trashed));

    let hits = search_titles(&tree, "ShOpPiNg");
    let ids: Vec<NoteId> = hits.iter().map(|node| node.id.clone()).collect();
    assert_eq!(ids, vec![folder, note]);
    assert!(!ids.contains(&other));

    assert!(search_titles(&tree, "missing").is_empty());
    assert!(search_titles(&tree, "").is_empty());
    assert!(search_titles(&tree, "   ").is_empty());
}

#[test]
fn text_search_counts_hits_in_plain_text_sidecars() {
    let (_tmp, data) = fresh("search-text");
    let mut tree = Tree::new();

    let first = tree.new_node(NodeKind::Note, "First", None);
    data.save_note(&first, b"{\\rtf1}", "Alpha beta ALPHA gamma alpha")
        .expect("save first");
    let second = tree.new_node(NodeKind::Note, "Second", None);
    data.save_note(&second, b"{\\rtf1}", "nothing to see")
        .expect("save second");
    let trashed = tree.new_node(NodeKind::Note, "Gone", None);
    data.save_note(&trashed, b"{\\rtf1}", "alpha alpha")
        .expect("save trashed");
    assert!(tree.trash(&trashed));
    // A folder with content is never searched.
    let folder = tree.new_node(NodeKind::Folder, "Folder", None);
    data.save_note(&folder, b"{\\rtf1}", "alpha")
        .expect("save folder sidecar");

    let hits = search_text(&data, &tree, "aLpHa");
    assert_eq!(hits, vec![(first.clone(), 3)]);
    assert!(search_text(&data, &tree, "alpha").len() == 1);
    assert!(search_text(&data, &tree, "").is_empty());
    assert!(search_text(&data, &tree, "not-there").is_empty());
    let _ = second;
}

#[test]
fn links_round_trip_and_reject_foreign_urls() {
    let id = NoteId::generate();
    let link = format_note_link(&id);
    assert_eq!(link, format!("{LINK_PREFIX}{id}"));
    assert_eq!(parse_note_link(&link), Some(id.clone()));
    assert_eq!(parse_note_link(&format!("  {link}  ")), Some(id.clone()));

    assert_eq!(parse_note_link(""), None);
    assert_eq!(parse_note_link("nodi://note/"), None);
    assert_eq!(parse_note_link("nodi://note/not-an-id"), None);
    assert_eq!(parse_note_link("https://example.com/note"), None);
    assert_eq!(parse_note_link(&format!("nodi://folder/{id}")), None);
    assert_eq!(
        parse_note_link(&format!("nodi://note/{}", id.as_str().to_uppercase())),
        None,
        "ids are lowercase only"
    );
}
