use nodi_core::{NodeKind, NoteId, Tree, TreeError};

fn chain() -> (Tree, NoteId, NoteId, NoteId) {
    let mut tree = Tree::new();
    let a = tree.new_node(NodeKind::Folder, "A", None);
    let b = tree.new_node(NodeKind::Folder, "B", Some(a.clone()));
    let c = tree.new_node(NodeKind::Note, "C", Some(b.clone()));
    (tree, a, b, c)
}

#[test]
fn rename_keeps_children_and_round_trip_preserves_everything() {
    let (mut tree, a, b, c) = chain();
    assert!(tree.set_icon(&a, Some("folder-star")));
    assert!(tree.rename(&b, "B renamed"));

    assert_eq!(tree.get(&c).expect("C exists").parent, Some(b.clone()));
    assert_eq!(
        tree.path_titles(&c),
        vec!["A".to_owned(), "B renamed".to_owned(), "C".to_owned()]
    );
    assert_eq!(tree.children(Some(&b)).len(), 1);

    let json = tree.to_json().expect("serialise");
    assert!(
        json.contains("\"version\":1"),
        "tree.json must carry the format version: {json}"
    );
    let back = Tree::from_json(&json).expect("deserialise");
    assert_eq!(back.len(), 3);
    assert_eq!(back, tree, "round-trip must preserve every field");
    assert_eq!(back.get(&c).expect("C exists").title, "C");
    assert_eq!(
        back.get(&a).expect("A exists").icon.as_deref(),
        Some("folder-star")
    );
    assert!(back.get(&a).expect("A exists").created > 0);
}

#[test]
fn serde_rejects_unknown_version() {
    let err = Tree::from_json(r#"{"version":2,"nodes":[]}"#);
    assert!(err.is_err(), "version 2 must be rejected");
    let empty = Tree::from_json(r#"{"version":1,"nodes":[]}"#).expect("version 1 parses");
    assert!(empty.is_empty());
}

#[test]
fn moving_a_node_into_its_own_descendant_is_rejected() {
    let (mut tree, a, b, c) = chain();

    assert_eq!(
        tree.move_node(&a, Some(c.clone()), 0),
        Err(TreeError::Cycle)
    );
    assert_eq!(
        tree.move_node(&a, Some(a.clone()), 0),
        Err(TreeError::Cycle)
    );
    assert_eq!(
        tree.move_node(&b, Some(c.clone()), 0),
        Err(TreeError::Cycle)
    );
    // Nothing changed.
    assert_eq!(tree.get(&a).expect("A exists").parent, None);
    assert_eq!(tree.get(&c).expect("C exists").parent, Some(b.clone()));

    // Moving a subtree sideways is fine.
    assert_eq!(tree.move_node(&b, None, 0), Ok(()));
    assert_eq!(tree.get(&b).expect("B exists").parent, None);
    assert_eq!(tree.path_titles(&c), vec!["B".to_owned(), "C".to_owned()]);
}

#[test]
fn notes_can_have_children_and_order_is_respected() {
    let mut tree = Tree::new();
    let parent_note = tree.new_node(NodeKind::Note, "Parent note", None);
    let first = tree.new_node(NodeKind::Note, "First", Some(parent_note.clone()));
    let second = tree.new_node(NodeKind::Note, "Second", Some(parent_note.clone()));
    let third = tree.new_node(NodeKind::Note, "Third", Some(parent_note.clone()));

    let children: Vec<&str> = tree
        .children(Some(&parent_note))
        .iter()
        .map(|node| node.title.as_str())
        .collect();
    assert_eq!(children, vec!["First", "Second", "Third"]);

    // Reorder: move "Third" to the front.
    tree.move_node(&third, Some(parent_note.clone()), 0)
        .expect("reorder");
    let children: Vec<&str> = tree
        .children(Some(&parent_note))
        .iter()
        .map(|node| node.title.as_str())
        .collect();
    assert_eq!(children, vec!["Third", "First", "Second"]);

    // Move "Second" to the end.
    tree.move_node(&second, Some(parent_note.clone()), 99)
        .expect("clamped index");
    let children: Vec<&str> = tree
        .children(Some(&parent_note))
        .iter()
        .map(|node| node.title.as_str())
        .collect();
    assert_eq!(children, vec!["Third", "First", "Second"]);
    assert_eq!(tree.descendants(&parent_note).len(), 3);
    assert!(tree.get(&first).is_some());
}

#[test]
fn trash_and_restore_work_on_the_whole_subtree() {
    let (mut tree, a, b, c) = chain();

    assert!(tree.trash(&a));
    assert!(tree.get(&a).expect("A exists").trashed);
    assert!(tree.get(&b).expect("B exists").trashed);
    assert!(tree.get(&c).expect("C exists").trashed);
    assert!(tree.children(None).is_empty());

    assert!(tree.restore(&a));
    assert!(!tree.get(&c).expect("C exists").trashed);
    assert_eq!(tree.children(None).len(), 1);
    assert_eq!(tree.children(Some(&a)).len(), 1);
    assert_eq!(tree.path_titles(&c).len(), 3);

    // Restoring a child whose parent is still trashed moves it to the root.
    assert!(tree.trash(&b));
    assert!(tree.restore(&c));
    let c_node = tree.get(&c).expect("C exists");
    assert_eq!(c_node.parent, None);
    assert!(!c_node.trashed);
    assert!(tree.children(None).iter().any(|node| node.id == c));
}

#[test]
fn purge_only_removes_trashed_subtrees() {
    let (mut tree, a, b, c) = chain();

    assert!(tree.purge(&a).is_empty(), "live nodes cannot be purged");
    assert_eq!(tree.len(), 3);

    tree.trash(&a);
    let removed = tree.purge(&a);
    assert_eq!(removed.len(), 3);
    assert!(removed.contains(&a) && removed.contains(&b) && removed.contains(&c));
    assert!(tree.is_empty());
    assert!(tree.get(&c).is_none());
}

#[test]
fn duplicate_copies_only_the_node() {
    let (mut tree, _a, b, c) = chain();
    let copy = tree.duplicate(&b).expect("B exists");
    assert_ne!(copy, b);
    let copy_node = tree.get(&copy).expect("copy exists");
    assert_eq!(copy_node.title, "B (copy)");
    assert_eq!(copy_node.parent, Some(_a.clone()));
    assert!(!copy_node.trashed);
    assert!(
        tree.children(Some(&copy)).is_empty(),
        "duplicate must not copy children"
    );
    // Original untouched.
    assert_eq!(tree.get(&c).expect("C exists").parent, Some(b));
}

#[test]
fn parents_that_are_missing_are_repaired_on_load() {
    let a = "a".repeat(32);
    let b = "b".repeat(32);
    let ghost = "c".repeat(32);
    let json = format!(
        r#"{{"version":1,"nodes":[
            {{"id":"{a}","kind":"folder","title":"A","icon":null,"parent":null,"order":0,"trashed":false,"created":1,"modified":1}},
            {{"id":"{b}","kind":"note","title":"B","icon":null,"parent":"{ghost}","order":0,"trashed":false,"created":1,"modified":1}}
        ]}}"#
    );
    let repaired = Tree::from_json(&json).expect("load repaired tree");
    assert_eq!(repaired.len(), 2);
    let b_id = NoteId::parse(&b).expect("valid id");
    assert_eq!(
        repaired.get(&b_id).expect("B exists").parent,
        None,
        "a dangling parent must fall back to the root"
    );
    for node in repaired.iter() {
        if let Some(parent) = &node.parent {
            assert!(repaired.contains(parent), "dangling parent left behind");
        }
    }
}

