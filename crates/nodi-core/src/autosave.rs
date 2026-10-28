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
//!
//! The Markdown mirror for AI agents is derived data and lowest priority: its
//! updates are collected and written once the writer has been idle for
//! [`MIRROR_DELAY`] (and once more on shutdown), so a burst of saves never
//! queues up mirror work and closing the app stays fast.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::id::NoteId;
use crate::mirror;
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
    /// Full Markdown-mirror rebuild from this `tree.json` text (Tools >
    /// Rebuild AI Markdown Mirror).
    MirrorRebuild(String),
    /// Startup sync from this `tree.json` text: only notes whose mirror file
    /// is missing or older than their RTF are converted again.
    MirrorSync(String),
    /// Enables or disables the Markdown mirror without restarting.
    MirrorEnabled(bool),
}

/// What a [`SaveResult`] refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Note(NoteId),
    Tree,
    Session,
    /// The Markdown mirror (per-note write or full rebuild). `seq` is `0`.
    Mirror,
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
    /// Queued mirror rebuild (tree JSON, `true` = full rebuild).
    mirror_rebuild: Option<(String, bool)>,
}

/// How long the writer must be idle before it updates the Markdown mirror.
pub const MIRROR_DELAY: Duration = Duration::from_millis(300);

/// Writer-side state of the Markdown mirror.
struct MirrorState {
    enabled: bool,
    /// The last tree written (or handed over by a rebuild/sync job), so a
    /// note save can update its mirror without the UI thread parsing the
    /// tree.
    tree: Option<Tree>,
    /// Where each note's mirror file was written last.
    layout: mirror::Layout,
    /// Notes saved since the last mirror pass, with their newest RTF.
    dirty: Vec<(NoteId, Vec<u8>)>,
    /// The tree changed since the last mirror pass.
    relayout: bool,
    /// An explicit request: `Some(true)` = full rebuild, `Some(false)` =
    /// startup sync that trusts mirror files newer than their note.
    requested: Option<bool>,
}

impl MirrorState {
    fn has_work(&self) -> bool {
        self.enabled && (!self.dirty.is_empty() || self.relayout || self.requested.is_some())
    }

    fn note_saved(&mut self, note: &NoteId, rtf: &[u8]) {
        match self.dirty.iter_mut().find(|(id, _)| id == note) {
            Some(entry) => entry.1 = rtf.to_vec(),
            None => self.dirty.push((note.clone(), rtf.to_vec())),
        }
    }

