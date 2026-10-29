//! Application state and the Phase 2+3 logic: the note tree, open tabs, the
//! autosave state machine and the session file.
//!
//! The `App` lives in `GWLP_USERDATA` on the frame window (see [`crate::wnd`])
//! and is owned by the window procedure.

use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::ptr;
use std::time::Duration;

use nodi_core::autosave::{Job, SaveRequest, SaveResult, Target, Writer};
use nodi_core::backup;
use nodi_core::storage::{Session, Settings, TabState, WindowRect, DEFAULT_SIDEBAR_WIDTH};
use nodi_core::tree::Position;
use nodi_core::{DataDir, NodeKind, NoteId, Tree};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, DEFAULT_CHARSET, FW_BOLD, FW_NORMAL, HBRUSH, HFONT, LOGFONTW,
};
use windows_sys::Win32::UI::Controls::HIMAGELIST;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, GetWindowPlacement, KillTimer, LoadCursorW, PeekMessageW, PostMessageW,
    SendMessageW, SetTimer, SetWindowPos, SetWindowTextW, SystemParametersInfoW, HACCEL, HCURSOR,
    HMENU, IDC_SIZEWE, NONCLIENTMETRICSW, PM_REMOVE, SPI_GETNONCLIENTMETRICS, SWP_NOACTIVATE,
    SWP_NOZORDER, SW_SHOWNORMAL, WINDOWPLACEMENT, WM_SETFONT, WNDCLASS_STYLES,
};

use crate::win;
use crate::wnd::form::{Field, Form};
use crate::wnd::{clipboard, dialogs, editor, image, link, menu, status, table, tabs, theme, tree};

/// Custom message posted by the writer thread with a boxed [`SaveResult`].
pub const WM_APP_SAVE_RESULT: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1;

/// Notes Explorer changes kept for Ctrl+Z.
pub const TREE_UNDO_LIMIT: usize = 20;
/// Debounce delay for the per-tab autosave timer.
const DEBOUNCE_MS: u32 = 500;
/// Retry delay while a save keeps failing.
const RETRY_MS: u32 = 5000;
/// Timer id used by the "retry failed saves" timer.
pub const RETRY_TIMER: usize = usize::MAX - 1;
/// Timer id of the debounced session save.
pub const SESSION_TIMER: usize = usize::MAX - 2;
const SESSION_DELAY_MS: u32 = 1500;
/// Backups older than this are refreshed at startup.
const BACKUP_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// One open note in one Rich Edit control.
pub struct Tab {
    pub editor: HWND,
    pub note: NoteId,
    /// Base title, without the dirty marker.
    pub title: String,
    /// Edited since the last submit.
    pub dirty: bool,
    /// Sequence number of the newest submitted save.
    pub seq: u64,
    /// Highest acknowledged (Ok) sequence number.
    pub acked: u64,
    pub last_error: Option<String>,
    /// One MessageBox per failure streak.
    pub failure_streak: bool,
    /// Owned debounce timer id (`0` = none).
    pub timer: usize,
}

impl Tab {
    fn is_saved(&self) -> bool {
        !self.dirty && self.last_error.is_none() && self.seq <= self.acked
    }
}

pub struct App {
    pub hwnd: HWND,
    pub hinstance: HINSTANCE,
    pub menu: HMENU,
    pub accel: HACCEL,

    pub data: Option<DataDir>,
    pub tree: Tree,
    pub writer: Option<Writer>,

    pub tabs: Vec<Tab>,
    pub active: Option<usize>,

    /// Tree item the user selected / the context-menu target.
    pub selected: Option<NoteId>,
    pub context_note: Option<NoteId>,
    pub expanded: HashSet<NoteId>,
    pub tree_items: HashMap<isize, NoteId>,
    pub item_for: HashMap<NoteId, isize>,
    pub images: HIMAGELIST,
    pub icon_keys: Vec<&'static str>,

    /// Nesting guards for the notifications the UI generates itself.
    pub suppress_change: u32,
    pub suppress_select: u32,
    pub next_timer: usize,

    pub drag_source: Option<NoteId>,
    pub dragging: bool,
    /// Item and zone under the drag cursor (drives the drop indicator).
    pub drag_hover: Option<(NoteId, Position)>,
    /// When the cursor first hovered the current drop target.
    pub drag_hover_since: Option<std::time::Instant>,
    /// Last drag position in tree client coordinates (for the drag timer).
    pub drag_point: Option<(i32, i32)>,

    pub dpi: u32,
    pub ui_font: HFONT,
    pub bold_font: HFONT,
    pub hcursor_we: HCURSOR,

    pub toolbar: HWND,
    pub formatbar: HWND,
    pub style_combo: HWND,
    pub font_combo: HWND,
    pub size_combo: HWND,
    /// >0 while the combos are updated from the editor (no commands back).
    pub suppress_combo: u32,
    pub sidebar_title: HWND,
    pub tree_view: HWND,
    pub tabctrl: HWND,
    pub status: HWND,
    /// Line-number strip left of the editor (View > Line Numbers).
    pub gutter: HWND,

    pub toolbar_visible: bool,
    pub status_visible: bool,
    pub sidebar_width: i32,
    pub splitter_drag: bool,
    pub restored_window: bool,

    pub toolbar_height: i32,
    pub formatbar_height: i32,
    /// The "Trash (N)" pseudo-item at the bottom of the tree, and its icon.
    pub trash_item: isize,
    pub trash_image: i32,
    /// New notes whose first (empty) file write is still queued.
    pub pending_new: HashSet<NoteId>,
    /// View > Word Wrap, applied to every editor.
    pub word_wrap: bool,
    /// Zoom percentage applied to every editor.
    pub zoom: i32,
    /// `settings.json` (theme, mirror toggle, spell check).
    pub settings: Settings,
    /// Resolved colours for the current theme.
    pub palette: theme::Palette,
    /// Frame/splitter background; owned by the app.
    pub background_brush: HBRUSH,
    /// The five status-bar texts, so the owner-draw path can redraw them.
    pub status_text: Vec<String>,
    /// File > Recent, newest first (also persisted in `session.json`).
    pub recent: Vec<NoteId>,
    /// Tab whose context menu is open (right-click on the tab strip).
    pub tab_context: Option<usize>,
    /// Notes Explorer changes Ctrl+Z (tree focused) can take back, newest
    /// last; at most [`TREE_UNDO_LIMIT`], this session only.
    pub tree_undo: Vec<nodi_core::TreeUndo>,
    /// Notes Explorer multi-selection (Ctrl+click / Shift+click), in tree
    /// order; empty or one entry = just the normal selection.
    pub multi: Vec<NoteId>,
    /// Where a Shift+click range starts.
    pub multi_anchor: Option<NoteId>,
    /// Set once `WM_CREATE` finished: from then on the window procedure owns
    /// the boxed `App` and frees it in `WM_NCDESTROY`.
    pub created: bool,
}

impl App {
    pub(crate) fn new() -> Self {
        Self {
            hwnd: ptr::null_mut(),
            hinstance: ptr::null_mut(),
            menu: ptr::null_mut(),
            accel: ptr::null_mut(),
            data: None,
            tree: Tree::new(),
            writer: None,
            tabs: Vec::new(),
            active: None,
            selected: None,
            context_note: None,
            expanded: HashSet::new(),
            tree_items: HashMap::new(),
            item_for: HashMap::new(),
            images: 0,
            icon_keys: Vec::new(),
            suppress_change: 0,
            suppress_select: 0,
            next_timer: 0,
            drag_source: None,
            dragging: false,
            drag_hover: None,
            drag_hover_since: None,
            drag_point: None,
            dpi: 96,
            ui_font: ptr::null_mut(),
            bold_font: ptr::null_mut(),
            // SAFETY: `IDC_SIZEWE` is a predefined system cursor; the call
            // takes no pointers we own and returns a shared handle.
            hcursor_we: unsafe { LoadCursorW(ptr::null_mut(), IDC_SIZEWE) },
            toolbar: ptr::null_mut(),
            formatbar: ptr::null_mut(),
            style_combo: ptr::null_mut(),
            font_combo: ptr::null_mut(),
            size_combo: ptr::null_mut(),
            suppress_combo: 0,
            sidebar_title: ptr::null_mut(),
            tree_view: ptr::null_mut(),
            tabctrl: ptr::null_mut(),
            status: ptr::null_mut(),
            gutter: ptr::null_mut(),
            toolbar_visible: true,
            status_visible: true,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            splitter_drag: false,
            restored_window: false,
            toolbar_height: 0,
            formatbar_height: 0,
            trash_item: 0,
            trash_image: 0,
            pending_new: HashSet::new(),
            word_wrap: true,
            zoom: 100,
            settings: Settings::default(),
            palette: theme::Palette::default(),
            background_brush: ptr::null_mut(),
            status_text: vec![String::new(); 5],
            recent: Vec::new(),
            tab_context: None,
            tree_undo: Vec::new(),
            multi: Vec::new(),
            multi_anchor: None,
            created: false,
        }
    }

    pub fn active_editor(&self) -> Option<HWND> {
        self.active
            .and_then(|index| self.tabs.get(index))
            .map(|tab| tab.editor)
    }

