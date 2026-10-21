mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nodi_core::autosave::{Job, SaveRequest, SaveResult, Target, Writer};
use nodi_core::{DataDir, NodeKind, NoteId, Session, Tree};
use support::TempDir;

type Results = Arc<Mutex<Vec<SaveResult>>>;

fn sink() -> (Results, Box<dyn Fn(SaveResult) + Send>) {
    let results: Results = Arc::default();
    let handle = Arc::clone(&results);
    let notify: Box<dyn Fn(SaveResult) + Send> = Box::new(move |result| {
        handle.lock().expect("results lock").push(result);
    });
    (results, notify)
}

fn note_job(id: &NoteId, seq: u64, body: &str) -> Job {
    Job::Note(SaveRequest {
        note: id.clone(),
        seq,
        rtf: format!("{{\\rtf1 {body}}}").into_bytes(),
        txt: body.to_owned(),
    })
}

fn fresh(label: &str) -> (TempDir, DataDir) {
    let tmp = TempDir::new(label);
    let data = tmp.data_dir();
    data.ensure_layout().expect("layout");
    (tmp, data)
}

#[test]
fn fifty_rapid_saves_leave_the_newest_content_on_disk() {
    let (_tmp, data) = fresh("autosave-burst");
    let (results, notify) = sink();
    let writer = Writer::spawn(data.clone(), notify);
    let id = NoteId::generate();

    for seq in 1..=50u64 {
        assert!(writer.submit(note_job(&id, seq, &format!("body-{seq}"))));
    }
    assert!(
        writer.flush_blocking(Duration::from_secs(10)),
        "writer must acknowledge the flush"
    );

    assert_eq!(
        data.load_note(&id).expect("note content"),
        b"{\\rtf1 body-50}"
    );
    assert_eq!(data.load_note_text(&id).expect("sidecar"), "body-50");

    let results = results.lock().expect("results lock");
    assert!(!results.is_empty(), "at least one result must be reported");
    let last = results.last().expect("last result");
    assert_eq!(last.target, Target::Note(id.clone()));
    assert_eq!(last.seq, 50, "the newest sequence must be reported last");
    assert!(last.result.is_ok(), "last result: {:?}", last.result);
    for result in results.iter() {
        assert!(result.result.is_ok(), "unexpected failure: {result:?}");
        assert_eq!(result.target, Target::Note(id.clone()));
        assert!((1..=50).contains(&result.seq));
    }
}

#[test]
fn failed_save_reports_err_then_succeeds_when_the_fault_is_cleared() {
    let (_tmp, data) = fresh("autosave-fail");
    let (results, notify) = sink();
    let writer = Writer::spawn(data.clone(), notify);
    let id = NoteId::generate();

    data.set_fail_writes(true);
    assert!(writer.submit(note_job(&id, 1, "lost")));
    assert!(writer.flush_blocking(Duration::from_secs(10)));
    {
        let results = results.lock().expect("results lock");
        let last = results.last().expect("failure result");
        assert_eq!(last.seq, 1);
        let message = last.result.as_ref().expect_err("must be an error");
        assert!(
            message.contains("injected failure"),
            "unexpected error text: {message}"
        );
    }
    assert!(
        data.load_note(&id).is_none(),
        "nothing may be written while the fault is active"
    );

    data.set_fail_writes(false);
    assert!(writer.submit(note_job(&id, 2, "kept")));
    assert!(writer.flush_blocking(Duration::from_secs(10)));
    {
        let results = results.lock().expect("results lock");
        let last = results.last().expect("success result");
        assert_eq!(last.seq, 2);
        assert!(last.result.is_ok(), "second attempt: {:?}", last.result);
    }
    assert_eq!(data.load_note(&id).expect("note"), b"{\\rtf1 kept}");
    assert_eq!(data.load_note_text(&id).expect("txt"), "kept");
}

#[test]
fn newer_content_wins_even_when_requests_arrive_out_of_order() {
    let (_tmp, data) = fresh("autosave-order");
    let (results, notify) = sink();
    let writer = Writer::spawn(data.clone(), notify);
    let id = NoteId::generate();

    assert!(writer.submit(note_job(&id, 9, "nine")));
    assert!(writer.submit(note_job(&id, 4, "four")));
    assert!(writer.flush_blocking(Duration::from_secs(10)));

    assert_eq!(data.load_note(&id).expect("note"), b"{\\rtf1 nine}");
    let results = results.lock().expect("results lock");
    assert_eq!(results.last().expect("result").seq, 9);
}

#[test]
fn tree_and_session_jobs_are_written_and_validated() {
    let (_tmp, data) = fresh("autosave-tree");
    let (results, notify) = sink();
    let writer = Writer::spawn(data.clone(), notify);

    let mut tree = Tree::new();
    let folder = tree.new_node(NodeKind::Folder, "Inbox", None);
    let json = tree.to_json().expect("tree json");
    let session_json = serde_json::to_string(&Session::default()).expect("session json");

    assert!(writer.submit(Job::TreeSave(json)));
    assert!(writer.submit(Job::SessionSave(session_json)));
    assert!(writer.flush_blocking(Duration::from_secs(10)));

    assert_eq!(data.load_tree().expect("tree"), tree);
    assert_eq!(data.load_tree().expect("tree").children(None).len(), 1);
    assert!(data.session_path().is_file());
    {
        let results = results.lock().expect("results lock");
        assert!(results
            .iter()
            .any(|r| r.target == Target::Tree && r.result.is_ok()));
        assert!(results
            .iter()
            .any(|r| r.target == Target::Session && r.result.is_ok()));
    }

    // Invalid JSON is refused and the good file survives.
    assert!(writer.submit(Job::TreeSave("{ definitely not json".to_owned())));
    assert!(writer.flush_blocking(Duration::from_secs(10)));
    {
        let results = results.lock().expect("results lock");
        let last = results.last().expect("result");
        assert_eq!(last.target, Target::Tree);
        assert!(last.result.is_err(), "invalid JSON must be rejected");
    }
    assert_eq!(data.load_tree().expect("tree").children(None).len(), 1);
    assert!(data.load_tree().expect("tree").contains(&folder));
}

#[test]
fn dropping_the_writer_flushes_pending_jobs() {
    let (_tmp, data) = fresh("autosave-drop");
    let (_results, notify) = sink();
    let id = NoteId::generate();
    {
        let writer = Writer::spawn(data.clone(), notify);
        assert!(writer.submit(note_job(&id, 1, "written on drop")));
    }
    assert_eq!(
        data.load_note(&id).expect("note written during drop"),
        b"{\\rtf1 written on drop}"
    );
}

#[test]
fn empty_flush_is_acknowledged() {
    let (_tmp, data) = fresh("autosave-empty");
    let (_results, notify) = sink();
    let writer = Writer::spawn(data, notify);
    assert!(writer.flush_blocking(Duration::from_secs(5)));
}