    /// Writes the collected mirror updates: the saved notes, then a re-plan
    /// of the whole mirror when the tree changed ([`mirror::sync`] converts
    /// only notes whose place changed) or a full rebuild when one was asked
    /// for. Only failures and full rebuilds are reported.
    fn run(&mut self, data: &DataDir, notify: &(dyn Fn(SaveResult) + Send)) {
        let dirty = std::mem::take(&mut self.dirty);
        let relayout = std::mem::take(&mut self.relayout);
        let requested = self.requested.take();
        if !self.enabled {
            return;
        }
        let Some(tree) = self.tree.as_ref() else {
            return;
        };
        if !dirty.is_empty() {
            let notes: Vec<(&NoteId, &[u8])> =
                dirty.iter().map(|(id, rtf)| (id, rtf.as_slice())).collect();
            if let Err(err) = mirror::write_many(data, tree, &notes) {
                notify(SaveResult {
                    target: Target::Mirror,
                    seq: 0,
                    result: Err(format!("mirror update failed: {err}")),
                });
            }
        }
        if !relayout && requested.is_none() {
            return;
        }
        let full = requested == Some(true);
        let outcome = if full {
            mirror::rebuild(data, tree).map(|_| mirror::layout(tree))
        } else {
            mirror::sync(data, tree, &self.layout, requested == Some(false))
        };
        let outcome = match outcome {
            Ok(layout) => {
                self.layout = layout;
                Ok(())
            }
            Err(err) => Err(format!("mirror update failed: {err}")),
        };
        if full || outcome.is_err() {
            notify(SaveResult {
                target: Target::Mirror,
                seq: 0,
                result: outcome,
            });
        }
    }
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
        self.notes.is_empty()
            && self.tree.is_none()
            && self.session.is_none()
            && self.mirror_rebuild.is_none()
    }

    /// Writes everything queued, in a stable order, reporting one result per
    /// written job.
    ///
    /// `last_written` remembers the highest sequence number that reached the
    /// disk per note. A request older than that is dropped: it was already
    /// superseded, and writing it would roll the note back. This can happen
    /// when two submissions arrive in separate drain passes.
    ///
    /// Mirror work is only recorded in `state` here; [`MirrorState::run`]
    /// does it once the writer is idle, so it never delays `Saved`.
    fn flush(
        &mut self,
        data: &DataDir,
        notify: &(dyn Fn(SaveResult) + Send),
        last_written: &mut HashMap<NoteId, u64>,
        state: &mut MirrorState,
    ) {
        let mut tree_written = false;
        if let Some(json) = self.tree.take() {
            match serde_json::from_str::<Tree>(&json) {
                Ok(tree) => {
                    let outcome = data
                        .atomic_write(&data.tree_path(), json.as_bytes())
                        .map_err(|err| err.to_string());
                    if outcome.is_ok() {
                        state.tree = Some(tree);
                        tree_written = true;
                    }
                    notify(SaveResult {
                        target: Target::Tree,
                        seq: 0,
                        result: outcome,
                    });
                }
                Err(err) => notify(SaveResult {
                    target: Target::Tree,
                    seq: 0,
                    result: Err(format!("refusing invalid tree.json: {err}")),
                }),
            }
        }

        let mut saved: Vec<SaveRequest> = Vec::new();
        for request in self.notes.drain(..) {
            if last_written
                .get(&request.note)
                .is_some_and(|seq| request.seq < *seq)
            {
                continue;
            }
            let result = data
                .save_note(&request.note, &request.rtf, &request.txt)
                .map_err(|err| err.to_string());
            let ok = result.is_ok();
            if ok {
                let highest = last_written.entry(request.note.clone()).or_insert(0);
                *highest = (*highest).max(request.seq);
            }
            notify(SaveResult {
                target: Target::Note(request.note.clone()),
                seq: request.seq,
                result,
            });
            if ok {
                saved.push(request);
            }
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

        if let Some((json, full)) = self.mirror_rebuild.take() {
            match serde_json::from_str::<Tree>(&json) {
                Ok(tree) => {
                    state.tree = Some(tree);
                    // A full rebuild also covers a sync.
                    if state.requested != Some(true) {
                        state.requested = Some(full);
                    }
                }
                Err(err) => notify(SaveResult {
                    target: Target::Mirror,
                    seq: 0,
                    result: Err(format!("refusing invalid mirror tree.json: {err}")),
                }),
            }
        }
        if state.enabled {
            state.relayout |= tree_written;
            for request in &saved {
                state.note_saved(&request.note, &request.rtf);
            }
        }
    }
}

fn run(data: DataDir, rx: Receiver<Message>, notify: Box<dyn Fn(SaveResult) + Send>) {
    // Upper bound for one drain pass; keeps latency bounded on huge bursts.
    const MAX_BATCH: usize = 100_000;

    let mut pending = Pending::default();
    let mut last_written: HashMap<NoteId, u64> = HashMap::new();
    let mut state = MirrorState {
        enabled: true,
        tree: None,
        layout: mirror::Layout::new(),
        dirty: Vec::new(),
        relayout: false,
        requested: None,
    };
    loop {
        let first = if state.has_work() {
            match rx.recv_timeout(MIRROR_DELAY) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => {
                    state.run(&data, notify.as_ref());
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(message) => message,
                Err(_) => break,
            }
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
                Message::Job(Job::MirrorRebuild(json)) => {
                    pending.mirror_rebuild = Some((json, true));
                }
                Message::Job(Job::MirrorSync(json)) => {
                    // A queued full rebuild already covers a sync.
                    if !pending
                        .mirror_rebuild
                        .as_ref()
                        .is_some_and(|(_, full)| *full)
                    {
                        pending.mirror_rebuild = Some((json, false));
                    }
                }
                Message::Job(Job::MirrorEnabled(enabled)) => {
                    state.enabled = enabled;
                    if !enabled {
                        // Work queued while the mirror is off is dropped so it
                        // cannot fire later behind the user's back.
                        state.dirty.clear();
                        state.relayout = false;
                        state.requested = None;
                    }
                }
                Message::Flush(ack) => waiters.push(ack),
                Message::Shutdown => stop = true,
            }
        }

        if !pending.is_empty() {
            pending.flush(&data, notify.as_ref(), &mut last_written, &mut state);
        }
        for waiter in waiters {
            let _ = waiter.send(());
        }
        if stop {
            break;
        }
    }
    // Whatever mirror work is still collected is written once, coalesced.
    state.run(&data, notify.as_ref());
}