    pub fn tab_index(&self, editor: HWND) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.editor == editor)
    }

    pub fn tab_index_for_note(&self, id: &NoteId) -> Option<usize> {
        self.tabs.iter().position(|tab| &tab.note == id)
    }

    pub fn scale(&self, value: i32) -> i32 {
        win::mul_div(value, self.dpi as i32, 96)
    }

    fn timer_id(&mut self) -> usize {
        self.next_timer += 1;
        self.next_timer
    }

    // ---------------------------------------------------------------- startup

    /// Resolves the data directory, loads/repairs the tree, starts the writer
    /// and restores the session. Returns false when Nodi must not continue
    /// (a corrupt tree the user does not want to restore, or no data dir).
    pub fn startup(&mut self) -> bool {
        let data = DataDir::resolve();
        if let Err(err) = data.ensure_layout() {
            dialogs::error(
                self.hwnd,
                &format!(
                    "Nodi could not create its data directory:\n{}\n\n{err}",
                    data.root().display()
                ),
            );
            return false;
        }
        if std::env::var_os("NODI_FAIL_WRITES").is_some_and(|value| value == "1") {
            data.set_fail_writes(true);
        }

        match data.recover_leftovers() {
            // An interrupted save (crash, power cut, killed process) leaves
            // its unfinished temp file; the note itself is the last complete
            // version. Say so in the status bar instead of a dialog that
            // would stop the start before the window is even shown.
            Ok(paths) if !paths.is_empty() => {
                let message = format!(
                    "An interrupted save was found: {} unfinished file(s) kept in \
                     data\\recovery — your notes are the last complete versions",
                    paths.len()
                );
                status::set_message(self, &message);
            }
            Ok(_) => {}
            Err(err) => dialogs::error(
                self.hwnd,
                &format!("Nodi could not scan for leftover files:\n{err}"),
            ),
        }

        let tree = match data.load_tree() {
            Ok(tree) => tree,
            Err(err) => {
                let snapshots = backup::list(&data).unwrap_or_default();
                let message = format!(
                    "tree.json could not be read:\n{err}\n\n\
                     Restore the newest backup now?\n\n\
                     (No closes Nodi without touching anything.)"
                );
                match snapshots.last() {
                    Some(snapshot) => {
                        if !dialogs::ask_yes_no(self.hwnd, &message) {
                            return false;
                        }
                        if let Err(restore) = backup::restore(&data, snapshot) {
                            dialogs::error(
                                self.hwnd,
                                &format!("The backup could not be restored:\n{restore}"),
                            );
                            return false;
                        }
                        match data.load_tree() {
                            Ok(tree) => tree,
                            Err(again) => {
                                dialogs::error(
                                    self.hwnd,
                                    &format!(
                                        "tree.json is still unreadable after the restore:\n{again}"
                                    ),
                                );
                                return false;
                            }
                        }
                    }
                    None => {
                        dialogs::error(
                            self.hwnd,
                            &format!(
                                "tree.json is corrupt and there is no backup to restore:\n{err}\n\n\
                                 Nodi closes without touching the file."
                            ),
                        );
                        return false;
                    }
                }
            }
        };
        self.tree = tree;

        // Settings decide the palette before any note is streamed into an
        // editor, so the very first paint is already themed.
        self.settings = data.load_settings();
        self.apply_body_font();
        let chosen = self.settings.markdown_dir.trim();
        if !chosen.is_empty() {
            data.set_markdown_dir(Some(std::path::PathBuf::from(chosen)));
        }
        self.palette = theme::Palette::from_settings(&self.settings);
        self.palette.install_display_colours();

        // Writer thread: results are posted back as boxed `SaveResult`s. The
        // window handle travels as `isize` so the closure stays `Send`.
        let hwnd_value = self.hwnd as isize;
        self.writer = Some(Writer::spawn(
            data.clone(),
            Box::new(move |result| {
                let raw = Box::into_raw(Box::new(result)) as isize;
                // SAFETY: posting to the frame window; on failure (window
                // already gone) reclaim and drop the box.
                let posted =
                    unsafe { PostMessageW(hwnd_value as HWND, WM_APP_SAVE_RESULT, 0, raw) };
                if posted == 0 {
                    unsafe {
                        drop(Box::from_raw(raw as *mut SaveResult));
                    }
                }
            }),
        ));
        self.data = Some(data.clone());

        // The Markdown mirror is opt-out; a sync at startup (only notes changed
        // since their mirror file was written) also hands the writer its tree
        // for the per-save updates.
        if let Some(writer) = &self.writer {
            writer.submit(Job::MirrorEnabled(self.settings.markdown_mirror));
            if self.settings.markdown_mirror {
                if let Ok(json) = self.tree.to_json() {
                    writer.submit(Job::MirrorSync(json));
                }
            }
        }

        let session = data.load_session().unwrap_or_default();
        if session.sidebar_width > 0 {
            self.sidebar_width = session.sidebar_width;
        }
        self.expanded = session.expanded.iter().cloned().collect();
        self.word_wrap = session.word_wrap;
        self.zoom = self.settings.default_zoom.clamp(25, 500);
        self.recent = session
            .recent
            .iter()
            .filter(|id| {
                self.tree
                    .get(id)
                    .is_some_and(|node| node.kind == NodeKind::Note && !node.trashed)
            })
            .take(10)
            .cloned()
            .collect();
        menu::set_checked(self, menu::IDM_VIEW_WORD_WRAP, self.word_wrap);
        menu::sync_settings(self);
        menu::refresh_recent(self);
        if let Some(rect) = session.window {
            self.apply_window_rect(rect);
        }

        if self.tree.is_empty() {
            let id = self.create_note_node(NodeKind::Note, "Untitled", None);
            self.tree.set_auto_title(&id, true);
            self.tree_save();
            self.selected = Some(id.clone());
            self.expand(Some(id.clone()));
            self.open_note(id);
            self.save_session_now();
        } else {
            self.restore_session_tabs(&session);
        }

        self.load_custom_icons();
        self.rebuild_tree();
        self.maybe_startup_backup();
        true
    }

    /// Appends every icon the user copied into `data/icons/` to the image list
    /// (the tree stores keys like `file:star.png`).
    fn load_custom_icons(&mut self) {
        let Some(data) = self.data.clone() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(data.icons_dir()) else {
            return;
        };
        let mut files: Vec<std::path::PathBuf> = entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.path())
            .collect();
        files.sort();
        let icons: Vec<(String, std::path::PathBuf)> = files
            .into_iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|name| (name.to_string_lossy().into_owned(), path.clone()))
            })
            .collect();
        for (name, path) in icons {
            let key: &'static str = Box::leak(format!("file:{name}").into_boxed_str());
            if self.icon_keys.contains(&key) {
                continue;
            }
            tree::add_icon(self, key, &path);
        }
    }

    fn restore_session_tabs(&mut self, session: &Session) {
        let mut opened = 0usize;
        for tab in &session.tabs {
            let usable = self
                .tree
                .get(&tab.note)
                .is_some_and(|node| node.kind == NodeKind::Note && !node.trashed);
            if !usable {
                continue;
            }
            let caret = tab.caret as i32;
            self.open_note_with(tab.note.clone(), Some((caret, caret)), Some(tab.scroll));
            opened += 1;
        }

        if opened == 0 {
            if let Some(id) = self.first_visible_note() {
                self.selected = Some(id.clone());
                self.open_note_with(id, None, None);
            }
            return;
        }

        let target = session.active.min(self.tabs.len() - 1);
        self.activate_tab_impl(target, false);
    }

    fn first_visible_note(&self) -> Option<NoteId> {
        self.tree
            .iter_ordered()
            .into_iter()
            .find(|node| !node.trashed && node.kind == NodeKind::Note)
            .map(|node| node.id.clone())
    }

    fn apply_window_rect(&mut self, rect: WindowRect) {
        if rect.width < 200 || rect.height < 120 {
            return;
        }
        // SAFETY: `hwnd` is our frame window.
        unsafe {
            SetWindowPos(
                self.hwnd,
                ptr::null_mut(),
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        self.restored_window = true;
    }

    fn maybe_startup_backup(&self) {
        let Some(data) = self.data.clone() else {
            return;
        };
        std::thread::spawn(move || {
            let stale = match backup::list(&data) {
                Ok(list) => match list.last() {
                    Some(newest) => snapshot_is_stale(newest),
                    None => true,
                },
                Err(_) => true,
            };
            if stale {
                let _ = backup::snapshot(&data);
                let _ = backup::prune(&data, backup::DEFAULT_KEEP);
            }
        });
    }

    /// `Tools > Backup Now`: snapshots + prunes on a short-lived thread so the
    /// UI never blocks on a large notes directory.
    pub fn backup_now(&mut self) {
        let Some(data) = self.data.clone() else {
            return;
        };
        status::set_message(self, "Backup started");
        std::thread::spawn(move || {
            let _ = backup::snapshot(&data);
            let _ = backup::prune(&data, backup::DEFAULT_KEEP);
        });
    }

    /// `Tools > Open Data Folder`.
    pub fn open_data_folder(&mut self) {
        let Some(data) = &self.data else {
            return;
        };
        let path = win::wide(&data.root().to_string_lossy());
        let verb = win::wide("explore");
        // SAFETY: all pointers are NUL terminated and outlive the call.
        unsafe {
            ShellExecuteW(
                self.hwnd,
                verb.as_ptr(),
                path.as_ptr(),
                ptr::null(),
                ptr::null(),
                SW_SHOWNORMAL,
            );
        }
    }

    /// Tools > AI Mirror Folder...: pick where the Markdown mirror goes.
    pub fn choose_mirror_folder(&mut self) {
        let Some(folder) = dialogs::open_folder(self.hwnd) else {
            return;
        };
        if let Err(message) = self.set_mirror_folder(Some(folder)) {
            dialogs::error(self.hwnd, &message);
        }
    }

    /// Moves the mirror (`None` = `data\markdown`) and rebuilds it there. The
    /// old folder is left as it is. Only local drives are accepted.
    pub fn set_mirror_folder(&mut self, folder: Option<String>) -> Result<(), String> {
        let Some(data) = self.data.clone() else {
            return Err("No data directory".to_owned());
        };
        let mut folder = folder.map(|folder| folder.trim().trim_end_matches('\\').to_owned());
        if let Some(chosen) = folder.as_mut() {
            let bytes = chosen.as_bytes();
            let drive =
                bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && &bytes[1..3] == b":\\";
            if !drive || crate::wnd::link::drive_is_remote(chosen) {
                return Err(format!(
                    "The AI mirror folder must be a folder on a local drive \
                     (like E:\\catatan-ai):\n{chosen}"
                ));
            }
            // The mirror moves Markdown files it does not own into `.trash`,
            // so it never takes over a folder that already holds other files:
            // it uses a "Nodi AI" folder inside it instead.
            let path = std::path::Path::new(chosen.as_str());
            let used = std::fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_some());
            if used && !path.join("index.json").is_file() {
                *chosen = path.join("Nodi AI").to_string_lossy().into_owned();
            }
            std::fs::create_dir_all(chosen.as_str()).map_err(|err| format!("{chosen}: {err}"))?;
        }
        data.set_markdown_dir(folder.as_ref().map(std::path::PathBuf::from));
        self.settings.markdown_dir = folder.unwrap_or_default();
        self.save_settings();
        if self.settings.markdown_mirror {
            self.rebuild_mirror();
        }
        let shown = data.markdown_dir().to_string_lossy().into_owned();
        status::set_message(self, &format!("AI mirror folder: {shown}"));
        Ok(())
    }

    /// The folder AI agents should read.
    pub fn ai_folder(&self) -> Option<std::path::PathBuf> {
        self.data.as_ref().map(DataDir::markdown_dir)
    }

    /// Tools > Copy AI Folder Path.
    pub fn copy_ai_folder_path(&mut self) {
        let Some(folder) = self.ai_folder() else {
            return;
        };
        let text = folder.to_string_lossy().into_owned();
        if crate::wnd::clipboard::set_text(self.hwnd, &text) {
            status::set_message(self, &format!("Copied: {text}"));
        }
    }

    /// Tools > Open AI Folder.
    pub fn open_ai_folder(&mut self) {
        let Some(folder) = self.ai_folder() else {
            return;
        };
        let _ = std::fs::create_dir_all(&folder);
        let path = win::wide(&folder.to_string_lossy());
        let verb = win::wide("explore");
        // SAFETY: all pointers are NUL terminated and outlive the call.
        unsafe {
            ShellExecuteW(
                self.hwnd,
                verb.as_ptr(),
                path.as_ptr(),
                ptr::null(),
                ptr::null(),
                SW_SHOWNORMAL,
            );
        }
    }

    /// Help > Using Nodi with AI Agents.
    pub fn explain_ai_mirror(&mut self) {
        let folder = self
            .ai_folder()
            .map(|folder| folder.to_string_lossy().into_owned())
            .unwrap_or_default();
        let state = if self.settings.markdown_mirror {
            "on"
        } else {
            "OFF (Tools > Keep Markdown Mirror for AI Agents)"
        };
        dialogs::info(
            self.hwnd,
            &format!(
                "Nodi keeps every note as a Markdown file for AI agents (mirror {state}):\n\n\
                 {folder}\n\n\
                 Point the agent at this folder and let it start with INDEX.md \
                 (index.json is the machine-readable list). The files are \
                 regenerated after each save; changes made there are not read \
                 back into Nodi.\n\n\
                 Tools > AI Mirror Folder... moves it (e.g. to E:\\catatan-ai); \
                 Tools > Copy AI Folder Path copies the path."
            ),
        );
    }

    // ------------------------------------------------------------------- tabs

    /// Opens (or re-uses) the tab for a note.
    pub fn open_note(&mut self, id: NoteId) {
        self.open_note_with(id.clone(), None, None);
        if self.tab_index_for_note(&id).is_some() {
            self.note_opened(&id);
        }
        self.save_session_now();
    }

    /// Records a note in File > Recent (newest first, max 10).
    fn note_opened(&mut self, id: &NoteId) {
        self.recent.retain(|recent| recent != id);
        self.recent.insert(0, id.clone());
        self.recent.truncate(10);
        menu::refresh_recent(self);
    }

    /// File > Recent: opens the entry at `index`.
    pub fn open_recent(&mut self, index: usize) {
        let Some(id) = self.recent.get(index).cloned() else {
            return;
        };
        self.open_note(id);
    }

    /// The tab a context menu acts on: the right-clicked one, else the active.
    pub fn context_tab(&self) -> Option<usize> {
        self.tab_context
            .filter(|index| *index < self.tabs.len())
            .or(self.active)
    }

    pub fn close_context_tab(&mut self) {
        if let Some(index) = self.context_tab() {
            self.close_tab(index);
        }
    }

    /// Tab menu "Close Others": keeps one tab, saves and closes the rest.
    pub fn close_other_tabs(&mut self) {
        let Some(keep) = self.context_tab() else {
            return;
        };
        let keep_note = self.tabs[keep].note.clone();
        for index in (0..self.tabs.len()).rev() {
            if self.tabs[index].note != keep_note {
                self.close_tab(index);
            }
        }
        if let Some(index) = self.tab_index_for_note(&keep_note) {
            self.activate_tab(index);
        }
        self.tab_context = None;
    }

    /// Tab menu "Close All": saves and closes every tab.
    pub fn close_all_tabs(&mut self) {
        for index in (0..self.tabs.len()).rev() {
            self.close_tab(index);
        }
        self.tab_context = None;
    }

    /// Tab menu "Copy Internal Link".
    pub fn copy_tab_link(&mut self) {
        let Some(index) = self.context_tab() else {
            return;
        };
        let note = self.tabs[index].note.clone();
        copy_link_for(self, &note);
        self.tab_context = None;
    }

    /// Tab menu "Reveal in Tree": selects the note in the sidebar.
    pub fn reveal_tab_note(&mut self) {
        let Some(index) = self.context_tab() else {
            return;
        };
        let note = self.tabs[index].note.clone();
        self.selected = Some(note.clone());
        self.expand(self.tree.get(&note).and_then(|node| node.parent.clone()));
        crate::wnd::tree::select_item(self, &note);
        crate::wnd::tree::focus_tree(self);
        self.tab_context = None;
    }

    /// Drops every tab without submitting it (used before a backup restore,
    /// where the pending editor content must not overwrite the restored data).
    fn discard_all_tabs(&mut self) {
        for index in (0..self.tabs.len()).rev() {
            let timer = self.tabs[index].timer;
            if timer != 0 {
                // SAFETY: killing a timer we started on our own window.
                unsafe {
                    KillTimer(self.hwnd, timer);
                }
            }
            let editor_window = self.tabs[index].editor;
            if !editor_window.is_null() {
                crate::wnd::format::forget_editor(editor_window);
                // SAFETY: the editor is a child window of this process.
                unsafe {
                    DestroyWindow(editor_window);
                }
            }
            self.tabs.remove(index);
            tabs::delete(self, index);
        }
        self.active = None;
        self.tab_context = None;
        status::set_save_state(self, "");
        status::set_message(self, "Restored from backup");
        status::clear_caret(self);
        status::update_counts(self);
        crate::wnd::layout(self);
    }

    fn open_note_with(&mut self, id: NoteId, selection: Option<(i32, i32)>, scroll: Option<u32>) {
        if let Some(index) = self.tab_index_for_note(&id) {
            self.activate_tab_impl(index, false);
            return;
        }
        let usable = self
            .tree
            .get(&id)
            .is_some_and(|node| node.kind == NodeKind::Note && !node.trashed);
        if !usable {
            return;
        }
        self.flush_active();

        let Some(editor) = editor::create(self) else {
            status::set_message(self, "Could not create the editor");
            return;
        };

        let bytes = self.data.as_ref().and_then(|data| data.load_note(&id));
        let loaded = match &bytes {
            Some(bytes) => editor::load_rtf(editor, &theme::theme_bytes(&self.palette, bytes)),
            None => {
                editor::clear(editor);
                true
            }
        };
        let mut loaded = loaded;
        if !loaded {
            let previous = self.data.as_ref().and_then(|data| data.load_note_prev(&id));
            match previous {
                Some(previous_bytes) => {
                    let message = "This note could not be read. Open its previous saved version?";
                    if dialogs::ask_yes_no(self.hwnd, message) {
                        loaded = editor::load_rtf(
                            editor,
                            &theme::theme_bytes(&self.palette, &previous_bytes),
                        );
                    } else {
                        editor::clear(editor);
                    }
                }
                None => {
                    editor::clear(editor);
                    status::set_message(self, "The note file could not be read; starting empty");
                }
            }
        }
        let _ = loaded;

        // Rich Edit drops EM_SETZOOM when new content is streamed in, so the
        // preferred zoom has to be applied again after the note is loaded.
        editor::set_zoom_percent(editor, self.zoom);

        // Rich Edit's default tab stop is 0.5"; Nodi uses half of that for the
        // whole document (new paragraphs inherit it).
        crate::wnd::format::apply_document_tabs(editor);
        if let Some((from, to)) = selection {
            editor::set_selection(editor, from, to);
        }
        // Headings made by older versions may not have their `# ` marker yet.
        crate::wnd::format::normalize_heading_markers(editor);
        editor::set_modified(editor, false);
        if let Some(line) = scroll {
            editor::scroll_to_line(editor, line);
        }
        editor::set_change_events(editor, true);
        if self.tree.is_locked(&id) {
            editor::set_read_only(editor, true);
            status::set_message(
                self,
                "This note is locked (read-only) — right-click > Unlock to edit",
            );
        }

        let title = self
            .tree
            .get(&id)
            .map(|node| node.title.clone())
            .unwrap_or_else(|| "Untitled".to_owned());
        // A new note's empty file may still be queued: "Saving..." until the
        // writer confirms it (seq 1).
        let first_write_pending = self.pending_new.contains(&id);
        self.tabs.push(Tab {
            editor,
            note: id.clone(),
            title,
            dirty: false,
            seq: u64::from(first_write_pending),
            acked: 0,
            last_error: None,
            failure_streak: false,
            timer: 0,
        });
        let index = self.tabs.len() - 1;
        tabs::insert(self, index, &self.tabs[index].title);
        self.selected = Some(id.clone());
        self.activate_tab_impl(index, false);
        // Selecting is enough; rebuilding the whole tree per opened note made
        // restoring many tabs quadratic.
        tree::select_item(self, &id);
    }

    /// Flushes the active tab (if needed) and switches to `index`.
    pub fn activate_tab(&mut self, index: usize) {
        self.activate_tab_impl(index, true);
    }

    fn activate_tab_impl(&mut self, index: usize, persist: bool) {
        if index >= self.tabs.len() {
            return;
        }
        if self.active != Some(index) {
            self.flush_active();
        }

        // Editors are created hidden; only the outgoing one needs hiding.
        // SAFETY: every handle came from `CreateWindowExW` and is still alive.
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE, SW_SHOW};
            if let Some(previous) = self.active.filter(|previous| *previous != index) {
                if let Some(tab) = self.tabs.get(previous) {
                    ShowWindow(tab.editor, SW_HIDE);
                }
            }
            self.active = Some(index);
            crate::wnd::layout(self);
            ShowWindow(self.tabs[index].editor, SW_SHOW);
        }

        tabs::set_cur(self, index);
        status::update_caret(self);
        status::update_counts(self);
        crate::wnd::format::update_heading_marker(self.tabs[index].editor);
        crate::wnd::toolbar::sync_format(self);
        status::set_message(self, "Ready");
        self.refresh_tab_ui(index);

        let editor = self.tabs[index].editor;
        // SAFETY: the editor window is our child and alive.
        unsafe {
            SetFocus(editor);
        }
        if persist {
            self.save_session_now();
        }
    }

    pub fn close_active_tab(&mut self) {
        if let Some(index) = self.active {
            self.close_tab(index);
        }
    }

    pub fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.submit_tab_if_needed(index);
        let timer = self.tabs[index].timer;
        if timer != 0 {
            // SAFETY: killing a timer we started on our own window.
            unsafe {
                KillTimer(self.hwnd, timer);
            }
        }
        let editor = self.tabs[index].editor;
        if !editor.is_null() {
            crate::wnd::format::forget_editor(editor);
            // SAFETY: the editor is a child window of this process.
            unsafe {
                DestroyWindow(editor);
            }
        }
        self.tabs.remove(index);
        tabs::delete(self, index);

        let previous_active = self.active;
        if self.tabs.is_empty() {
            self.active = None;
            status::set_save_state(self, "");
            status::set_message(self, "No note open");
            status::clear_caret(self);
            status::update_counts(self);
            sync_window_title(self);
            tree::focus_tree(self);
        } else {
            let target = match previous_active {
                Some(active) if active > index => active - 1,
                Some(active) if active < index => active,
                _ => index.min(self.tabs.len() - 1),
            };
            self.active = None;
            self.activate_tab_impl(target, false);
        }

        crate::wnd::layout(self);
        self.update_retry_timer();
        self.save_session_now();
    }

    pub fn cycle_tab(&mut self, delta: i32) {
        let Some(active) = self.active else {
            return;
        };
        let count = self.tabs.len() as i32;
        if count <= 1 {
            return;
        }
        let next = ((active as i32 + delta) % count + count) % count;
        self.activate_tab(next as usize);
    }

    // ------------------------------------------------------------ save machine

    /// `EN_CHANGE`: mark dirty, arm the 500 ms debounce timer, update chrome.
    pub fn mark_dirty(&mut self, editor: HWND) {
        let Some(index) = self.tab_index(editor) else {
            return;
        };
        self.tabs[index].dirty = true;
        let timer = if self.tabs[index].timer == 0 {
            let id = self.timer_id();
            // SAFETY: our own window, id unique to this tab.
            unsafe {
                SetTimer(self.hwnd, id, DEBOUNCE_MS, None);
            }
            self.tabs[index].timer = id;
            id
        } else {
            self.tabs[index].timer
        };
        // Re-arm the timer so rapid keystrokes keep debouncing.
        // SAFETY: same timer id.
        unsafe {
            SetTimer(self.hwnd, timer, DEBOUNCE_MS, None);
        }
        self.refresh_tab_ui(index);
    }

    /// Save state of the active tab, as shown in the status bar.
    pub fn active_state(&self) -> Option<&'static str> {
        self.active
            .filter(|&index| index < self.tabs.len())
            .map(|index| self.state_text(index))
    }

    fn state_text(&self, index: usize) -> &'static str {
        let tab = &self.tabs[index];
        if tab.last_error.is_some() {
            "Save Failed"
        } else if tab.dirty {
            "Modified"
        } else if tab.seq > tab.acked {
            "Saving..."
        } else {
            "Saved"
        }
    }

    /// Updates the tab label (` *` while unsaved) and part 2 / caption when the
    /// tab is active.
    pub fn refresh_tab_ui(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let label = if self.tabs[index].is_saved() {
            self.tabs[index].title.clone()
        } else {
            format!("{} *", self.tabs[index].title)
        };
        tabs::set_title(self, index, &label);
        if self.active == Some(index) {
            let state = self.state_text(index);
            status::set_save_state(self, state);
            sync_window_title(self);
        }
    }

    /// Sets the save state for whichever tab is active.
    pub fn refresh_active_ui(&mut self) {
        if let Some(index) = self.active {
            self.refresh_tab_ui(index);
        }
    }

    fn submit_tab_if_needed(&mut self, index: usize) {
        let needs = {
            let tab = &self.tabs[index];
            tab.dirty || tab.seq > tab.acked || tab.last_error.is_some()
        };
        if needs {
            self.submit_tab(index);
        }
    }

    /// Streams RTF + plain text out and hands them to the writer.
    pub fn submit_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let timer = self.tabs[index].timer;
        if timer != 0 {
            // SAFETY: killing a timer we started.
            unsafe {
                KillTimer(self.hwnd, timer);
            }
            self.tabs[index].timer = 0;
        }

        let editor = self.tabs[index].editor;
        let Some(raw) = editor::stream_out_rtf(editor) else {
            self.tabs[index].last_error = Some("the editor content could not be read".to_owned());
            self.refresh_tab_ui(index);
            return;
        };
        // Dark mode renders auto/black text through sentinel colour entries;
        // the note on disk must keep the user's real colours.
        let rtf = theme::fold_bytes(&self.palette, &raw);
        let txt = editor::plain_text(editor);
        let seq = self.tabs[index].seq + 1;
        let note = self.tabs[index].note.clone();
        // A note that was never renamed by hand is titled by its first line;
        // checking at save time keeps it off the keystroke path. Notes made
        // before titles followed the text still carry the default title:
        // they follow too, once, as soon as they get text (not when locked).
        let default_title = self.tree.get(&note).is_some_and(|node| {
            !node.auto_title
                && !node.locked
                && matches!(node.title.as_str(), "New Note" | "Untitled")
        });
        if default_title && !txt.trim().is_empty() && !self.tree.is_locked(&note) {
            self.tree.set_auto_title(&note, true);
        }
        if let Some(title) = self.tree.follow_text(&note, &txt) {
            self.tree_save();
            tree::set_item_text(self, &note, &title);
            self.tabs[index].title = title;
        }
        self.tabs[index].seq = seq;
        self.tabs[index].dirty = false;

        let request = SaveRequest {
            note,
            seq,
            rtf,
            txt,
        };
        let submitted = self
            .writer
            .as_ref()
            .is_some_and(|writer| writer.submit(Job::Note(request)));
        if !submitted {
            self.tabs[index].last_error = Some("the save queue is closed".to_owned());
        }
        self.refresh_tab_ui(index);
    }

    /// `Ctrl+S` / `File > Save`: submits the active tab even when it is not
    /// dirty, so a reloaded note also gets a fresh acknowledgement.
    pub fn save_active(&mut self) {
        if let Some(index) = self.active {
            self.submit_tab(index);
        }
    }

    /// Flushes the active tab when it has pending work.
    pub fn flush_active(&mut self) {
        if let Some(index) = self.active {
            self.submit_tab_if_needed(index);
        }
    }

    /// Flushes every tab (used on close / session end).
    pub fn flush_all(&mut self) {
        for index in 0..self.tabs.len() {
            self.submit_tab_if_needed(index);
        }
    }

    /// Applies a result posted by the writer thread.
    pub fn on_save_result(&mut self, result: SaveResult) {
        match result.target {
            Target::Note(note) => {
                // The first write may be coalesced with a later one, so any
                // result for the note settles it.
                self.pending_new.remove(&note);
                let Some(index) = self.tab_index_for_note(&note) else {
                    return;
                };
                let mut failure: Option<(String, String)> = None;
                match result.result {
                    Ok(()) => {
                        let tab = &mut self.tabs[index];
                        tab.acked = tab.acked.max(result.seq);
                        tab.last_error = None;
                        tab.failure_streak = false;
                    }
                    Err(message) => {
                        let tab = &mut self.tabs[index];
                        tab.last_error = Some(message.clone());
                        if !tab.failure_streak {
                            tab.failure_streak = true;
                            failure = Some((tab.title.clone(), message));
                        }
                    }
                }
                self.refresh_tab_ui(index);
                self.update_retry_timer();
                if let Some((title, message)) = failure {
                    dialogs::save_failed(self.hwnd, &title, &message);
                }
            }
            Target::Tree => {
                if let Err(message) = result.result {
                    status::set_message(self, &format!("Tree save failed: {message}"));
                }
            }
            Target::Session => {
                if let Err(message) = result.result {
                    status::set_message(self, &format!("Session save failed: {message}"));
                }
            }
            Target::Mirror => {
                if let Err(message) = result.result {
                    status::set_message(self, &message);
                }
            }
        }
    }

    /// Timer callback: either a per-tab debounce or the 5 s retry.
    pub fn on_timer(&mut self, id: usize) {
        if id == SESSION_TIMER {
            self.write_session();
            return;
        }
        if id == status::COUNTS_TIMER {
            // SAFETY: one-shot use of our own timer.
            unsafe {
                KillTimer(self.hwnd, status::COUNTS_TIMER);
            }
            status::update_counts(self);
            return;
        }
        if id == RETRY_TIMER {
            let failing: Vec<usize> = (0..self.tabs.len())
                .filter(|&index| self.tabs[index].last_error.is_some())
                .collect();
            for index in failing {
                self.submit_tab(index);
            }
            self.update_retry_timer();
            return;
        }
        if let Some(index) = self.tabs.iter().position(|tab| tab.timer == id) {
            self.tabs[index].timer = 0;
            self.submit_tab(index);
        }
    }

    fn update_retry_timer(&self) {
        let failing = self.tabs.iter().any(|tab| tab.last_error.is_some());
        // SAFETY: our own window and a fixed timer id.
        unsafe {
            if failing {
                SetTimer(self.hwnd, RETRY_TIMER, RETRY_MS, None);
            } else {
                KillTimer(self.hwnd, RETRY_TIMER);
            }
        }
    }

    /// Reclaims the posted `SaveResult` boxes without re-entering the window
    /// procedure (used right before shutdown decisions).
    fn drain_save_results(&mut self) {
        loop {
            let mut message: windows_sys::Win32::UI::WindowsAndMessaging::MSG =
                unsafe { std::mem::zeroed() };
            // SAFETY: peek only our own custom message for our window.
            let more = unsafe {
                PeekMessageW(
                    &mut message,
                    self.hwnd,
                    WM_APP_SAVE_RESULT,
                    WM_APP_SAVE_RESULT,
                    PM_REMOVE,
                )
            };
            if more == 0 {
                break;
            }
            let raw = message.lParam as *mut SaveResult;
            if !raw.is_null() {
                // SAFETY: the writer allocated this box for us.
                let result = unsafe { *Box::from_raw(raw) };
                self.on_save_result(result);
            }
        }
    }

    /// `WM_CLOSE`: flush everything, wait for the writer, then ask before
    /// dropping unsaved content. Returns true when the app may close.
    pub fn maybe_close(&mut self) -> bool {
        self.flush_all();
        self.write_session();
        if let Some(writer) = &self.writer {
            let _ = writer.flush_blocking(Duration::from_secs(5));
        }
        self.drain_save_results();

        let unsaved: Vec<String> = self
            .tabs
            .iter()
            .filter(|tab| tab.seq > tab.acked || tab.last_error.is_some())
            .map(|tab| tab.title.clone())
            .collect();
        if unsaved.is_empty() {
            return true;
        }
        dialogs::unsaved_close(self.hwnd, &unsaved)
    }

    /// `WM_QUERYENDSESSION` / `WM_ENDSESSION`: best-effort synchronous flush.
    pub fn flush_for_session_end(&mut self) {
        self.flush_all();
        self.write_session();
        if let Some(writer) = &self.writer {
            let _ = writer.flush_blocking(Duration::from_secs(5));
        }
        self.drain_save_results();
    }

    // ---------------------------------------------------------------- session

    fn collect_session(&self) -> Session {
        let mut tabs = Vec::with_capacity(self.tabs.len());
        for tab in &self.tabs {
            let (caret, _) = editor::selection(tab.editor);
            tabs.push(TabState {
                note: tab.note.clone(),
                caret: caret.max(0) as u32,
                scroll: editor::first_visible_line(tab.editor),
            });
        }
        let mut expanded: Vec<NoteId> = self.expanded.iter().cloned().collect();
        expanded.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Session {
            tabs,
            active: self.active.unwrap_or(0),
            sidebar_width: self.sidebar_width,
            window: self.window_rect(),
            expanded,
            word_wrap: self.word_wrap,
            zoom: self.zoom,
            recent: self.recent.iter().take(10).cloned().collect(),
        }
    }

    fn window_rect(&self) -> Option<WindowRect> {
        let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
        placement.length = size_of::<WINDOWPLACEMENT>() as u32;
        // SAFETY: `placement` is valid for the call.
        if unsafe { GetWindowPlacement(self.hwnd, &mut placement) } == 0 {
            return None;
        }
        let rect = placement.rcNormalPosition;
        Some(WindowRect {
            x: rect.left,
            y: rect.top,
            width: rect.right - rect.left,
            height: rect.bottom - rect.top,
        })
    }

    /// Schedules a `session.json` save (debounced: tabs, caret and window
    /// state change constantly). Closing writes it immediately.
    pub fn save_session_now(&self) {
        // SAFETY: our own window and a fixed timer id.
        unsafe {
            SetTimer(self.hwnd, SESSION_TIMER, SESSION_DELAY_MS, None);
        }
    }

    /// Queues `session.json` on the writer thread right away (no disk I/O on
    /// the UI thread).
    pub fn write_session(&self) {
        // SAFETY: our own window and a fixed timer id.
        unsafe {
            KillTimer(self.hwnd, SESSION_TIMER);
        }
        let session = self.collect_session();
        match &self.writer {
            Some(writer) => {
                writer.submit(Job::SessionSave(session.to_json()));
            }
            None => {
                if let Some(data) = &self.data {
                    let _ = data.save_session(&session);
                }
            }
        }
    }

    // ------------------------------------------------------------------- tree

    fn create_note_node(&mut self, kind: NodeKind, title: &str, parent: Option<NoteId>) -> NoteId {
        let id = self.tree.new_node(kind, title, parent);
        if kind == NodeKind::Note {
            // The empty file is written by the writer thread; the tab opened
            // for the note shows "Saving..." until that write is confirmed.
            if let Some(writer) = &self.writer {
                writer.submit(Job::Note(SaveRequest {
                    note: id.clone(),
                    seq: 1,
                    rtf: Vec::new(),
                    txt: String::new(),
                }));
                self.pending_new.insert(id.clone());
            }
        }
        id
    }

    pub fn tree_save(&self) {
        let Ok(json) = self.tree.to_json() else {
            return;
        };
        if let Some(writer) = &self.writer {
            writer.submit(Job::TreeSave(json));
        }
    }

    pub fn rebuild_tree(&mut self) {
        tree::rebuild(self);
    }

    /// Creates a note/folder relative to the selected (or context) node.
    pub fn create_note(&mut self, kind: NodeKind, as_child: bool) {
        let anchor = self.context_note.clone().or_else(|| self.selected.clone());
        let mut parent = match (&anchor, as_child) {
            (Some(id), true) => Some(id.clone()),
            (Some(id), false) => self.tree.get(id).and_then(|node| node.parent.clone()),
            (None, _) => None,
        };
        // Nothing new goes into a locked folder: it lands next to it.
        while let Some(locked) = parent.clone().filter(|id| self.tree.is_locked(id)) {
            parent = self.tree.get(&locked).and_then(|node| node.parent.clone());
        }
        let title = match kind {
            NodeKind::Note => "New Note",
            NodeKind::Folder => "New Folder",
        };
        let id = self.create_note_node(kind, title, parent.clone());
        if kind == NodeKind::Note {
            // Titled by its first line until the user renames it.
            self.tree.set_auto_title(&id, true);
        }
        self.tree_save();
        self.selected = Some(id.clone());
        // Insert just the new item; rebuilding the whole tree made every new
        // note slower than the last.
        if !tree::add_node(self, &id) {
            self.rebuild_tree();
        }
        self.expand(parent);
        if kind == NodeKind::Note {
            self.open_note(id);
        } else {
            tree::begin_rename(self);
        }
        self.save_session_now();
    }

    /// IDs of a node and its whole subtree.
    fn subtree_ids(&self, id: &NoteId) -> Vec<NoteId> {
        let mut ids = vec![id.clone()];
        ids.extend(self.tree.descendants(id));
        ids
    }

    pub fn trash_node(&mut self, id: NoteId) {
        let title = self
            .tree
            .get(&id)
            .map(|node| node.title.clone())
            .unwrap_or_default();
        let sub_notes = self
            .tree
            .descendants(&id)
            .into_iter()
            .filter(|child| {
                self.tree
                    .get(child)
                    .is_some_and(|node| node.kind == NodeKind::Note)
            })
            .count();
        let question = if sub_notes == 0 {
            format!("Move \"{title}\" to the Trash?")
        } else if sub_notes == 1 {
            format!("Move \"{title}\" (and its 1 sub-note) to the Trash?")
        } else {
            format!("Move \"{title}\" (and its {sub_notes} sub-notes) to the Trash?")
        };
        if !dialogs::ask_yes_no(self.hwnd, &question) {
            return;
        }
        let ids = self.subtree_ids(&id);
        for index in (0..self.tabs.len()).rev() {
            if ids.contains(&self.tabs[index].note) {
                self.close_tab(index);
            }
        }
        if let Some(change) = self.tree.trash_undoable(&id) {
            self.remember_tree_change(change);
            self.tree_save();
            self.rebuild_tree();
            status::set_message(self, "Moved to Trash (Ctrl+Z in the tree undoes it)");
            self.save_session_now();
        }
    }

    pub fn restore_node(&mut self, id: NoteId) {
        if self.tree.restore(&id) {
            self.tree_save();
            self.rebuild_tree();
            status::set_message(self, "Restored from Trash");
        }
    }

    pub fn purge_node(&mut self, id: NoteId) {
        let removed = self.tree.purge(&id);
        if removed.is_empty() {
            return;
        }
        if let Some(data) = &self.data {
            for note in &removed {
                // Files are moved to `recovery/purged-...`, never deleted.
                let _ = data.purge_note_files(note);
            }
        }
        self.tree_save();
        self.rebuild_tree();
        status::set_message(self, "Deleted permanently (files kept in recovery/)");
    }

    /// "Empty Trash...": purges every trashed item after one confirmation.
    pub fn empty_trash(&mut self) {
        let entries = self.trashed_roots();
        if entries.is_empty() {
            dialogs::info(self.hwnd, "The Trash is empty.");
            return;
        }
        let question = format!(
            "Permanently delete all {} item(s) in the Trash?\n\n\
             Their files are moved to data/recovery/ (nothing is deleted).",
            entries.len()
        );
        if !dialogs::ask_yes_no(self.hwnd, &question) {
            return;
        }
        let count = entries.len();
        for (id, _) in entries {
            self.purge_node(id);
        }
        status::set_message(
            self,
            &format!("Trash emptied ({count} item(s); files kept in recovery/)"),
        );
    }

    pub fn duplicate_node(&mut self, id: NoteId) {
        let Some(new_id) = self.tree.duplicate(&id) else {
            return;
        };
        let source_is_note = self
            .tree
            .get(&id)
            .is_some_and(|node| node.kind == NodeKind::Note);
        if source_is_note {
            if let Some(data) = &self.data {
                if let Some(rtf) = data.load_note(&id) {
                    let txt = data.load_note_text(&id).unwrap_or_default();
                    let _ = data.save_note(&new_id, &rtf, &txt);
                }
            }
        }
        self.tree_save();
        self.selected = Some(new_id);
        self.rebuild_tree();
        status::set_message(self, "Duplicated");
    }

    pub fn rename_node(&mut self, id: NoteId, title: String) {
        let title = title.trim().to_owned();
        if title.is_empty() || !self.check_unlocked(&id) {
            return;
        }
        let change = self.tree.rename_undo(&id, &title);
        if !self.tree.rename(&id, title.clone()) {
            return;
        }
        if let Some(change) = change.filter(|change| match change {
            nodi_core::TreeUndo::Rename { title: old, .. } => *old != title,
            _ => true,
        }) {
            self.remember_tree_change(change);
        }
        self.tree_save();
        tree::set_item_text(self, &id, &title);
        if let Some(index) = self.tab_index_for_note(&id) {
            self.tabs[index].title = title;
            self.refresh_tab_ui(index);
        }
    }

    pub fn set_icon(&mut self, id: NoteId, key: &str) {
        if !self.tree.set_icon(&id, Some(key)) {
            return;
        }
        let index = self
            .icon_keys
            .iter()
            .position(|candidate| *candidate == key)
            .unwrap_or(0) as i32;
        tree::set_item_icon(self, &id, index);
        self.tree_save();
    }

    /// Moves a node before / after `target`, or inside it (last child).
    pub fn move_node_relative(
        &mut self,
        source: NoteId,
        target: NoteId,
        position: Position,
    ) -> Result<(), String> {
        if source == target {
            return Ok(());
        }
        let change = self.tree.move_undo(&source);
        match self.tree.move_relative(&source, &target, position) {
            Ok(()) => {
                if let Some(change) = change {
                    self.remember_tree_change(change);
                }
                self.tree_save();
                if position == Position::Inside {
                    self.expand(Some(target));
                }
                self.selected = Some(source);
                self.rebuild_tree();
                let message = match position {
                    Position::Before | Position::After => "Reordered",
                    Position::Inside => "Moved",
                };
                status::set_message(self, message);
                Ok(())
            }
            Err(err) => {
                status::set_message(self, &format!("Move rejected: {err}"));
                Err(err.to_string())
            }
        }
    }

    /// Notes > Move to...: pick any other node (or the top level) as the new
    /// parent and move the selection there as the last child.
    pub fn move_selected_to_picker(&mut self) {
        let ids = self.selection_ids();
        let Some(id) = ids.first().cloned() else {
            return;
        };
        if ids.len() > 1 {
            let mut entries: Vec<(Option<NoteId>, String)> =
                vec![(None, "— Top level —".to_owned())];
            for node in self.tree.iter_ordered() {
                if node.trashed || ids.iter().any(|moved| self.tree.is_within(&node.id, moved)) {
                    continue;
                }
                let depth = self.tree.ancestors(&node.id).len();
                entries.push((
                    Some(node.id.clone()),
                    format!("{}{}", "    ".repeat(depth), node.title),
                ));
            }
            if let Some(choice) = dialogs::move_to(self.hwnd, &entries) {
                self.move_selection_into(choice);
            }
            return;
        }
        let mut entries: Vec<(Option<NoteId>, String)> = vec![(None, "— Top level —".to_owned())];
        for node in self.tree.iter_ordered() {
            if node.trashed || self.tree.is_within(&node.id, &id) {
                continue;
            }
            let depth = self.tree.ancestors(&node.id).len();
            let kind = match node.kind {
                NodeKind::Folder => "folder",
                NodeKind::Note => "note",
            };
            entries.push((
                Some(node.id.clone()),
                format!("{}{} ({kind})", "    ".repeat(depth), node.title),
            ));
        }
        let Some(choice) = dialogs::move_to(self.hwnd, &entries) else {
            return;
        };
        // The same checks as for several notes (locked notes and folders).
        self.context_note = Some(id);
        self.move_selection_into(choice);
    }

    pub fn expand(&mut self, id: Option<NoteId>) {
        if let Some(id) = id {
            if self.expanded.insert(id.clone()) {
                let _ = self.tab_index_for_note(&id);
                crate::wnd::tree::expand_item(self, &id);
            }
        }
    }

    pub fn expand_all(&mut self) {
        let ids: Vec<NoteId> = self
            .tree
            .iter_ordered()
            .into_iter()
            .filter(|node| !node.trashed)
            .map(|node| node.id.clone())
            .collect();
        // Only parents need an expand flag.
        for id in ids {
            let has_children = !self.tree.children(Some(&id)).is_empty();
            if has_children {
                self.expanded.insert(id);
            }
        }
        self.rebuild_tree();
        self.save_session_now();
    }

    pub fn collapse_all(&mut self) {
        self.expanded.clear();
        self.rebuild_tree();
        self.save_session_now();
    }

    /// One line per trashed root, for the Trash window.
    pub fn trashed_roots(&self) -> Vec<(NoteId, String)> {
        let mut entries = Vec::new();
        for node in self.tree.iter_ordered() {
            if !node.trashed {
                continue;
            }
            let parent_trashed = node
                .parent
                .as_ref()
                .and_then(|parent| self.tree.get(parent))
                .is_some_and(|parent| parent.trashed);
            if !parent_trashed {
                entries.push((node.id.clone(), node.title.clone()));
            }
        }
        entries
    }

    /// Every visible title, for the Find Note dialog.
    pub fn visible_titles(&self) -> Vec<(NoteId, String)> {
        self.tree
            .iter_ordered()
            .into_iter()
            .filter(|node| !node.trashed)
            .map(|node| (node.id.clone(), node.title.clone()))
            .collect()
    }
}

