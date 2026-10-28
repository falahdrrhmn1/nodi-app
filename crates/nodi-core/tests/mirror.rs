mod support;

use std::fs;

use nodi_core::mirror;
use nodi_core::{DataDir, NodeKind, Tree};
use support::TempDir;

const RTF: &[u8] = br"{\rtf1\ansi Isi catatan\par}";

fn fresh(label: &str) -> (TempDir, DataDir) {
    let tmp = TempDir::new(label);
    let data = tmp.data_dir();
    data.ensure_layout().expect("layout");
    (tmp, data)
}

fn read(data: &DataDir, relative: &str) -> String {
    fs::read_to_string(data.markdown_dir().join(relative)).expect(relative)
}

#[test]
fn rebuild_writes_notes_index_and_front_matter() {
    let (_tmp, data) = fresh("mirror-rebuild");
    let mut tree = Tree::new();
    let folder = tree.new_node(NodeKind::Folder, "Kuliah", None);
    let note = tree.new_node(NodeKind::Note, "Rapat: 1/2", Some(folder.clone()));
    data.save_note(&note, RTF, "Isi catatan").expect("save");

    mirror::rebuild(&data, &tree).expect("rebuild");

    let body = read(&data, "Kuliah/Rapat_ 1_2.md");
    assert!(body.starts_with("---\n"), "{body}");
    assert!(body.contains(&format!("id: {note}")), "{body}");
    assert!(body.contains("Isi catatan"), "{body}");
    assert!(read(&data, "INDEX.md").contains("Rapat"));
    assert!(read(&data, "index.json").contains(&note.to_string()));
}

#[test]
fn sync_moves_renamed_notes_and_keeps_the_old_file_in_trash() {
    let (_tmp, data) = fresh("mirror-sync");
    let mut tree = Tree::new();
    let note = tree.new_node(NodeKind::Note, "Lama", None);
    data.save_note(&note, RTF, "Isi catatan").expect("save");
    let layout = mirror::sync(&data, &tree, &mirror::Layout::new(), false).expect("sync");
    assert!(data.markdown_dir().join("Lama.md").is_file());

    tree.rename(&note, "Baru");
    mirror::sync(&data, &tree, &layout, false).expect("sync");

    assert!(read(&data, "Baru.md").contains("Isi catatan"));
    assert!(!data.markdown_dir().join("Lama.md").exists());
    let trash = data.markdown_dir().join(".trash");
    let kept = fs::read_dir(&trash)
        .expect("trash")
        .filter_map(Result::ok)
        .any(|stamp| stamp.path().join("Lama.md").is_file());
    assert!(kept, "old mirror file must be kept in .trash");
}

#[test]
fn sync_skips_unchanged_notes() {
    let (_tmp, data) = fresh("mirror-skip");
    let mut tree = Tree::new();
    let note = tree.new_node(NodeKind::Note, "Tetap", None);
    data.save_note(&note, RTF, "Isi catatan").expect("save");
    let layout = mirror::sync(&data, &tree, &mirror::Layout::new(), false).expect("sync");

    // A marker written into the mirror file survives a sync with the same
    // layout: the note was not converted again.
    let path = data.markdown_dir().join("Tetap.md");
    fs::write(&path, "penanda").expect("write");
    tree.new_node(NodeKind::Note, "Lain", None);
    mirror::sync(&data, &tree, &layout, false).expect("sync");
    assert_eq!(fs::read_to_string(&path).expect("read"), "penanda");
    assert!(data.markdown_dir().join("Lain.md").is_file());
}

#[test]
fn startup_sync_trusts_newer_mirror_files_only() {
    let (_tmp, data) = fresh("mirror-startup");
    let mut tree = Tree::new();
    let note = tree.new_node(NodeKind::Note, "Catatan", None);
    data.save_note(&note, RTF, "Isi catatan").expect("save");
    mirror::sync(&data, &tree, &mirror::Layout::new(), false).expect("sync");

    // The note changes after its mirror was written (e.g. while the mirror
    // was switched off): the startup sync converts it again.
    std::thread::sleep(std::time::Duration::from_millis(20));
    data.save_note(&note, br"{\rtf1\ansi Versi dua\par}", "Versi dua")
        .expect("save");
    mirror::sync(&data, &tree, &mirror::Layout::new(), true).expect("sync");
    assert!(read(&data, "Catatan.md").contains("Versi dua"));
}

#[test]
fn trashed_notes_leave_the_mirror() {
    let (_tmp, data) = fresh("mirror-trash");
    let mut tree = Tree::new();
    let note = tree.new_node(NodeKind::Note, "Buang", None);
    data.save_note(&note, RTF, "Isi catatan").expect("save");
    let layout = mirror::sync(&data, &tree, &mirror::Layout::new(), false).expect("sync");

    tree.trash(&note);
    mirror::sync(&data, &tree, &layout, false).expect("sync");
    assert!(!data.markdown_dir().join("Buang.md").exists());
    assert!(!read(&data, "index.json").contains(&note.to_string()));
}
