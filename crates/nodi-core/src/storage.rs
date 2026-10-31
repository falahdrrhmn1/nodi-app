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
use std::sync::{Arc, RwLock};

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
    /// Folder of the Markdown mirror chosen in Tools > AI Mirror Folder
    /// (`None` = `data/markdown`). Shared with the writer thread's clone.
    markdown_override: Arc<RwLock<Option<PathBuf>>>,
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
            markdown_override: Arc::new(RwLock::new(None)),
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

    /// `settings.json`
    pub fn settings_path(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    /// `markdown/` — the AI-agent-readable mirror (derived, never read back),
    /// or the folder chosen with [`DataDir::set_markdown_dir`].
    pub fn markdown_dir(&self) -> PathBuf {
        let chosen = self
            .markdown_override
            .read()
            .ok()
            .and_then(|guard| guard.clone());
        chosen.unwrap_or_else(|| self.root.join("markdown"))
    }

    /// Moves the Markdown mirror to `dir` (`None` = back to `data/markdown`).
    /// Every clone of this `DataDir` (the writer's too) follows. The old
    /// folder is left as it is.
    pub fn set_markdown_dir(&self, dir: Option<PathBuf>) {
        if let Ok(mut guard) = self.markdown_override.write() {
            *guard = dir;
        }
    }

    /// `icons/` - copies of note icons the user picked from a file.
    pub fn icons_dir(&self) -> PathBuf {
        self.root.join("icons")
    }

    /// `icons/<name>` for a user-supplied icon.
    pub fn icon_path(&self, name: &str) -> PathBuf {
        self.icons_dir().join(name)
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
            self.icons_dir(),
            self.markdown_dir(),
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
            rename_retrying(&tmp, path)
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

    /// Copies a user-chosen icon into `icons/` and returns its path. The name
    /// is reduced to safe characters; the extension is preserved so the tree
    /// loader knows whether to use `LoadImage` or the PNG decoder.
    pub fn save_icon(&self, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
        let mut safe: String = name
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | ' ') {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        safe = safe.trim().trim_end_matches('.').to_owned();
        if safe.is_empty() {
            safe = "icon.bin".to_owned();
        }
        let path = self.icon_path(&safe);
        self.atomic_write(&path, bytes)?;
        Ok(path)
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

/// `rename` that waits out a short lock on the target: on Windows replacing a
/// file fails while another program (a virus scanner, a sync client, an
/// indexer, a reader) has it open. Retries for up to about a second.
fn rename_retrying(from: &Path, to: &Path) -> io::Result<()> {
    const ATTEMPTS: u32 = 20;
    let mut attempt = 0;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(err) => {
                // ERROR_ACCESS_DENIED (5) and ERROR_SHARING_VIOLATION (32).
                let locked = err.kind() == io::ErrorKind::PermissionDenied
                    || matches!(err.raw_os_error(), Some(5 | 32));
                attempt += 1;
                if !locked || attempt >= ATTEMPTS {
                    return Err(err);
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
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
    /// File > Recent, newest first.
    #[serde(default)]
    pub recent: Vec<NoteId>,
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
            recent: Vec::new(),
        }
    }
}

impl Session {
    /// The `session.json` text (for the background writer).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Keeps the active tab index inside the tab list.
    pub fn clamp(&mut self) {
        if self.tabs.is_empty() {
            self.active = 0;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        }
    }
}

/// Colour theme chosen in View > Theme.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Light,
    Dark,
    Custom,
}

/// The three colours picked in View > Theme > Custom...
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomTheme {
    /// Editor background, `#rrggbb`.
    pub background: String,
    /// Editor text, `#rrggbb`.
    pub text: String,
    /// Selection/highlight accent, `#rrggbb`.
    pub accent: String,
}

impl Default for CustomTheme {
    fn default() -> Self {
        Self {
            background: "#ffffff".to_owned(),
            text: "#1f1f1f".to_owned(),
            accent: "#cce8ff".to_owned(),
        }
    }
}

/// App-wide settings (`settings.json`). Missing files and unknown fields are
/// fine; every value has a safe default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub custom: CustomTheme,
    /// Tools > "Keep Markdown mirror for AI agents" (default ON).
    #[serde(default = "default_true")]
    pub markdown_mirror: bool,
    /// View > Spell Check (default off).
    #[serde(default)]
    pub spell_check: bool,
    /// View > "Ctrl+Up/Down Scrolls the View" (Notepad++ style, default on).
    #[serde(default = "default_true")]
    pub ctrl_arrow_scroll: bool,
    /// Tools > AI Mirror Folder: where the Markdown mirror goes (empty =
    /// `data/markdown`).
    #[serde(default)]
    pub markdown_dir: String,
    /// View > Sort Notes.
    #[serde(default)]
    pub tree_sort: crate::tree::SortOrder,
    #[serde(default)]
    pub tree_sort_desc: bool,
    /// Folders above notes when sorted (not in the manual order).
    #[serde(default = "default_true")]
    pub folders_first: bool,
    /// Preferences > Sounds: beeps and message-box sounds (default off).
    #[serde(default)]
    pub sounds: bool,
    /// Preferences > two quick spaces type ". ".
    #[serde(default = "default_true")]
    pub double_space_period: bool,
    /// View > same-page word autocomplete (default on).
    #[serde(default = "default_true")]
    pub autocomplete: bool,
    /// User text expansions. Scope is `page`, `all`, or `folder`.
    #[serde(default)]
    pub text_shortcuts: Vec<TextShortcut>,
    /// View > Line Numbers.
    #[serde(default = "default_true")]
    pub line_numbers: bool,
    /// View > Background: plain, lines, grid or dots behind the text.
    #[serde(default)]
    pub background: Background,
    /// View > Cursor: the shape of the text caret.
    #[serde(default)]
    pub caret_style: CaretStyle,
    /// Preferences > default editor zoom in percent (also Reset Zoom).
    #[serde(default = "default_zoom")]
    pub default_zoom: i32,
    /// Preferences > default font for new text.
    #[serde(default = "default_body_font")]
    pub body_font: String,
    /// Preferences > default size (points) for new text.
    #[serde(default = "default_body_size")]
    pub body_size: u32,
    /// Name of the chosen theme preset or saved theme (`theme` keeps the
    /// light/dark/custom mode for older files).
    #[serde(default)]
    pub theme_name: String,
    /// Themes the user saved (Theme Editor > Save As My Theme).
    #[serde(default)]
    pub my_themes: Vec<ThemeSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextShortcut {
    pub trigger: String,
    pub replacement: String,
    /// `page`, `all`, or `folder`.
    #[serde(default = "default_shortcut_scope")]
    pub scope: String,
    #[serde(default)]
    pub note: Option<crate::NoteId>,
    #[serde(default)]
    pub folder: Option<crate::NoteId>,
}

fn default_shortcut_scope() -> String {
    "page".to_owned()
}

/// Editor background pattern (View > Background).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Background {
    #[default]
    Plain,
    Lines,
    Grid,
    Dots,
}

/// Shape of the text caret (View > Cursor).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CaretStyle {
    /// The thin line Rich Edit draws itself (the default).
    #[default]
    Line,
    /// A line three pixels wide.
    Thick,
    /// A block over the character after the caret.
    Block,
    /// A line under the character after the caret.
    Underline,
}