// ------------------------------------------------- settings, theme and tools

impl App {
    pub(crate) fn save_settings(&self) {
        if let Some(data) = &self.data {
            let _ = data.save_settings(&self.settings);
        }
    }

    /// View > Spell Check.
    pub fn toggle_spell_check(&mut self) {
        self.settings.spell_check = !self.settings.spell_check;
        for tab in &self.tabs {
            editor::set_spell_check(tab.editor, self.settings.spell_check);
        }
        self.save_settings();
        menu::set_checked(self, menu::IDM_VIEW_SPELL, self.settings.spell_check);
        status::set_message(
            self,
            if self.settings.spell_check {
                "Spell check on"
            } else {
                "Spell check off"
            },
        );
    }

    /// View > Ctrl+Up/Down Scrolls the View.
    pub fn toggle_ctrl_arrow_scroll(&mut self) {
        self.settings.ctrl_arrow_scroll = !self.settings.ctrl_arrow_scroll;
        crate::wnd::editkeys::set_ctrl_arrow_scroll(self.settings.ctrl_arrow_scroll);
        self.save_settings();
        menu::set_checked(
            self,
            menu::IDM_VIEW_CTRL_SCROLL,
            self.settings.ctrl_arrow_scroll,
        );
        status::set_message(
            self,
            if self.settings.ctrl_arrow_scroll {
                "Ctrl+Up/Down scroll the view"
            } else {
                "Ctrl+Up/Down move by paragraph"
            },
        );
    }

