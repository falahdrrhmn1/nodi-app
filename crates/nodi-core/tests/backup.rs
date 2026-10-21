mod support;

use std::fs;

use nodi_core::backup::{self, timestamp_name};
use nodi_core::{DataDir, NodeKind, NoteId, Tree};
use support::TempDir;

fn fresh(label: &str) -> (TempDir, DataDir) {
    let tmp = TempDir::new(label);
    let data = tmp.data_dir();
    data.ensure_layout().expect("layout");
    (tmp, data)
}

fn seed(data: &DataDir) -> (Tree, NoteId) {
    let mut tree = Tree::new();
    let folder = tree.new_node(NodeKind::Folder, "Inbox", None);
    let note = tree.new_node(NodeKind::Note, "Hello", Some(folder));
    data.save_tree(&tree).expect("save tree");
    data.save_note(&note, b"{\\rtf1 version one}", "version one")
        .expect("save note");
    let attachment = data.attachments_dir().join(note.as_str());
    fs::create_dir_all(&attachment).expect("attachments dir");
    fs::write(attachment.join("source.txt"), b"original source").expect("attachment");
    (tree, note)
}

#[test]
fn timestamp_names_are_utc() {
    assert_eq!(timestamp_name(0), "19700101-000000");
    assert_eq!(timestamp_name(1_700_000_000), "20231114-221320");
    assert_eq!(timestamp_name(1_700_000_001), "20231114-221321");
}

#[test]
fn two_snapshots_in_the_same_second_are_distinct_directories() {
    let (_tmp, data) = fresh("backup-same-second");
    seed(&data);

    let first = backup::snapshot(&data).expect("first snapshot");
    let second = backup::snapshot(&data).expect("second snapshot");

    let backups = data.backups_dir();
    assert_ne!(first, second, "snapshots must never overwrite each other");
    assert_eq!(first.parent(), Some(backups.as_path()));
    assert_eq!(second.parent(), Some(backups.as_path()));
    assert!(first.join("tree.json").is_file());
    assert!(first.join("notes").is_dir());
    assert!(first.join("attachments").is_dir());
    assert_eq!(backup::list(&data).expect("list").len(), 2);
}

#[test]
fn snapshots_contain_tree_notes_and_attachments() {
    let (_tmp, data) = fresh("backup-content");
    let (_tree, note) = seed(&data);
    let snap = backup::snapshot(&data).expect("snapshot");

    let note_copy = snap.join("notes").join(format!("{note}.rtf"));
    assert_eq!(
        fs::read(note_copy).expect("note in snapshot"),
        b"{\\rtf1 version one}"
    );
    assert!(snap
        .join("attachments")
        .join(note.as_str())
        .join("source.txt")
        .is_file());
}

#[test]
fn prune_keeps_the_newest_n_snapshots() {
    let (_tmp, data) = fresh("backup-prune");
    seed(&data);
    let mut created = Vec::new();
    for _ in 0..5 {
        created.push(backup::snapshot(&data).expect("snapshot"));
    }
    let listed = backup::list(&data).expect("list");
    assert_eq!(listed, created, "list is oldest first");

    let removed = backup::prune(&data, 2).expect("prune");
    assert_eq!(removed.len(), 3);
    for dir in &removed {
        assert!(
            !dir.exists(),
            "pruned snapshot still exists: {}",
            dir.display()
        );
    }
    let remaining = backup::list(&data).expect("list after prune");
    assert_eq!(remaining, created[3..].to_vec());

    assert!(backup::prune(&data, 10).expect("prune noop").is_empty());
    assert_eq!(backup::list(&data).expect("list").len(), 2);
}

#[test]
fn restore_round_trips_and_snapshots_the_state_it_replaces() {
    let (_tmp, data) = fresh("backup-restore");
    let (tree, note) = seed(&data);
    let snap = backup::snapshot(&data).expect("snapshot");

    // Diverge from the snapshot.
    data.save_note(&note, b"{\\rtf1 version two}", "version two")
        .expect("save v2");
    let mut renamed = tree.clone();
    assert!(renamed.rename(&note, "Renamed"));
    data.save_tree(&renamed).expect("save renamed tree");

    let safety = backup::restore(&data, &snap).expect("restore");
    assert!(safety.is_dir(), "restore must snapshot the replaced state");

    assert_eq!(
        data.load_note(&note).expect("restored note"),
        b"{\\rtf1 version one}"
    );
    assert_eq!(
        data.load_note_text(&note).expect("restored txt"),
        "version one"
    );
    let restored = data.load_tree().expect("restored tree");
    assert_eq!(restored.get(&note).expect("note").title, "Hello");
    assert!(restored.contains(&note));

    // The safety snapshot holds the state that was replaced.
    let safety_note = safety.join("notes").join(format!("{note}.rtf"));
    assert_eq!(
        fs::read(safety_note).expect("safety note"),
        b"{\\rtf1 version two}"
    );
    assert_eq!(backup::list(&data).expect("list").len(), 2);
}

#[test]
fn restore_of_a_missing_snapshot_fails() {
    let (_tmp, data) = fresh("backup-missing");
    seed(&data);
    let err = backup::restore(&data, &data.backups_dir().join("nope")).expect_err("must fail");
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
}