fn default_body_font() -> String {
    "Segoe UI".to_owned()
}

fn default_body_size() -> u32 {
    11
}

/// A complete colour + font theme: every colour of the window, as `#rrggbb`.
/// Presets ship with Nodi; users save their own and copy them as JSON text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeSpec {
    pub name: String,
    /// Note area.
    pub editor_bg: String,
    pub editor_text: String,
    /// Frame, toolbars, menus and dialogs around the note.
    pub window_bg: String,
    pub window_text: String,
    /// Notes Explorer.
    pub tree_bg: String,
    pub tree_text: String,
    /// Tabs.
    pub tab_bg: String,
    pub tab_active: String,
    /// Selection / checked buttons / highlights.
    pub accent: String,
    pub border: String,
    /// Line numbers and the background pattern.
    pub gutter_text: String,
    pub pattern: String,
    /// Font of the tree, tabs and status bar (empty = Segoe UI).
    pub ui_font: String,
    pub ui_size: u32,
}

impl Default for ThemeSpec {
    fn default() -> Self {
        Self {
            name: "Light".to_owned(),
            editor_bg: "#ffffff".to_owned(),
            editor_text: "#1f1f1f".to_owned(),
            window_bg: "#f0f0f0".to_owned(),
            window_text: "#1f1f1f".to_owned(),
            tree_bg: "#ffffff".to_owned(),
            tree_text: "#1f1f1f".to_owned(),
            tab_bg: "#f0f0f0".to_owned(),
            tab_active: "#ffffff".to_owned(),
            accent: "#cce8ff".to_owned(),
            border: "#c8c8c8".to_owned(),
            gutter_text: "#9a9a9a".to_owned(),
            pattern: "#e8edf3".to_owned(),
            ui_font: "Segoe UI".to_owned(),
            ui_size: 9,
        }
    }
}

