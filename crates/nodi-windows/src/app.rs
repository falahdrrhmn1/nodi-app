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
use nodi_core::storage::{Session, TabState, WindowRect, DEFAULT_SIDEBAR_WIDTH};
use nodi_core::{DataDir, NodeKind, NoteId, Tree};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, DEFAULT_CHARSET, FW_BOLD, FW_NORMAL, HFONT, LOGFONTW,
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
use crate::wnd::{clipboard, dialogs, editor, image, link, menu, status, table, tabs, tree};

/// Custom message posted by the writer thread with a boxed [`SaveResult`].
pub const WM_APP_SAVE_RESULT: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 1;

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

    pub dpi: u32,
    pub ui_font: HFONT,
    pub bold_font: HFONT,
    pub hcursor_we: HCURSOR,

    pub toolbar: HWND,
    pub sidebar_title: HWND,
    pub tree_view: HWND,
    pub tabctrl: HWND,
    pub status: HWND,

    pub toolbar_visible: bool,
    pub status_visible: bool,
    pub sidebar_width: i32,
    pub splitter_drag: bool,
    pub restored_window: bool,

    pub toolbar_height: i32,
    /// New notes whose first (empty) file write is still queued.
    pub pending_new: HashSet<NoteId>,
    /// View > Word Wrap, applied to every editor.
    pub word_wrap: bool,
    /// Zoom percentage applied to every editor.
    pub zoom: i32,
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
            dpi: 96,
            ui_font: ptr::null_mut(),
            bold_font: ptr::null_mut(),
            // SAFETY: `IDC_SIZEWE` is a predefined system cursor; the call
            // takes no pointers we own and returns a shared handle.
            hcursor_we: unsafe { LoadCursorW(ptr::null_mut(), IDC_SIZEWE) },
            toolbar: ptr::null_mut(),
            sidebar_title: ptr::null_mut(),
            tree_view: ptr::null_mut(),
            tabctrl: ptr::null_mut(),
            status: ptr::null_mut(),
            toolbar_visible: true,
            status_visible: true,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            splitter_drag: false,
            restored_window: false,
            toolbar_height: 0,
            pending_new: HashSet::new(),
            word_wrap: true,
            zoom: 100,
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
            Ok(paths) if !paths.is_empty() => dialogs::recovered(self.hwnd, &paths),
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

        let session = data.load_session().unwrap_or_default();
        if session.sidebar_width > 0 {
            self.sidebar_width = session.sidebar_width;
        }
        self.expanded = session.expanded.iter().cloned().collect();
        self.word_wrap = session.word_wrap;
        self.zoom = session.zoom.clamp(25, 500);
        menu::set_checked(self, menu::IDM_VIEW_WORD_WRAP, self.word_wrap);
        if let Some(rect) = session.window {
            self.apply_window_rect(rect);
        }

        if self.tree.is_empty() {
            let id = self.create_note_node(NodeKind::Note, "Untitled", None);
            self.tree_save();
            self.selected = Some(id.clone());
            self.expand(Some(id.clone()));
            self.open_note(id);
            self.save_session_now();
        } else {
            self.restore_session_tabs(&session);
        }

        self.rebuild_tree();
        self.maybe_startup_backup();
        true
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

    // ------------------------------------------------------------------- tabs

    /// Opens (or re-uses) the tab for a note.
    pub fn open_note(&mut self, id: NoteId) {
        self.open_note_with(id, None, None);
        self.save_session_now();
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
            Some(bytes) => editor::load_rtf(editor, bytes),
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
                        loaded = editor::load_rtf(editor, &previous_bytes);
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

        editor::set_modified(editor, false);
        if let Some((from, to)) = selection {
            editor::set_selection(editor, from, to);
        }
        if let Some(line) = scroll {
            editor::scroll_to_line(editor, line);
        }
        editor::set_change_events(editor, true);

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
        let Some(rtf) = editor::stream_out_rtf(editor) else {
            self.tabs[index].last_error = Some("the editor content could not be read".to_owned());
            self.refresh_tab_ui(index);
            return;
        };
        let txt = editor::plain_text(editor);
        let seq = self.tabs[index].seq + 1;
        let note = self.tabs[index].note.clone();
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
        }
    }

    /// Timer callback: either a per-tab debounce or the 5 s retry.
    pub fn on_timer(&mut self, id: usize) {
        if id == SESSION_TIMER {
            self.write_session();
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
        let parent = match (&anchor, as_child) {
            (Some(id), true) => Some(id.clone()),
            (Some(id), false) => self.tree.get(id).and_then(|node| node.parent.clone()),
            (None, _) => None,
        };
        let title = match kind {
            NodeKind::Note => "New Note",
            NodeKind::Folder => "New Folder",
        };
        let id = self.create_note_node(kind, title, parent.clone());
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
        let ids = self.subtree_ids(&id);
        for index in (0..self.tabs.len()).rev() {
            if ids.contains(&self.tabs[index].note) {
                self.close_tab(index);
            }
        }
        if self.tree.trash(&id) {
            self.tree_save();
            self.rebuild_tree();
            status::set_message(self, "Moved to Trash");
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
        if title.is_empty() {
            return;
        }
        if !self.tree.rename(&id, title.clone()) {
            return;
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

    pub fn move_node(&mut self, source: NoteId, target: NoteId) {
        if source == target {
            return;
        }
        match self
            .tree
            .move_node(&source, Some(target.clone()), usize::MAX)
        {
            Ok(()) => {
                self.tree_save();
                self.expand(Some(target));
                self.rebuild_tree();
                status::set_message(self, "Moved");
            }
            Err(err) => {
                status::set_message(self, &format!("Move rejected: {err}"));
            }
        }
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
    let ui = unsafe { make_ui_font(app.dpi, false) };
    let bold = unsafe { make_ui_font(app.dpi, true) };
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
        for control in [app.tree_view, app.tabctrl, app.status, app.toolbar] {
            if !control.is_null() {
                SendMessageW(control, WM_SETFONT, app.ui_font as usize, 1);
            }
        }
        if !app.sidebar_title.is_null() && !app.bold_font.is_null() {
            SendMessageW(app.sidebar_title, WM_SETFONT, app.bold_font as usize, 1);
        }
    }
}

unsafe fn make_ui_font(dpi: u32, bold: bool) -> HFONT {
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
    status::update_caret(app);
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
    let current = app
        .tree
        .get(&id)
        .and_then(|node| node.icon.clone())
        .unwrap_or_default();
    if let Some(key) = dialogs::pick_icon(app.hwnd, &current) {
        app.set_icon(id, key);
    }
}

/// Opens the selected note (context menu "Open").
pub fn open_selected(app: &mut App) {
    if let Some(id) = app.context_note.clone().or_else(|| app.selected.clone()) {
        app.open_note(id);
    }
}

impl App {
    /// Notes > Move Up / Move Down (Alt+Up / Alt+Down).
    pub fn move_selected(&mut self, delta: i32) {
        let Some(id) = self.context_note.clone().or_else(|| self.selected.clone()) else {
            return;
        };
        if self.tree.move_among_siblings(&id, delta) {
            self.tree_save();
            self.selected = Some(id);
            self.rebuild_tree();
        }
    }

    /// Notes > Move to Top Level.
    pub fn move_selected_to_root(&mut self) {
        let Some(id) = self.context_note.clone().or_else(|| self.selected.clone()) else {
            return;
        };
        if self.tree.move_node(&id, None, usize::MAX).is_ok() {
            self.tree_save();
            self.selected = Some(id);
            self.rebuild_tree();
            status::set_message(self, "Moved to the top level");
        }
    }
}

/// Insert > Image...: file picker, then the picture goes in at the caret.
pub fn insert_image_file(app: &mut App) {
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

/// View > Zoom In / Out (`Some(true)` / `Some(false)`) or reset (`None`).
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
        None => 100,
    };
    app.zoom = next;
    for tab in &app.tabs {
        editor::set_zoom_percent(tab.editor, next);
    }
    status::set_message(app, &format!("Zoom {next}%"));
    app.save_session_now();
}

/// Notes > Copy Internal Link: puts a link to the note on the clipboard, as
/// RTF (pastes as a clickable title) and as plain `nodi://note/<id>` text.
pub fn copy_internal_link(app: &mut App) {
    let Some(id) = app.context_note.clone().or_else(|| app.selected.clone()) else {
        return;
    };
    let title = app
        .tree
        .get(&id)
        .map(|node| node.title.clone())
        .unwrap_or_default();
    let url = nodi_core::links::format_note_link(&id);
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
fn safe_file_name(title: &str) -> String {
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
    let path = std::path::PathBuf::from(path);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or(name);
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