#[test]
fn cycles_are_broken_on_load() {
    let a = "a".repeat(32);
    let b = "b".repeat(32);
    let json = format!(
        r#"{{"version":1,"nodes":[
            {{"id":"{a}","kind":"folder","title":"A","icon":null,"parent":"{b}","order":0,"trashed":false,"created":1,"modified":1}},
            {{"id":"{b}","kind":"folder","title":"B","icon":null,"parent":"{a}","order":0,"trashed":false,"created":1,"modified":1}}
        ]}}"#
    );
    let repaired = Tree::from_json(&json).expect("load repaired tree");
    assert_eq!(repaired.len(), 2);
    let a_id = NoteId::parse(&a).expect("valid id");
    let b_id = NoteId::parse(&b).expect("valid id");
    assert!(
        repaired.ancestors(&a_id).is_empty() || repaired.ancestors(&b_id).is_empty(),
        "at least one side of the cycle must be cut"
    );
    assert!(!repaired.children(None).is_empty());
}

#[test]
fn moving_out_of_a_parent_leaves_dense_orders() {
    let mut tree = Tree::new();
    let root = tree.new_node(NodeKind::Folder, "Root", None);
    let a = tree.new_node(NodeKind::Note, "A", Some(root.clone()));
    let b = tree.new_node(NodeKind::Note, "B", Some(root.clone()));
    let c = tree.new_node(NodeKind::Note, "C", Some(root.clone()));
    let other = tree.new_node(NodeKind::Folder, "Other", None);

    tree.move_node(&b, Some(other.clone()), 0).expect("move B");
    let root_orders: Vec<i64> = tree
        .children(Some(&root))
        .iter()
        .map(|node| node.order)
        .collect();
    assert_eq!(root_orders, vec![0, 1]);
    let root_titles: Vec<&str> = tree
        .children(Some(&root))
        .iter()
        .map(|node| node.title.as_str())
        .collect();
    assert_eq!(root_titles, vec!["A", "C"]);
    assert_eq!(tree.children(Some(&other)).len(), 1);
    tree.move_node(&a, Some(other.clone()), 0).expect("move A");
    assert_eq!(tree.children(Some(&root)).len(), 1);
    assert_eq!(tree.children(Some(&other)).len(), 2);
    let _ = c;
}

#[test]
fn path_titles_and_ancestors_are_root_first() {
    let (tree, a, b, _c) = chain();
    assert_eq!(tree.path_titles(&a), vec!["A".to_owned()]);
    assert_eq!(tree.ancestors(&b), vec![a]);
    assert!(tree.path_titles(&NoteId::generate()).is_empty());
}

