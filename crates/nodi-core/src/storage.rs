//! Data directory layout and the only code path allowed to write user data.
//!
//! Layout (see `docs/ARCHITECTURE.md`):
//!
//! ```text
//! data/
//!   tree.json
//!   session.json
//!   notes/<id>.rtf
//!   notes/<id>.txt
//!   notes/<id>.rtf.prev
//!   attachments/<id>/
//!   backups/<yyyymmdd-hhmmss>/
//!   logs/nodi.log
//!   recovery/
//! ```
//!
//! Every write is atomic: `<file>.tmp` → `write_all` → `sync_all` → optional
//! `.prev` copy → `rename`. A failed write leaves the original file untouched.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::id::{now_secs, NoteId};
use crate::tree::Tree;

/// Message used by the failure-injection switch.
pub const INJECTED_FAILURE: &str = "injected failure";

/// Default width of the notes sidebar in logical pixels.
pub const DEFAULT_SIDEBAR_WIDTH: i32 = 240;

/// Handle to the portable data directory.
#[derive(Clone, Debug)]
pub struct DataDir {
    root: PathBuf,
    /// Test/diagnostic switch: when true [`DataDir::atomic_write`] fails before
    /// touching the disk. Production code never sets it.
    pub fail_writes: Arc<AtomicBool>,
}

impl DataDir {
    /// Resolves the data directory: `NODI_DATA_DIR` if set, else
    /// `<exe_dir>\data`. Never AppData.
    ///
    /// Exception: an exe running straight out of a Cargo `target\<profile>`
    /// folder uses `<workspace>\data`, because `cargo clean` deletes `target`
    /// and would take the notes with it.
    pub fn resolve() -> Self {
        if let Some(dir) = std::env::var_os("NODI_DATA_DIR") {
            if !dir.is_empty() {
                return Self::at(dir);
            }
        }
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        Self::at(data_dir_for_exe_dir(&exe_dir))
    }

    /// Uses an explicit directory (tests, portable mode, restores).
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            fail_writes: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The data directory itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `tree.json`
    pub fn tree_path(&self) -> PathBuf {
        self.root.join("tree.json")
    }

    /// `session.json`
    pub fn session_path(&self) -> PathBuf {
        self.root.join("session.json")
    }

    /// `notes/`
    pub fn notes_dir(&self) -> PathBuf {
        self.root.join("notes")
    }

    /// `attachments/`
    pub fn attachments_dir(&self) -> PathBuf {
        self.root.join("attachments")
    }