    /// Tools > Rebuild AI Markdown Mirror.
    pub fn rebuild_mirror(&mut self) {
        let Some(writer) = &self.writer else {
            return;
        };
        let Ok(json) = self.tree.to_json() else {
            return;
        };
        writer.submit(Job::MirrorRebuild(json));
        status::set_message(self, "Rebuilding the AI Markdown mirror...");
    }

    /// Tools > Keep Markdown Mirror for AI Agents.
    pub fn toggle_mirror(&mut self) {
        self.settings.markdown_mirror = !self.settings.markdown_mirror;
        if let Some(writer) = &self.writer {
            writer.submit(Job::MirrorEnabled(self.settings.markdown_mirror));
        }
        self.save_settings();
        menu::set_checked(
            self,
            menu::IDM_TOOLS_MIRROR_TOGGLE,
            self.settings.markdown_mirror,
        );
        if self.settings.markdown_mirror {
            status::set_message(self, "Markdown mirror on");
            self.rebuild_mirror();
        } else {
            status::set_message(self, "Markdown mirror off");
        }
    }

    /// Tools > Restore Backup...
    pub fn restore_backup_ui(&mut self) {
        let Some(data) = self.data.clone() else {
            return;
        };
        let snapshots = backup::list(&data).unwrap_or_default();
        if snapshots.is_empty() {
            dialogs::info(self.hwnd, "There are no backups yet.");
            return;
        }
        let entries: Vec<(String, std::path::PathBuf)> = snapshots
            .iter()
            .rev()
            .map(|path| (snapshot_label(path), path.clone()))
            .collect();
        let Some(snapshot) = dialogs::pick_snapshot(self.hwnd, &entries) else {
            return;
        };
        let question = format!(
            "Restore the backup from {}?\n\n\
             Nodi snapshots the current state first, so this can be undone.",
            snapshot
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        );
        if !dialogs::ask_yes_no(self.hwnd, &question) {
            return;
        }
        self.restore_snapshot(&snapshot);
    }