#[test]
fn move_among_siblings_skips_trashed_and_stops_at_the_ends() {
    let mut tree = Tree::new();
    let a = tree.new_node(NodeKind::Note, "A", None);
    let hidden = tree.new_node(NodeKind::Note, "Hidden", None);
    let b = tree.new_node(NodeKind::Note, "B", None);
    let c = tree.new_node(NodeKind::Note, "C", None);
    assert!(tree.trash(&hidden));
    let titles = |tree: &Tree| -> Vec<String> {
        tree.children(None)
            .iter()
            .map(|node| node.title.clone())
            .collect()
    };

    assert!(tree.move_among_siblings(&c, -1));
    assert_eq!(titles(&tree), vec!["A", "C", "B"]);
    assert!(tree.move_among_siblings(&c, -1));
    assert_eq!(titles(&tree), vec!["C", "A", "B"]);
    assert!(!tree.move_among_siblings(&c, -1), "already first");
    assert!(tree.move_among_siblings(&a, 1));
    assert_eq!(titles(&tree), vec!["C", "B", "A"]);
    assert!(!tree.move_among_siblings(&a, 1), "already last");
    let _ = b;
}

#[test]
fn auto_title_takes_the_first_line_without_markers() {
    use nodi_core::tree::auto_title;
    assert_eq!(auto_title("## Rapat mingguan\risi"), "Rapat mingguan");
    assert_eq!(auto_title("\r\r  •\tpoin satu\r"), "poin satu");
    assert_eq!(auto_title("- a   b\tc"), "a b c");
    assert_eq!(auto_title("\u{FFFC}\r"), "New Note");
    assert_eq!(auto_title(""), "New Note");
    assert_eq!(auto_title(&"x".repeat(100)).chars().count(), 60);
}

#[test]
fn only_automatic_titles_follow_the_text_and_renaming_stops_it() {
    let mut tree = Tree::new();
    let note = tree.new_node(NodeKind::Note, "New Note", None);
    assert_eq!(tree.follow_text(&note, "Judul"), None, "manual by default");
    tree.set_auto_title(&note, true);
    assert_eq!(
        tree.follow_text(&note, "Judul\risi").as_deref(),
        Some("Judul")
    );
    assert_eq!(tree.follow_text(&note, "Judul\rlain"), None, "unchanged");
    assert!(tree.rename(&note, "X"));
    assert_eq!(tree.follow_text(&note, "Baru"), None);
    assert_eq!(tree.get(&note).unwrap().title, "X");
    // Round trip through JSON keeps the flag; old files default to manual.
    tree.set_auto_title(&note, true);
    let back = Tree::from_json(&tree.to_json().unwrap()).unwrap();
    assert!(back.get(&note).unwrap().auto_title);
}

#[test]
fn undo_trash_brings_back_only_what_it_trashed_at_the_same_place() {
    let mut tree = Tree::new();
    let a = tree.new_node(NodeKind::Note, "A", None);
    let folder = tree.new_node(NodeKind::Folder, "Folder", None);
    let child = tree.new_node(NodeKind::Note, "Child", Some(folder.clone()));
    let old = tree.new_node(NodeKind::Note, "Old", Some(folder.clone()));
    tree.trash(&old);
    let change = tree.trash_undoable(&folder).unwrap();
    assert!(change.question(&tree).contains("\"Folder\" and 1 sub-note"));
    tree.undo(&change).unwrap();
    let roots: Vec<&str> = tree
        .children(None)
        .iter()
        .map(|n| n.title.as_str())
        .collect();
    assert_eq!(roots, vec!["A", "Folder"]);
    assert!(!tree.get(&child).unwrap().trashed);
    assert!(tree.get(&old).unwrap().trashed, "was trashed before");
    let _ = a;
}

#[test]
fn undo_move_and_rename() {
    let mut tree = Tree::new();
    let a = tree.new_node(NodeKind::Note, "A", None);
    let b = tree.new_node(NodeKind::Note, "B", None);
    let box_ = tree.new_node(NodeKind::Folder, "Box", None);
    let change = tree.move_undo(&a).unwrap();
    tree.move_node(&a, Some(box_.clone()), usize::MAX).unwrap();
    assert_eq!(
        change.question(&tree),
        "Undo: move \"A\" back to the top level?"
    );
    tree.undo(&change).unwrap();
    let roots: Vec<&str> = tree
        .children(None)
        .iter()
        .map(|n| n.title.as_str())
        .collect();
    assert_eq!(roots, vec!["A", "B", "Box"]);

    tree.set_auto_title(&b, true);
    let rename = tree.rename_undo(&b, "Baru").unwrap();
    tree.rename(&b, "Baru");
    assert_eq!(
        rename.question(&tree),
        "Undo: rename \"Baru\" back to \"B\"?"
    );
    tree.undo(&rename).unwrap();
    assert_eq!(tree.get(&b).unwrap().title, "B");
    assert!(tree.get(&b).unwrap().auto_title);
}