    /// `backups/`
    pub fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }

    /// `logs/`
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// `logs/nodi.log`
    pub fn log_path(&self) -> PathBuf {
        self.logs_dir().join("nodi.log")
    }

    /// `recovery/`
    pub fn recovery_dir(&self) -> PathBuf {
        self.root.join("recovery")
    }

    /// `notes/<id>.rtf`
    pub fn note_path(&self, id: &NoteId) -> PathBuf {
        self.notes_dir().join(format!("{id}.rtf"))
    }

    /// `notes/<id>.txt`
    pub fn note_txt_path(&self, id: &NoteId) -> PathBuf {
        self.notes_dir().join(format!("{id}.txt"))
    }

    /// `notes/<id>.rtf.prev`
    pub fn note_prev_path(&self, id: &NoteId) -> PathBuf {
        self.notes_dir().join(format!("{id}.rtf.prev"))
    }

    /// Creates every subdirectory of the layout (idempotent).
    pub fn ensure_layout(&self) -> io::Result<()> {
        for dir in [
            self.root.clone(),
            self.notes_dir(),
            self.attachments_dir(),
            self.backups_dir(),
            self.logs_dir(),
            self.recovery_dir(),
        ] {
            fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    /// Convenience for tests: toggles the failure injection switch.
    pub fn set_fail_writes(&self, fail: bool) {
        self.fail_writes.store(fail, Ordering::SeqCst);
    }

    /// Atomic write without keeping a previous generation.
    pub fn atomic_write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.write_atomic(path, bytes, false)
    }

    /// Atomic write that copies the current file to `<path>.prev` first.
    /// Only note content uses this (one previous generation).
    pub fn atomic_write_keep_prev(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.write_atomic(path, bytes, true)
    }

    fn write_atomic(&self, path: &Path, bytes: &[u8], keep_prev: bool) -> io::Result<()> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(io::Error::other(INJECTED_FAILURE));
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let tmp = sibling(path, ".tmp");
        let outcome = (|| -> io::Result<()> {
            {
                let mut file = fs::File::create(&tmp)?;
                file.write_all(bytes)?;
                file.sync_all()?;
            }
            if keep_prev && path.exists() {
                fs::copy(path, sibling(path, ".prev"))?;
            }
            fs::rename(&tmp, path)?;
            Ok(())
        })();

        if outcome.is_err() {
            // Only ever remove the temp file this call created.
            let _ = fs::remove_file(&tmp);
        }
        outcome
    }

    /// Reads `tree.json`; a missing file is an empty tree, a corrupt file is
    /// an error (the UI offers a backup instead of silently starting empty).
    pub fn load_tree(&self) -> io::Result<Tree> {
        let path = self.tree_path();
        match fs::read_to_string(&path) {
            Ok(text) => Tree::from_json(&text).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{} is corrupt: {err}", path.display()),
                )
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Tree::new()),
            Err(err) => Err(err),
        }
    }

    /// Saves `tree.json` atomically.
    pub fn save_tree(&self, tree: &Tree) -> io::Result<()> {
        let json = tree.to_json().map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("cannot serialise tree: {err}"),
            )
        })?;
        self.atomic_write(&self.tree_path(), json.as_bytes())
    }

    /// Reads `notes/<id>.rtf`. `None` when missing or unreadable.
    pub fn load_note(&self, id: &NoteId) -> Option<Vec<u8>> {
        fs::read(self.note_path(id)).ok()
    }

    /// Reads the previous generation `notes/<id>.rtf.prev`.
    pub fn load_note_prev(&self, id: &NoteId) -> Option<Vec<u8>> {
        fs::read(self.note_prev_path(id)).ok()
    }

    /// Reads the plain-text sidecar used by search.
    pub fn load_note_text(&self, id: &NoteId) -> Option<String> {
        let bytes = fs::read(self.note_txt_path(id)).ok()?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Moves every content file of a permanently deleted (purged) note into
    /// `recovery/purged-<id>-<ts>/`. Nothing is ever deleted: if the move
    /// fails the file is copied, leaving the original in place. Returns the
    /// recovery directory when at least one file existed.
    pub fn purge_note_files(&self, id: &NoteId) -> io::Result<Option<PathBuf>> {
        let candidates = [
            self.note_path(id),
            self.note_txt_path(id),
            self.note_prev_path(id),
        ];
        if candidates.iter().all(|path| !path.exists()) {
            return Ok(None);
        }
        self.ensure_layout()?;

        let ts = now_secs();
        let mut dest = self.recovery_dir().join(format!("purged-{id}-{ts}"));
        let mut attempt = 1usize;
        while dest.exists() {
            dest = self
                .recovery_dir()
                .join(format!("purged-{id}-{ts}-{attempt}"));
            attempt += 1;
        }
        fs::create_dir_all(&dest)?;

        for path in candidates {
            if !path.exists() {
                continue;
            }
            let name = path.file_name().map(OsString::from).unwrap_or_default();
            let target = dest.join(name);
            if fs::rename(&path, &target).is_err() {
                // Locked or on another volume: keep the original and copy it.
                fs::copy(&path, &target)?;
            }
        }
        Ok(Some(dest))
    }

    /// Writes note content and its search sidecar. The RTF is written first
    /// (authoritative) and keeps one previous generation; the sidecar is
    /// derived data and is rewritten afterwards.
    pub fn save_note(&self, id: &NoteId, rtf: &[u8], txt: &str) -> io::Result<()> {
        self.atomic_write_keep_prev(&self.note_path(id), rtf)?;
        self.atomic_write(&self.note_txt_path(id), txt.as_bytes())
    }

    /// Reads `session.json`, falling back to defaults when the file is absent.
    pub fn load_session(&self) -> io::Result<Session> {
        let path = self.session_path();
        match fs::read_to_string(&path) {
            Ok(text) => {
                let mut session: Session = serde_json::from_str(&text).map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("{} is corrupt: {err}", path.display()),
                    )
                })?;
                session.clamp();
                Ok(session)
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Session::default()),
            Err(err) => Err(err),
        }
    }

    /// Saves `session.json` atomically.
    pub fn save_session(&self, session: &Session) -> io::Result<()> {
        let json = serde_json::to_string(session).map_err(|err| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("cannot serialise session: {err}"),
            )
        })?;
        self.atomic_write(&self.session_path(), json.as_bytes())
    }

    /// Moves every `*.tmp` file found under `notes/` into `recovery/` as
    /// `recovered-<name>-<ts>[.ext]`. Never deletes anything. Returns the new
    /// paths.
    pub fn recover_leftovers(&self) -> io::Result<Vec<PathBuf>> {
        let notes = self.notes_dir();
        if !notes.is_dir() {
            return Ok(Vec::new());
        }
        self.ensure_layout()?;

        let mut moved = Vec::new();
        let mut stack = vec![notes];
        while let Some(dir) = stack.pop() {
            let entries = match fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            };
            for entry in entries {
                let entry = entry?;
                let path = entry.path();
                let file_type = entry.file_type()?;
                if file_type.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !has_tmp_suffix(&path) {
                    continue;
                }
                let dest = self.recovery_path_for(&path);
                match fs::rename(&path, &dest) {
                    Ok(()) => moved.push(dest),
                    Err(_) => {
                        // Cross-device or locked: copy instead and keep the
                        // original in place. Recovery must never destroy data.
                        fs::copy(&path, &dest)?;
                        moved.push(dest);
                    }
                }
            }
        }
        moved.sort();
        Ok(moved)
    }

    fn recovery_path_for(&self, tmp: &Path) -> PathBuf {
        let name = tmp
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".to_owned());
        let stripped = name.strip_suffix(".tmp").unwrap_or(&name);
        let (base, ext) = match stripped.rsplit_once('.') {
            Some((base, ext)) if !base.is_empty() && !ext.is_empty() => (base, Some(ext)),
            _ => (stripped, None),
        };
        let ts = now_secs();
        let build = |suffix: Option<usize>| -> String {
            let n = suffix.map(|n| format!("-{n}")).unwrap_or_default();
            match ext {
                Some(ext) => format!("recovered-{base}-{ts}{n}.{ext}"),
                None => format!("recovered-{base}-{ts}{n}"),
            }
        };
        let mut candidate = build(None);
        let mut attempt = 1usize;
        while self.recovery_dir().join(&candidate).exists() {
            candidate = build(Some(attempt));
            attempt += 1;
        }
        self.recovery_dir().join(candidate)
    }
}