impl ThemeSpec {
    /// Every colour field with its label (Theme Editor rows).
    pub fn colours_mut(&mut self) -> [(&'static str, &mut String); 12] {
        [
            ("Note background", &mut self.editor_bg),
            ("Note text", &mut self.editor_text),
            ("Window background", &mut self.window_bg),
            ("Window text", &mut self.window_text),
            ("Notes Explorer background", &mut self.tree_bg),
            ("Notes Explorer text", &mut self.tree_text),
            ("Tab bar", &mut self.tab_bg),
            ("Active tab", &mut self.tab_active),
            ("Accent (selection, checked)", &mut self.accent),
            ("Borders", &mut self.border),
            ("Line numbers", &mut self.gutter_text),
            ("Background pattern", &mut self.pattern),
        ]
    }

    /// Replaces unparsable colours with this theme's defaults, so a pasted
    /// or hand-edited theme can never make text unreadable.
    pub fn clamp(&mut self) {
        let fallback = ThemeSpec::default();
        let pairs = [
            (&mut self.editor_bg, &fallback.editor_bg),
            (&mut self.editor_text, &fallback.editor_text),
            (&mut self.window_bg, &fallback.window_bg),
            (&mut self.window_text, &fallback.window_text),
            (&mut self.tree_bg, &fallback.tree_bg),
            (&mut self.tree_text, &fallback.tree_text),
            (&mut self.tab_bg, &fallback.tab_bg),
            (&mut self.tab_active, &fallback.tab_active),
            (&mut self.accent, &fallback.accent),
            (&mut self.border, &fallback.border),
            (&mut self.gutter_text, &fallback.gutter_text),
            (&mut self.pattern, &fallback.pattern),
        ];
        for (value, default) in pairs {
            match parse_hex_color(value) {
                Some(rgb) => *value = format_hex_color(rgb),
                None => *value = default.clone(),
            }
        }
        if self.ui_font.trim().is_empty() {
            self.ui_font = fallback.ui_font;
        }
        self.ui_size = self.ui_size.clamp(7, 16);
        let name = self.name.trim().to_owned();
        self.name = if name.is_empty() {
            "My theme".to_owned()
        } else {
            name.chars().take(60).collect()
        };
    }

    /// The theme as pretty JSON (Copy Theme).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// A theme pasted as JSON text (Paste Theme); colours are checked.
    pub fn from_json(text: &str) -> Option<Self> {
        let mut spec: ThemeSpec = serde_json::from_str(text.trim()).ok()?;
        spec.clamp();
        Some(spec)
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::Light,
            custom: CustomTheme::default(),
            markdown_mirror: true,
            spell_check: false,
            ctrl_arrow_scroll: true,
            markdown_dir: String::new(),
            tree_sort: crate::tree::SortOrder::Manual,
            tree_sort_desc: false,
            folders_first: true,
            sounds: false,
            double_space_period: true,
            autocomplete: true,
            text_shortcuts: Vec::new(),
            line_numbers: true,
            background: Background::Plain,
            caret_style: CaretStyle::Line,
            default_zoom: 100,
            body_font: default_body_font(),
            body_size: default_body_size(),
            theme_name: String::new(),
            my_themes: Vec::new(),
        }
    }
}

impl Settings {
    /// The `settings.json` text.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Replaces an unparsable colour with its default so a hand-edited file
    /// can never make the editor unreadable.
    pub fn clamp(&mut self) {
        let defaults = CustomTheme::default();
        for (value, fallback) in [
            (&mut self.custom.background, &defaults.background),
            (&mut self.custom.text, &defaults.text),
            (&mut self.custom.accent, &defaults.accent),
        ] {
            if parse_hex_color(value).is_none() {
                *value = (*fallback).clone();
            }
        }
        if self.body_font.trim().is_empty() {
            self.body_font = default_body_font();
        }
        self.body_size = self.body_size.clamp(6, 72);
        self.default_zoom = self.default_zoom.clamp(25, 500);
        for theme in &mut self.my_themes {
            theme.clamp();
        }
    }
}

/// Parses `#rrggbb` (or `rrggbb`) into RGB bytes.
pub fn parse_hex_color(text: &str) -> Option<(u8, u8, u8)> {
    let hex = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((red, green, blue))
}

/// Formats RGB bytes as `#rrggbb`.
pub fn format_hex_color((red, green, blue): (u8, u8, u8)) -> String {
    format!("#{red:02x}{green:02x}{blue:02x}")
}

impl DataDir {
    /// Reads `settings.json`; missing or corrupt files fall back to defaults.
    pub fn load_settings(&self) -> Settings {
        let mut settings = fs::read_to_string(self.settings_path())
            .ok()
            .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
            .unwrap_or_default();
        settings.clamp();
        settings
    }

    /// Saves `settings.json` atomically.
    pub fn save_settings(&self, settings: &Settings) -> io::Result<()> {
        self.atomic_write(&self.settings_path(), settings.to_json().as_bytes())
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
