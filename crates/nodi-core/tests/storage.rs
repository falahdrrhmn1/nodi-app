mod support;

use std::fs;
use std::io;

use nodi_core::storage::{Session, TabState, WindowRect, DEFAULT_SIDEBAR_WIDTH};
use nodi_core::{DataDir, NodeKind, NoteId, Tree};
use support::TempDir;

fn fresh(label: &str) -> (TempDir, DataDir) {
    let tmp = TempDir::new(label);
    let data = tmp.data_dir();
    data.ensure_layout().expect("layout");
    (tmp, data)
}

fn tmp_files(dir: &std::path::Path) -> Vec<String> {
    fs::read_dir(dir)
        .expect("read dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().to_string_lossy().ends_with(".tmp"))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn layout_has_every_folder_from_the_spec() {
    let (_tmp, data) = fresh("layout");
    for dir in [
        data.root().to_path_buf(),
        data.notes_dir(),
        data.attachments_dir(),
        data.backups_dir(),
        data.logs_dir(),
        data.recovery_dir(),
    ] {
        assert!(dir.is_dir(), "{} should exist", dir.display());
    }
}

#[test]
fn note_save_load_round_trip_and_prev_generation() {
    let (_tmp, data) = fresh("note-roundtrip");
    let id = NoteId::generate();

    assert!(data.load_note(&id).is_none());
    data.save_note(&id, b"{\\rtf1 first}", "first")
        .expect("first save");
    assert_eq!(data.load_note(&id).expect("load"), b"{\\rtf1 first}");
    assert_eq!(data.load_note_text(&id).expect("txt"), "first");
    assert!(
        data.load_note_prev(&id).is_none(),
        "the first save has no previous generation"
    );
    assert!(
        tmp_files(&data.notes_dir()).is_empty(),
        "no scratch files may survive a successful save"
    );

    data.save_note(&id, b"{\\rtf1 second}", "second")
        .expect("second save");
    assert_eq!(data.load_note(&id).expect("load"), b"{\\rtf1 second}");
    assert_eq!(
        data.load_note_prev(&id).expect("prev"),
        b"{\\rtf1 first}",
        ".prev must hold the previous good version"
    );
    assert_eq!(data.load_note_text(&id).expect("txt"), "second");
}

#[test]
fn failure_injection_returns_err_and_keeps_the_original() {
    let (_tmp, data) = fresh("fail-writes");
    let id = NoteId::generate();
    data.save_note(&id, b"keep me", "keep me")
        .expect("initial save");

    data.set_fail_writes(true);
    let err = data
        .atomic_write(&data.note_path(&id), b"must not land")
        .expect_err("injected failure");
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert!(
        err.to_string().contains("injected failure"),
        "unexpected message: {err}"
    );
    assert_eq!(data.load_note(&id).expect("load"), b"keep me");

    let err = data
        .save_note(&id, b"also must not land", "nope")
        .expect_err("injected failure");
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert_eq!(data.load_note(&id).expect("load"), b"keep me");
    assert_eq!(data.load_note_text(&id).expect("txt"), "keep me");
    assert!(
        tmp_files(&data.notes_dir()).is_empty(),
        "a failed write must not leave scratch files"
    );

    data.set_fail_writes(false);
    data.save_note(&id, b"written after recovery", "ok")
        .expect("save works again");
    assert_eq!(
        data.load_note(&id).expect("load"),
        b"written after recovery"
    );
}

#[test]
fn tree_round_trip_through_disk() {
    let (_tmp, data) = fresh("tree-disk");
    assert!(data.load_tree().expect("empty tree").is_empty());

    let mut tree = Tree::new();
    let folder = tree.new_node(NodeKind::Folder, "Inbox", None);
    let note = tree.new_node(NodeKind::Note, "Hello", Some(folder.clone()));
    data.save_tree(&tree).expect("save tree");
    assert!(data.tree_path().is_file());

    let loaded = data.load_tree().expect("load tree");
    assert_eq!(loaded, tree);
    assert_eq!(loaded.path_titles(&note), vec!["Inbox", "Hello"]);
}

#[test]
fn corrupt_tree_is_an_error_not_an_empty_tree() {
    let (_tmp, data) = fresh("tree-corrupt");
    fs::write(data.tree_path(), "{ not json").expect("plant corrupt file");
    let err = data.load_tree().expect_err("corrupt tree must fail");
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn session_round_trip() {
    let (_tmp, data) = fresh("session");
    let default = data.load_session().expect("default session");
    assert_eq!(default, Session::default());
    assert_eq!(default.sidebar_width, DEFAULT_SIDEBAR_WIDTH);

    let note_a = NoteId::generate();
    let note_b = NoteId::generate();
    let session = Session {
        tabs: vec![
            TabState {
                note: note_a.clone(),
                caret: 12,
                scroll: 3,
            },
            TabState {
                note: note_b.clone(),
                caret: 0,
                scroll: 0,
            },
        ],
        active: 1,
        sidebar_width: 311,
        window: Some(WindowRect {
            x: 40,
            y: 60,
            width: 1024,
            height: 720,
        }),
    };
    data.save_session(&session).expect("save session");
    let loaded = data.load_session().expect("load session");
    assert_eq!(loaded, session);
    assert_eq!(loaded.tabs[0].note, note_a);
}

#[test]
fn session_active_index_is_clamped() {
    let (_tmp, data) = fresh("session-clamp");
    fs::write(
        data.session_path(),
        r#"{"tabs":[],"active":7,"sidebar_width":200,"window":null}"#,
    )
    .expect("plant session");
    let loaded = data.load_session().expect("load");
    assert_eq!(loaded.active, 0);
    assert_eq!(loaded.tabs.len(), 0);
    assert_eq!(loaded.sidebar_width, 200);
}

#[test]
fn recover_leftovers_moves_tmp_files_and_never_deletes_them() {
    let (_tmp, data) = fresh("recovery");
    let id = NoteId::generate();
    data.save_note(&id, b"good content", "good").expect("save");

    let planted = data.notes_dir().join(format!("{id}.rtf.tmp"));
    fs::write(&planted, b"half written").expect("plant tmp");
    let nested = data.notes_dir().join("sub");
    fs::create_dir_all(&nested).expect("nested dir");
    fs::write(nested.join("nested.rtf.tmp"), b"nested").expect("plant nested tmp");

    let recovered = data.recover_leftovers().expect("recover");
    assert_eq!(recovered.len(), 2, "both tmp files must be recovered");
    assert!(!planted.exists(), "the planted tmp must be moved away");
    assert!(tmp_files(&data.notes_dir()).is_empty());

    for path in &recovered {
        assert!(
            path.starts_with(data.recovery_dir()),
            "recovered file outside recovery dir: {}",
            path.display()
        );
        let name = path.file_name().expect("name").to_string_lossy();
        assert!(name.starts_with("recovered-"), "bad name: {name}");
        assert!(!fs::read(path).expect("read recovered").is_empty());
    }
    let contents: Vec<Vec<u8>> = recovered
        .iter()
        .map(|path| fs::read(path).expect("read recovered"))
        .collect();
    assert!(contents
        .iter()
        .any(|bytes| bytes.as_slice() == b"half written"));
    assert!(contents.iter().any(|bytes| bytes.as_slice() == b"nested"));

    // The good note was not touched.
    assert_eq!(data.load_note(&id).expect("load"), b"good content");
}

#[test]
fn recover_leftovers_recovers_same_named_files_twice_without_overwriting() {
    let (_tmp, data) = fresh("recovery-dup");
    let planted = data.notes_dir().join("stuck.rtf.tmp");
    fs::write(&planted, b"one").expect("plant");
    let first = data.recover_leftovers().expect("first recover");
    fs::write(&planted, b"two").expect("plant again");
    let second = data.recover_leftovers().expect("second recover");
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_ne!(first[0], second[0], "recovered names must not collide");
    assert_eq!(fs::read(&first[0]).expect("read"), b"one");
    assert_eq!(fs::read(&second[0]).expect("read"), b"two");
}