/// `<exe_dir>\data`, except for `<workspace>\target\<profile>` where it is
/// `<workspace>\data` (see [`DataDir::resolve`]).
fn data_dir_for_exe_dir(exe_dir: &Path) -> PathBuf {
    let in_target = exe_dir
        .parent()
        .filter(|parent| parent.file_name().is_some_and(|name| name == "target"));
    if let Some(workspace) = in_target.and_then(Path::parent) {
        if workspace.join("Cargo.toml").is_file() {
            return workspace.join("data");
        }
    }
    exe_dir.join("data")
}

/// Adds a suffix to a file name (`notes/a.rtf` + `.prev` → `notes/a.rtf.prev`).
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(suffix);
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
    }
}

fn has_tmp_suffix(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("tmp"))
}

/// One open tab in the session file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabState {
    pub note: NoteId,
    /// Caret position as a UTF-16/Rich Edit character offset.
    #[serde(default)]
    pub caret: u32,
    /// First visible line.
    #[serde(default)]
    pub scroll: u32,
}

/// Window placement in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Restored UI state (`session.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub tabs: Vec<TabState>,
    #[serde(default)]
    pub active: usize,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: i32,
    #[serde(default)]
    pub window: Option<WindowRect>,
    /// Tree items the user expanded.
    #[serde(default)]
    pub expanded: Vec<NoteId>,
    /// View > Word Wrap.
    #[serde(default = "default_true")]
    pub word_wrap: bool,
    /// Editor zoom in percent.
    #[serde(default = "default_zoom")]
    pub zoom: i32,
}

fn default_sidebar_width() -> i32 {
    DEFAULT_SIDEBAR_WIDTH
}

fn default_true() -> bool {
    true
}

fn default_zoom() -> i32 {
    100
}

impl Default for Session {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            window: None,
            expanded: Vec::new(),
            word_wrap: true,
            zoom: 100,
        }
    }
}

impl Session {
    /// Keeps the active tab index inside the tab list.
    pub fn clamp(&mut self) {
        if self.tabs.is_empty() {
            self.active = 0;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_prefers_the_environment_variable() {
        let wanted = std::env::temp_dir().join(format!("nodi-resolve-{}", std::process::id()));
        let previous = std::env::var_os("NODI_DATA_DIR");

        std::env::set_var("NODI_DATA_DIR", &wanted);
        let resolved = DataDir::resolve();

        match previous {
            Some(value) => std::env::set_var("NODI_DATA_DIR", value),
            None => std::env::remove_var("NODI_DATA_DIR"),
        }

        assert_eq!(resolved.root(), wanted.as_path());
    }

    #[test]
    fn cargo_target_exe_uses_workspace_data_dir() {
        let root = std::env::temp_dir().join(format!("nodi-ws-{}", NoteId::generate()));
        let release = root.join("target").join("release");
        fs::create_dir_all(&release).unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
        assert_eq!(data_dir_for_exe_dir(&release), root.join("data"));

        let portable = root.join("dist").join("Nodi");
        assert_eq!(data_dir_for_exe_dir(&portable), portable.join("data"));

        // A folder merely named `target` outside a Cargo workspace is portable.
        let lone = std::env::temp_dir()
            .join(format!("nodi-lone-{}", NoteId::generate()))
            .join("target")
            .join("debug");
        assert_eq!(data_dir_for_exe_dir(&lone), lone.join("data"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn temp_and_prev_paths_live_next_to_the_note() {
        let data = DataDir::at("data");
        let id = NoteId::generate();
        assert_eq!(
            data.note_path(&id),
            data.notes_dir().join(format!("{id}.rtf"))
        );
        assert_eq!(
            data.note_prev_path(&id),
            data.notes_dir().join(format!("{id}.rtf.prev"))
        );
        assert_eq!(data.log_path(), data.logs_dir().join("nodi.log"));
    }
}