    /// Restores one snapshot, dropping every open tab first (nothing pending
    /// may overwrite the restored files). Also used by the `restore-latest`
    /// test hook.
    pub fn restore_snapshot(&mut self, snapshot: &std::path::Path) {
        let Some(data) = self.data.clone() else {
            return;
        };
        // Everything queued belongs to the state we are about to replace.
        self.discard_all_tabs();
        if let Some(writer) = &self.writer {
            let _ = writer.flush_blocking(Duration::from_secs(5));
        }

        match backup::restore(&data, snapshot) {
            Ok(safety) => match data.load_tree() {
                Ok(tree) => {
                    self.tree = tree;
                    self.recent.clear();
                    menu::refresh_recent(self);
                    self.selected = None;
                    self.context_note = None;
                    self.expanded.clear();
                    self.rebuild_tree();
                    if let Some(id) = self.first_visible_note() {
                        self.selected = Some(id.clone());
                        self.open_note(id);
                    }
                    self.save_session_now();
                    status::set_message(
                        self,
                        &format!(
                            "Backup restored (previous state: {})",
                            safety
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        ),
                    );
                }
                Err(err) => dialogs::error(
                    self.hwnd,
                    &format!("The restored tree.json could not be read:\n{err}"),
                ),
            },
            Err(err) => dialogs::error(self.hwnd, &format!("Restore failed:\n{err}")),
        }
    }

