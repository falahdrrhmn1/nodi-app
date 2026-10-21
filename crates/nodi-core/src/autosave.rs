//! Single background writer thread.
//!
//! The UI thread streams RTF out of the editor, bumps a per-note sequence
//! number and submits a [`SaveRequest`]. The writer coalesces queued requests
//! for the same note (only the newest content needs to be written), writes it
//! through [`DataDir`]'s atomic path and reports a [`SaveResult`] for the newest
//! sequence number.
//!
//! The writer never blocks on the UI: results are delivered through the
//! callback given to [`Writer::spawn`] (the shell wraps it in `PostMessageW`).

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::id::NoteId;
use crate::storage::{DataDir, Session};
use crate::tree::Tree;

/// One note save, produced by the UI thread.
#[derive(Clone, Debug)]
pub struct SaveRequest {
    pub note: NoteId,
    /// Monotonically increasing per note. The UI only applies the result when
    /// it matches the note's latest sequence number.
    pub seq: u64,
    /// RTF streamed out of the Rich Edit control.
    pub rtf: Vec<u8>,
    /// Plain text sidecar used by search.
    pub txt: String,
}

/// A unit of work for the writer.
#[derive(Debug)]
pub enum Job {
    Note(SaveRequest),
    TreeSave(String),
    SessionSave(String),
}

/// What a [`SaveResult`] refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Note(NoteId),
    Tree,
    Session,
}

/// Outcome of one write, delivered on the writer thread.
#[derive(Debug)]
pub struct SaveResult {
    pub target: Target,
    /// The sequence number of the request that produced this result. `0` for
    /// tree and session saves (they have no sequence).
    pub seq: u64,
    pub result: Result<(), String>,
}

/// Handle to the background writer. Dropping it flushes and joins.
pub struct Writer {
    tx: Option<Sender<Message>>,
    handle: Option<JoinHandle<()>>,
}

enum Message {
    Job(Job),
    Flush(Sender<()>),
    Shutdown,
}

impl Writer {
    /// Starts the writer thread. A panic-free failure mode is impossible here:
    /// if the OS cannot create the thread, saving cannot be safe, so this
    /// panics immediately instead of losing data later.
    pub fn spawn(data: DataDir, notify: Box<dyn Fn(SaveResult) + Send>) -> Self {
        let (tx, rx) = mpsc::channel();
        let handle = thread::Builder::new()
            .name("nodi-autosave".to_owned())
            .spawn(move || run(data, rx, notify))
            .expect("failed to spawn the autosave writer thread");
        Self {
            tx: Some(tx),
            handle: Some(handle),
        }
    }

    /// Queues a job. Returns false when the writer is already shut down.
    pub fn submit(&self, job: Job) -> bool {
        match &self.tx {
            Some(tx) => tx.send(Message::Job(job)).is_ok(),
            None => false,
        }
    }

    /// Blocks until every job submitted *before* this call has been written,
    /// or until `timeout` expires. Returns true when the writer acknowledged.
    pub fn flush_blocking(&self, timeout: Duration) -> bool {
        let Some(tx) = &self.tx else {
            return false;
        };
        let (ack_tx, ack_rx) = mpsc::channel();
        if tx.send(Message::Flush(ack_tx)).is_err() {
            return false;
        }
        matches!(ack_rx.recv_timeout(timeout), Ok(()))
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Message::Shutdown);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Default)]
struct Pending {
    /// Insertion ordered, at most one request per note.
    notes: Vec<SaveRequest>,
    tree: Option<String>,
    session: Option<String>,
}

impl Pending {
    fn push_note(&mut self, request: SaveRequest) {
        if let Some(existing) = self.notes.iter_mut().find(|r| r.note == request.note) {
            if request.seq >= existing.seq {
                *existing = request;
            }
        } else {
            self.notes.push(request);
        }
    }

    fn is_empty(&self) -> bool {
        self.notes.is_empty() && self.tree.is_none() && self.session.is_none()
    }

    /// Writes everything queued, in a stable order, reporting one result per
    /// written job.
    fn flush(&mut self, data: &DataDir, notify: &(dyn Fn(SaveResult) + Send)) {
        for request in self.notes.drain(..) {
            let result = data
                .save_note(&request.note, &request.rtf, &request.txt)
                .map_err(|err| err.to_string());
            notify(SaveResult {
                target: Target::Note(request.note),
                seq: request.seq,
                result,
            });
        }

        if let Some(json) = self.tree.take() {
            let outcome = match serde_json::from_str::<Tree>(&json) {
                Ok(_) => data
                    .atomic_write(&data.tree_path(), json.as_bytes())
                    .map_err(|err| err.to_string()),
                Err(err) => Err(format!("refusing invalid tree.json: {err}")),
            };
            notify(SaveResult {
                target: Target::Tree,
                seq: 0,
                result: outcome,
            });
        }

        if let Some(json) = self.session.take() {
            let outcome = match serde_json::from_str::<Session>(&json) {
                Ok(_) => data
                    .atomic_write(&data.session_path(), json.as_bytes())
                    .map_err(|err| err.to_string()),
                Err(err) => Err(format!("refusing invalid session.json: {err}")),
            };
            notify(SaveResult {
                target: Target::Session,
                seq: 0,
                result: outcome,
            });
        }
    }
}

fn run(data: DataDir, rx: Receiver<Message>, notify: Box<dyn Fn(SaveResult) + Send>) {
    // Upper bound for one drain pass; keeps latency bounded on huge bursts.
    const MAX_BATCH: usize = 100_000;

    let mut pending = Pending::default();
    loop {
        let first = match rx.recv() {
            Ok(message) => message,
            Err(_) => break,
        };

        let mut batch = vec![first];
        while batch.len() < MAX_BATCH {
            match rx.try_recv() {
                Ok(message) => batch.push(message),
                Err(_) => break,
            }
        }

        let mut waiters: Vec<Sender<()>> = Vec::new();
        let mut stop = false;
        for message in batch {
            match message {
                Message::Job(Job::Note(request)) => pending.push_note(request),
                Message::Job(Job::TreeSave(json)) => pending.tree = Some(json),
                Message::Job(Job::SessionSave(json)) => pending.session = Some(json),
                Message::Flush(ack) => waiters.push(ack),
                Message::Shutdown => stop = true,
            }
        }

        if !pending.is_empty() {
            pending.flush(&data, notify.as_ref());
        }
        for waiter in waiters {
            let _ = waiter.send(());
        }
        if stop {
            break;
        }
    }
}