    /// Test hook: restores the newest snapshot without any dialog.
    pub fn restore_latest_backup(&mut self) -> Result<String, String> {
        let Some(data) = self.data.clone() else {
            return Err("no data directory".to_owned());
        };
        let snapshots = backup::list(&data).map_err(|err| err.to_string())?;
        let Some(latest) = snapshots.last().cloned() else {
            return Err("no backups".to_owned());
        };
        self.restore_snapshot(&latest);
        Ok(latest
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default())
    }

    /// Notes > Change Icon... > From file...
    pub fn add_icon_file(&mut self, path: &std::path::Path) -> Result<&'static str, String> {
        let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "icon".to_owned());
        let Some(data) = &self.data else {
            return Err("No data directory".to_owned());
        };
        let stored = data
            .save_icon(&name, &bytes)
            .map_err(|err| err.to_string())?;
        let key: &'static str = Box::leak(format!("file:{name}").into_boxed_str());
        if !self.icon_keys.contains(&key) && !tree::add_icon(self, key, &stored) {
            return Err(format!(
                "{} is not a usable .ico or 16–32 px .png icon.",
                path.display()
            ));
        }
        Ok(key)
    }

    /// Removes the icon override, back to the default note/folder glyph.
    pub fn clear_icon(&mut self, id: NoteId) {
        if !self.tree.set_icon(&id, None) {
            return;
        }
        let default = self
            .tree
            .get(&id)
            .map(|node| match node.kind {
                NodeKind::Folder => "folder",
                NodeKind::Note => "note",
            })
            .unwrap_or("note");
        let index = self
            .icon_keys
            .iter()
            .position(|candidate| *candidate == default)
            .unwrap_or(0) as i32;
        tree::set_item_icon(self, &id, index);
        self.tree_save();
    }
}

/// Human label for Tools > Restore Backup: `20260102-030405 — 1.2 MB`.
fn snapshot_label(path: &std::path::Path) -> String {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("{name} — {}", human_size(directory_size(path)))
}

fn directory_size(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if let Ok(meta) = entry.metadata() {
                total += meta.len();
            }
        }
    }
    total
}

fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// View > Outline (Ctrl+Alt+O).
pub fn show_outline(app: &mut App) {
    crate::wnd::outline::show(app);
}

fn snapshot_is_stale(path: &std::path::Path) -> bool {
    match std::fs::metadata(path).and_then(|meta| meta.modified()) {
        Ok(modified) => modified
            .elapsed()
            .map(|age| age > BACKUP_MAX_AGE)
            .unwrap_or(true),
        Err(_) => true,
    }
}

// ------------------------------------------------------------------ helpers

/// Keeps the frame caption in sync with the active tab (`<title> - Nodi`).
pub fn sync_window_title(app: &App) {
    if app.hwnd.is_null() {
        return;
    }
    let caption = match app.active.and_then(|index| app.tabs.get(index)) {
        Some(tab) => format!("{} - Nodi", tab.title),
        None => "Nodi".to_owned(),
    };
    let text = win::wide(&caption);
    // SAFETY: the buffer is NUL terminated and outlives the call.
    unsafe {
        SetWindowTextW(app.hwnd, text.as_ptr());
    }
}

/// Creates (or recreates, after a DPI change) the UI fonts and applies them to
/// every control that currently exists.
pub fn recreate_fonts(app: &mut App) {
    // SAFETY: both calls only read system metrics and create a GDI font.
    let (face, size) = crate::wnd::theme::ui_font();
    let ui = unsafe { make_ui_font(app.dpi, false, &face, size) };
    let bold = unsafe { make_ui_font(app.dpi, true, &face, size) };
    let old_ui = std::mem::replace(&mut app.ui_font, ui);
    let old_bold = std::mem::replace(&mut app.bold_font, bold);

    apply_fonts(app);

    delete_font(old_ui);
    delete_font(old_bold);
}

pub fn destroy_fonts(app: &mut App) {
    let ui = std::mem::replace(&mut app.ui_font, ptr::null_mut());
    let bold = std::mem::replace(&mut app.bold_font, ptr::null_mut());
    delete_font(ui);
    delete_font(bold);
    if app.images != 0 {
        // SAFETY: the image list belongs to the tree we created.
        unsafe {
            windows_sys::Win32::UI::Controls::ImageList_Destroy(app.images);
        }
        app.images = 0;
    }
}

fn delete_font(font: HFONT) {
    if font.is_null() {
        return;
    }
    // SAFETY: `font` was created by `CreateFontIndirectW` and has already been
    // replaced in every control (or its window is gone).
    unsafe {
        DeleteObject(font);
    }
}

/// `WM_SETFONT` for every control that uses the chrome font; the sidebar
/// caption gets the bold variant.
pub fn apply_fonts(app: &App) {
    if app.ui_font.is_null() {
        return;
    }
    // SAFETY: all handles are null (skipped) or child windows created by this
    // process; `WM_SETFONT` only stores the handle.
    unsafe {
        for control in [
            app.tree_view,
            app.tabctrl,
            app.status,
            app.toolbar,
            app.formatbar,
            app.style_combo,
            app.font_combo,
            app.size_combo,
        ] {
            if !control.is_null() {
                SendMessageW(control, WM_SETFONT, app.ui_font as usize, 1);
            }
        }
        if !app.sidebar_title.is_null() && !app.bold_font.is_null() {
            SendMessageW(app.sidebar_title, WM_SETFONT, app.bold_font as usize, 1);
        }
    }
}

unsafe fn make_ui_font(dpi: u32, bold: bool, face: &str, size: u32) -> HFONT {
    let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
    metrics.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
    let ok = SystemParametersInfoW(
        SPI_GETNONCLIENTMETRICS,
        metrics.cbSize,
        &mut metrics as *mut _ as *mut core::ffi::c_void,
        0,
    );

    let mut logfont: LOGFONTW = if ok != 0 {
        metrics.lfMessageFont
    } else {
        let mut fallback: LOGFONTW = std::mem::zeroed();
        fallback.lfHeight = -12;
        fallback.lfWeight = FW_NORMAL as i32;
        fallback.lfCharSet = DEFAULT_CHARSET;
        set_face(&mut fallback.lfFaceName, "Segoe UI");
        fallback
    };

    logfont.lfHeight = win::mul_div(logfont.lfHeight, dpi as i32, 96);
    if logfont.lfHeight == 0 {
        logfont.lfHeight = -win::mul_div(12, dpi as i32, 96);
    }
    // The theme's chrome font (Segoe UI 9 pt is the Windows default).
    if !face.is_empty() && !(face == "Segoe UI" && size == 9) {
        set_face(&mut logfont.lfFaceName, face);
        logfont.lfHeight = -win::mul_div(size.clamp(7, 16) as i32, dpi as i32, 72);
    }
    if bold {
        logfont.lfWeight = FW_BOLD as i32;
    }

    CreateFontIndirectW(&logfont)
}

fn set_face(buffer: &mut [u16; 32], name: &str) {
    buffer.fill(0);
    for (slot, unit) in buffer.iter_mut().zip(name.encode_utf16()) {
        *slot = unit;
    }
}

/// `EN_CHANGE` notification from an editor.
pub fn on_editor_change(app: &mut App, editor: HWND) {
    if app.suppress_change > 0 {
        return;
    }
    if app.tab_index(editor).is_none() {
        return;
    }
    app.mark_dirty(editor);
    crate::wnd::gutter::invalidate(editor);
    crate::wnd::gutter::refresh(app);
    status::update_caret(app);
    status::schedule_counts(app);
    crate::wnd::format::maintain_heading(editor);
    crate::wnd::format::update_heading_marker(editor);
}

/// `TCN_SELCHANGE` from the tab control.
pub fn on_tab_selchange(app: &mut App) {
    let Some(selected) = tabs::cur_sel(app) else {
        return;
    };
    if app.active != Some(selected) {
        app.activate_tab(selected);
    }
}

/// `Notes > Trash...`: modal list of trashed roots, one action per opening.
pub fn show_trash(app: &mut App) {
    loop {
        let entries = app.trashed_roots();
        if entries.is_empty() {
            dialogs::info(app.hwnd, "The Trash is empty.");
            return;
        }
        match dialogs::trash(app.hwnd, &entries) {
            dialogs::TrashChoice::Restore(id) => app.restore_node(id),
            dialogs::TrashChoice::Purge(id) => app.purge_node(id),
            dialogs::TrashChoice::Close => return,
        }
    }
}

/// `Search > Find Note by Title...`.
pub fn show_find_note(app: &mut App) {
    let entries = app.visible_titles();
    if let Some(id) = dialogs::find_note(app.hwnd, &entries) {
        app.open_note(id);
    }
}

/// `Notes > Rename` / F2: starts inline label editing on the selected item.
pub fn rename_selected(app: &mut App) {
    if let Some(id) = app.context_note.clone().or_else(|| app.selected.clone()) {
        if !app.check_unlocked(&id) {
            return;
        }
    }
    tree::begin_rename(app);
}

/// Properties dialog for the selected node.
pub fn show_properties(app: &mut App) {
    let Some(id) = app.context_note.clone().or_else(|| app.selected.clone()) else {
        return;
    };
    let Some(node) = app.tree.get(&id) else {
        return;
    };
    let children = app.tree.children(Some(&id)).len();
    let kind = match node.kind {
        NodeKind::Folder => "Folder",
        NodeKind::Note => "Note",
    };
    let message = format!(
        "Title: {}\nId: {}\nKind: {kind}\nCreated: {}\nModified: {}\nChildren: {children}",
        node.title,
        node.id,
        format_time(node.created),
        format_time(node.modified),
    );
    dialogs::info(app.hwnd, &message);
}

/// `Notes > Change Icon...`.
pub fn change_icon(app: &mut App) {
    let Some(id) = app.context_note.clone().or_else(|| app.selected.clone()) else {
        return;
    };
    if !app.check_unlocked(&id) {
        return;
    }
    let current = app
        .tree
        .get(&id)
        .and_then(|node| node.icon.clone())
        .unwrap_or_default();
    if let Some(choice) = dialogs::pick_icon(app.hwnd, &current) {
        match choice {
            dialogs::IconChoice::Builtin(key) => app.set_icon(id, key),
            dialogs::IconChoice::Remove => app.clear_icon(id),
            dialogs::IconChoice::File(path) => match app.add_icon_file(&path) {
                Ok(key) => app.set_icon(id, key),
                Err(message) => dialogs::error(app.hwnd, &message),
            },
        }
    }
}

/// Opens the selected note (context menu "Open").
pub fn open_selected(app: &mut App) {
    if let Some(id) = app.context_note.clone().or_else(|| app.selected.clone()) {
        app.open_note(id);
    }
}

impl App {
    /// Records a Notes Explorer change for Ctrl+Z (tree focused).
    pub fn remember_tree_change(&mut self, change: nodi_core::TreeUndo) {
        self.tree_undo.push(change);
        if self.tree_undo.len() > TREE_UNDO_LIMIT {
            self.tree_undo.remove(0);
        }
    }

    /// True while the Notes Explorer has the keyboard focus.
    pub fn tree_has_focus(&self) -> bool {
        // SAFETY: plain focus query.
        let focus = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus() };
        !self.tree_view.is_null() && focus == self.tree_view
    }

    /// Ctrl+Z in the Notes Explorer: asks, then takes back the newest change
    /// (restore from the Trash, move back, rename back).
    pub fn undo_tree_change(&mut self) {
        let Some(change) = self.tree_undo.last().cloned() else {
            status::set_message(self, "Nothing to undo in the Notes Explorer");
            return;
        };
        let question = change.question(&self.tree);
        if !dialogs::ask_yes_no(self.hwnd, &question) {
            return;
        }
        self.tree_undo.pop();
        match self.tree.undo(&change) {
            Ok(()) => {
                self.tree_save();
                self.selected = match &change {
                    nodi_core::TreeUndo::Rename { id, .. }
                    | nodi_core::TreeUndo::Move { id, .. } => Some(id.clone()),
                    nodi_core::TreeUndo::Trash { ids, .. } => ids.first().cloned(),
                    nodi_core::TreeUndo::Batch { .. } => self.selected.clone(),
                };
                self.rebuild_tree();
                self.refresh_titles();
                status::set_message(self, "Undone");
            }
            Err(err) => status::set_message(self, &format!("Undo failed: {err}")),
        }
    }

    /// Tab captions after titles changed outside a rename (undo).
    fn refresh_titles(&mut self) {
        for index in 0..self.tabs.len() {
            let title = self
                .tree
                .get(&self.tabs[index].note)
                .map(|node| node.title.clone());
            if let Some(title) = title {
                if self.tabs[index].title != title {
                    self.tabs[index].title = title;
                    self.refresh_tab_ui(index);
                }
            }
        }
    }

    /// Notes > Move Up / Move Down (Alt+Up / Alt+Down).
    pub fn move_selected(&mut self, delta: i32) {
        let Some(id) = self.context_note.clone().or_else(|| self.selected.clone()) else {
            return;
        };
        if self.tree.subtree_locked(&id) {
            self.check_unlocked(&id);
            return;
        }
        if self.settings.tree_sort != nodi_core::tree::SortOrder::Manual {
            status::set_message(
                self,
                "The notes are sorted — View > Sort Notes > Manual to arrange them by hand",
            );
            return;
        }
        let change = self.tree.move_undo(&id);
        if self.tree.move_among_siblings(&id, delta) {
            if let Some(change) = change {
                self.remember_tree_change(change);
            }
            self.tree_save();
            self.selected = Some(id);
            self.rebuild_tree();
        }
    }
}

/// Insert > Image...: file picker, then the picture goes in at the caret.
pub fn insert_image_file(app: &mut App) {
    if crate::wnd::editor::refuse_if_locked(app) {
        return;
    }
    let Some(editor) = app.active_editor() else {
        status::set_message(app, "Open a note first");
        return;
    };
    let Some((_, bytes)) = image::choose_file(app.hwnd) else {
        return;
    };
    match image::insert_bytes(editor, &bytes) {
        Ok(()) => app.mark_dirty(editor),
        Err(message) => dialogs::error(app.hwnd, &message),
    }
    // SAFETY: focusing our own editor.
    unsafe {
        SetFocus(editor);
    }
}

/// Insert > Table...
pub fn insert_table(app: &mut App) {
    if crate::wnd::editor::refuse_if_locked(app) {
        return;
    }
    let Some(editor) = app.active_editor() else {
        status::set_message(app, "Open a note first");
        return;
    };
    let Some(values) = Form::new("Insert Table")
        .field(Field::new("Rows:", "3"))
        .field(Field::new("Columns:", "3"))
        .show(app.hwnd)
    else {
        return;
    };
    let rows = values[0].trim().parse::<usize>().unwrap_or(0);
    let cols = values[1].trim().parse::<usize>().unwrap_or(0);
    if rows == 0 || cols == 0 || cols > 63 {
        dialogs::error(app.hwnd, "Enter at least 1 row and 1 to 63 columns.");
        return;
    }
    match table::insert(editor, rows, cols) {
        Ok(()) => app.mark_dirty(editor),
        Err(message) => dialogs::error(app.hwnd, &message),
    }
    // SAFETY: focusing our own editor.
    unsafe {
        SetFocus(editor);
    }
}

/// Edit > Go to Line... (lines = paragraphs, as in Notepad++ with wrapping).
pub fn go_to_line(app: &mut App) {
    let Some(editor) = app.active_editor() else {
        return;
    };
    let total = editor::text_units(editor, 0, editor::text_length(editor))
        .iter()
        .filter(|unit| **unit == 0x000D)
        .count()
        + 1;
    let Some(values) = Form::new("Go to Line")
        .field(Field::new(&format!("Line (1-{total}):"), ""))
        .show(app.hwnd)
    else {
        return;
    };
    let Ok(line) = values[0].trim().parse::<usize>() else {
        return;
    };
    let units = editor::text_units(editor, 0, editor::text_length(editor));
    let mut position = 0i32;
    let mut current = 1usize;
    for (index, unit) in units.iter().enumerate() {
        if current >= line {
            break;
        }
        if *unit == 0x000D {
            current += 1;
            position = index as i32 + 1;
        }
    }
    editor::set_selection(editor, position, position);
    editor::scroll_caret(editor);
    // SAFETY: focusing our own editor.
    unsafe {
        SetFocus(editor);
    }
}

/// View > Zoom In / Out (`Some(true)` / `Some(false)`) or reset (`None`,
/// which goes back to the default zoom in Preferences).
pub fn zoom(app: &mut App, larger: Option<bool>) {
    const STEPS: [i32; 13] = [50, 67, 75, 80, 90, 100, 110, 125, 150, 175, 200, 250, 300];
    let current = app
        .active_editor()
        .map(editor::zoom_percent)
        .unwrap_or(app.zoom);
    let next = match larger {
        Some(true) => STEPS
            .iter()
            .copied()
            .find(|step| *step > current)
            .unwrap_or(300),
        Some(false) => STEPS
            .iter()
            .rev()
            .copied()
            .find(|step| *step < current)
            .unwrap_or(50),
        None => app.settings.default_zoom.clamp(25, 500),
    };
    set_zoom(app, next);
}

/// Applies one zoom percentage to every open editor.
pub fn set_zoom(app: &mut App, percent: i32) {
    let next = percent.clamp(25, 500);
    app.zoom = next;
    for tab in &app.tabs {
        editor::set_zoom_percent(tab.editor, next);
    }
    status::set_message(app, &format!("Zoom {next}%"));
    app.save_session_now();
}

/// View > Set Zoom...: type the percentage instead of stepping through it.
pub fn zoom_dialog(app: &mut App) {
    let current = app
        .active_editor()
        .map(editor::zoom_percent)
        .unwrap_or(app.zoom);
    let Some(values) = Form::new("Zoom")
        .note("Choose a zoom level from 25% to 500%. The default level is in Tools > Preferences.")
        .field(Field::new("Zoom (%):", current.to_string()))
        .show(app.hwnd)
    else {
        return;
    };
    match values[0].trim().parse::<i32>() {
        Ok(percent) if (25..=500).contains(&percent) => set_zoom(app, percent),
        _ => status::set_message(app, "Zoom must be a whole number from 25 to 500"),
    }
}

/// Notes > Copy Internal Link: puts a link to the note on the clipboard, as
/// RTF (pastes as a clickable title) and as plain `nodi://note/<id>` text.
pub fn copy_internal_link(app: &mut App) {
    let Some(id) = app.context_note.clone().or_else(|| app.selected.clone()) else {
        return;
    };
    copy_link_for(app, &id);
}

/// Puts a link to `id` on the clipboard (shared by the tree and tab menus).
pub fn copy_link_for(app: &mut App, id: &NoteId) {
    let title = app
        .tree
        .get(id)
        .map(|node| node.title.clone())
        .unwrap_or_default();
    let url = nodi_core::links::format_note_link(id);
    let rtf = link::link_rtf(&title, &url);
    if clipboard::set_text_and_rtf(app.hwnd, &url, &rtf) {
        status::set_message(
            app,
            &format!("Link to \"{title}\" copied — paste it into a note"),
        );
    } else {
        dialogs::error(app.hwnd, "The clipboard is busy; try again.");
    }
}

/// Insert > Formula... (Alt+=).
pub fn insert_formula(app: &mut App) {
    if crate::wnd::editor::refuse_if_locked(app) {
        return;
    }
    crate::wnd::formula::insert_or_edit(app);
}

/// Markdown for the active note: (title, export). `None` without a note.
fn active_markdown(app: &App, asset_dir: &str) -> Option<(String, nodi_core::markdown::Export)> {
    let index = app.active?;
    let tab = app.tabs.get(index)?;
    let rtf = editor::stream_out_rtf(tab.editor)?;
    let text = String::from_utf8_lossy(&rtf);
    Some((
        tab.title.clone(),
        nodi_core::markdown::rtf_to_markdown(&text, asset_dir),
    ))
}

fn lossy_note(lossy: &[String]) -> String {
    if lossy.is_empty() {
        "Everything in this note has a Markdown equivalent.".to_owned()
    } else {
        format!(
            "Markdown cannot keep: {}. The note itself is unchanged.",
            lossy.join(", ")
        )
    }
}

/// Characters Windows does not allow in file names.
pub fn safe_file_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|ch| {
            if r#"<>:"/\|?*"#.contains(ch) || ch.is_control() {
                '_'
            } else {
                ch
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.');
    if trimmed.is_empty() {
        "note".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// View > Markdown Source: a read-only copy of the note as Markdown.
pub fn show_markdown(app: &mut App) {
    let Some((title, export)) = active_markdown(app, "attachments") else {
        status::set_message(app, "Open a note first");
        return;
    };
    let _ = Form::new(&format!("Markdown — {title}"))
        .note(&format!(
            "Read-only preview: edits here are not saved. Use File > Export Note as Markdown to write a .md file.\n{}",
            lossy_note(&export.lossy)
        ))
        .field(Field::multiline("Markdown:", export.markdown, 22))
        .show(app.hwnd);
}

/// File > Export Note as Markdown...: writes `<name>.md` plus a
/// `<name>_files` folder with the pictures. Never overwrites a picture folder.
pub fn export_markdown(app: &mut App) {
    let Some(index) = app.active else {
        status::set_message(app, "Open a note first");
        return;
    };
    let name = safe_file_name(&app.tabs[index].title);
    let Some(path) = dialogs::save_file(app.hwnd, &format!("{name}.md"), "Markdown (*.md)", "md")
    else {
        return;
    };
    export_markdown_to(app, std::path::Path::new(&path));
}

/// Writes the active note as Markdown to `path` (pictures in a
/// `<name>_files` folder next to it) and reports what Markdown cannot keep.
pub fn export_markdown_to(app: &mut App, path: &std::path::Path) {
    let path = path.to_path_buf();
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "note".to_owned());
    let folder = path.parent().unwrap_or(std::path::Path::new("."));
    let mut asset_dir = format!("{stem}_files");
    let mut attempt = 2;
    while folder.join(&asset_dir).exists() {
        asset_dir = format!("{stem}_files-{attempt}");
        attempt += 1;
    }
    let Some((_, export)) = active_markdown(app, &asset_dir) else {
        return;
    };
    let result = (|| -> std::io::Result<()> {
        std::fs::write(&path, export.markdown.as_bytes())?;
        if !export.assets.is_empty() {
            let dir = folder.join(&asset_dir);
            std::fs::create_dir_all(&dir)?;
            for (file, bytes) in &export.assets {
                std::fs::write(dir.join(file), bytes)?;
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => dialogs::info(
            app.hwnd,
            &format!(
                "Exported to\n{}\n\n{}",
                path.display(),
                lossy_note(&export.lossy)
            ),
        ),
        Err(err) => dialogs::error(app.hwnd, &format!("Export failed:\n{err}")),
    }
}

/// Local date and time for Insert > Date and Time (F5).
pub fn local_date_time() -> String {
    let mut now: windows_sys::Win32::Foundation::SYSTEMTIME = unsafe { std::mem::zeroed() };
    // SAFETY: `now` is a valid out parameter.
    unsafe {
        windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut now);
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        now.wYear, now.wMonth, now.wDay, now.wHour, now.wMinute
    )
}

/// Runs a Notes/Search/Tools command that came from the tree context menu.
pub fn context_command(app: &mut App, id: u16) {
    menu::on_command(app, id);
    app.context_note = None;
}

fn format_time(unix_secs: u64) -> String {
    if unix_secs == 0 {
        return "-".to_owned();
    }
    let (y, mo, d, h, mi, s) = civil_utc(unix_secs);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC")
}

fn civil_utc(unix_secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let seconds_of_day = (unix_secs % 86_400) as u32;
    let days = (unix_secs / 86_400) as i64;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as u32, day as u32, hour, minute, second)
}

#[allow(dead_code)]
fn _assert_styles(_: WNDCLASS_STYLES) {}
