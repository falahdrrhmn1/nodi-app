//! Notes Explorer commands on several nodes at once (Ctrl+click / Shift+click
//! selection), locking, and the sort order (View > Sort Notes).

use nodi_core::tree::{Position, SortOrder};
use nodi_core::{NoteId, TreeUndo};

use crate::app::App;
use crate::wnd::{dialogs, editor, menu, status, tree};

impl App {
    /// The nodes a Notes Explorer command acts on: the multi-selection when
    /// there is one, otherwise the right-clicked / selected node. A node whose
    /// ancestor is also selected is left out (it moves with that ancestor).
    pub fn selection_ids(&self) -> Vec<NoteId> {
        let live = |id: &NoteId| self.tree.get(id).is_some_and(|node| !node.trashed);
        let ids: Vec<NoteId> = if self.multi.len() > 1 {
            self.multi.iter().filter(|id| live(id)).cloned().collect()
        } else {
            self.context_note
                .clone()
                .or_else(|| self.selected.clone())
                .into_iter()
                .filter(|id| live(id))
                .collect()
        };
        ids.iter()
            .filter(|id| {
                !ids.iter()
                    .any(|other| other != *id && self.tree.is_within(id, other))
            })
            .cloned()
            .collect()
    }

    /// Drops the multi-selection (a plain click, Esc, or after a command).
    pub fn clear_multi(&mut self) {
        if !self.multi.is_empty() {
            self.multi.clear();
            tree::redraw(self);
        }
    }

    fn report_multi(&mut self) {
        let count = self.multi.len();
        if count > 1 {
            let message =
                format!("{count} items selected — Delete, right-click or drag acts on all of them");
            status::set_message(self, &message);
        }
    }

    /// Ctrl+click: adds `id` to the selection, or takes it out again.
    pub fn toggle_multi(&mut self, id: NoteId) {
        if self.multi.is_empty() {
            if let Some(selected) = self.selected.clone() {
                self.multi.push(selected);
            }
        }
        match self.multi.iter().position(|item| *item == id) {
            Some(index) => {
                self.multi.remove(index);
            }
            None => self.multi.push(id.clone()),
        }
        self.multi_anchor = Some(id);
        tree::redraw(self);
        self.report_multi();
    }

    /// Shift+click: every visible row between the anchor and `id`.
    pub fn range_multi(&mut self, id: NoteId) {
        let anchor = self
            .multi_anchor
            .clone()
            .or_else(|| self.selected.clone())
            .unwrap_or_else(|| id.clone());
        let visible = tree::visible_ids(self);
        let first = visible.iter().position(|item| *item == anchor);
        let last = visible.iter().position(|item| *item == id);
        self.multi = match (first, last) {
            (Some(a), Some(b)) => visible[a.min(b)..=a.max(b)].to_vec(),
            _ => vec![id],
        };
        self.multi_anchor = Some(anchor);
        tree::redraw(self);
        self.report_multi();
    }

    /// True when `id` may be changed; otherwise says why in the status bar.
    pub fn check_unlocked(&mut self, id: &NoteId) -> bool {
        if !self.tree.is_locked(id) {
            return true;
        }
        let title = self
            .tree
            .get(id)
            .map(|node| node.title.clone())
            .unwrap_or_default();
        let message = format!("\"{title}\" is locked — right-click > Unlock to change it");
        status::set_message(self, &message);
        false
    }

    /// Delete / Notes > Move to Trash on the selection.
    pub fn trash_selection(&mut self) {
        let ids = self.selection_ids();
        if ids.len() <= 1 {
            if let Some(id) = ids.into_iter().next() {
                if !self.tree.subtree_locked(&id) {
                    self.trash_node(id);
                } else {
                    self.check_unlocked(&id);
                    status::set_message(self, "Locked notes cannot be trashed — unlock them first");
                }
            }
            return;
        }
        let (locked, ids): (Vec<NoteId>, Vec<NoteId>) =
            ids.into_iter().partition(|id| self.tree.subtree_locked(id));
        if ids.is_empty() {
            status::set_message(self, "Locked notes cannot be trashed — unlock them first");
            return;
        }
        let skipped = if locked.is_empty() {
            String::new()
        } else {
            format!("\n\n{} locked item(s) stay where they are.", locked.len())
        };
        let question = format!(
            "Move {} items (and their sub-notes) to the Trash?{skipped}",
            ids.len()
        );
        if !dialogs::ask_yes_no(self.hwnd, &question) {
            return;
        }
        let mut gone: Vec<NoteId> = Vec::new();
        for id in &ids {
            gone.push(id.clone());
            gone.extend(self.tree.descendants(id));
        }
        for index in (0..self.tabs.len()).rev() {
            if gone.contains(&self.tabs[index].note) {
                self.close_tab(index);
            }
        }
        let changes: Vec<TreeUndo> = ids
            .iter()
            .filter_map(|id| self.tree.trash_undoable(id))
            .collect();
        let count = changes.len();
        self.remember_tree_change(TreeUndo::Batch {
            changes,
            title: format!("restore {count} items from the Trash"),
        });
        self.multi.clear();
        self.tree_save();
        self.rebuild_tree();
        let message = format!("Moved {count} items to the Trash (Ctrl+Z in the tree undoes it)");
        status::set_message(self, &message);
        self.save_session_now();
    }

    /// Why `ids` cannot go to `parent` (`None` = top level), if they cannot.
    fn move_refusal(&self, ids: &[NoteId], parent: Option<&NoteId>) -> Option<String> {
        if let Some(id) = ids.iter().find(|id| self.tree.subtree_locked(id)) {
            let title = self.tree.get(id).map(|node| node.title.clone());
            return Some(format!(
                "\"{}\" is locked — unlock it to move it",
                title.unwrap_or_default()
            ));
        }
        if let Some(parent) = parent.filter(|parent| self.tree.is_locked(parent)) {
            let title = self.tree.get(parent).map(|node| node.title.clone());
            return Some(format!(
                "\"{}\" is locked — nothing can be moved into it",
                title.unwrap_or_default()
            ));
        }
        None
    }

    /// Drag & drop of the selection: before / after / inside `target`.
    pub fn move_selection_relative(
        &mut self,
        source: NoteId,
        target: NoteId,
        position: Position,
    ) -> Result<(), String> {
        let mut ids = if self.multi.len() > 1 && self.multi.contains(&source) {
            self.selection_ids()
        } else {
            vec![source]
        };
        ids.retain(|id| !self.tree.is_within(&target, id));
        let parent = match position {
            Position::Inside => Some(target.clone()),
            Position::Before | Position::After => {
                self.tree.get(&target).and_then(|node| node.parent.clone())
            }
        };
        if let Some(reason) = self.move_refusal(&ids, parent.as_ref()) {
            status::set_message(self, &reason);
            return Err(reason);
        }
        if ids.len() <= 1 {
            return match ids.into_iter().next() {
                Some(id) => self.move_node_relative(id, target, position),
                None => Ok(()),
            };
        }
        // Before: a, b, c each go before the target (a b c target); after:
        // the last first, so they end up target a b c.
        let order: Vec<NoteId> = match position {
            Position::After => ids.iter().rev().cloned().collect(),
            _ => ids.clone(),
        };
        let mut changes = Vec::new();
        for id in &order {
            let change = self.tree.move_undo(id);
            if self.tree.move_relative(id, &target, position).is_ok() {
                changes.extend(change);
            }
        }
        let count = changes.len();
        self.remember_tree_change(TreeUndo::Batch {
            changes,
            title: format!("move {count} items back"),
        });
        self.tree_save();
        if position == Position::Inside {
            self.expand(Some(target));
        }
        self.rebuild_tree();
        status::set_message(self, &format!("Moved {count} items"));
        Ok(())
    }

    /// Notes > Move to... / Move to Top Level for the whole selection.
    pub fn move_selection_into(&mut self, parent: Option<NoteId>) {
        let mut ids = self.selection_ids();
        if let Some(parent) = &parent {
            ids.retain(|id| !self.tree.is_within(parent, id));
        }
        if let Some(reason) = self.move_refusal(&ids, parent.as_ref()) {
            status::set_message(self, &reason);
            return;
        }
        let mut changes = Vec::new();
        for id in &ids {
            let change = self.tree.move_undo(id);
            if self.tree.move_node(id, parent.clone(), usize::MAX).is_ok() {
                changes.extend(change);
            }
        }
        if changes.is_empty() {
            return;
        }
        let count = changes.len();
        let change = if count == 1 {
            changes.remove(0)
        } else {
            TreeUndo::Batch {
                changes,
                title: format!("move {count} items back"),
            }
        };
        self.remember_tree_change(change);
        self.tree_save();
        if parent.is_some() {
            self.expand(parent.clone());
        }
        if count == 1 {
            self.selected = ids.into_iter().next();
        }
        self.rebuild_tree();
        let place = match &parent {
            Some(_) => "Moved",
            None => "Moved to the top level",
        };
        status::set_message(self, &format!("{place} ({count} item(s))"));
    }

    /// Right-click > Lock / Unlock on the selection.
    pub fn set_selection_locked(&mut self, locked: bool) {
        let ids = self.selection_ids();
        if ids.is_empty() {
            return;
        }
        // Edits typed so far are saved before the editor turns read-only.
        self.flush_all();
        for id in &ids {
            self.tree.set_locked(id, locked);
        }
        self.tree_save();
        self.rebuild_tree();
        self.apply_locks();
        let message = match (locked, ids.len()) {
            (true, 1) => {
                "Locked — it cannot be edited, renamed, moved or trashed until unlocked".to_owned()
            }
            (true, count) => format!("Locked {count} items"),
            (false, 1) => "Unlocked".to_owned(),
            (false, count) => format!("Unlocked {count} items"),
        };
        status::set_message(self, &message);
    }

    /// Makes every open editor read-only exactly when its note is locked.
    pub fn apply_locks(&mut self) {
        for index in 0..self.tabs.len() {
            let locked = self.tree.is_locked(&self.tabs[index].note);
            editor::set_read_only(self.tabs[index].editor, locked);
            self.refresh_tab_ui(index);
        }
    }

    /// True when every selected node is locked (the context menu then offers
    /// Unlock instead of Lock).
    pub fn selection_locked(&self) -> bool {
        let ids = self.selection_ids();
        !ids.is_empty()
            && ids
                .iter()
                .all(|id| self.tree.get(id).is_some_and(|node| node.locked))
    }

    /// View > Sort Notes > Manual / Name / Created / Modified.
    pub fn set_sort(&mut self, order: SortOrder) {
        if self.settings.tree_sort == order {
            // Choosing the active order again flips the direction.
            self.settings.tree_sort_desc =
                !self.settings.tree_sort_desc && order != SortOrder::Manual;
        } else {
            self.settings.tree_sort = order;
            self.settings.tree_sort_desc = false;
        }
        self.save_settings();
        self.rebuild_tree();
        menu::sync_settings(self);
        let name = match order {
            SortOrder::Manual => "your own order",
            SortOrder::Name => "name",
            SortOrder::Created => "date created",
            SortOrder::Modified => "date modified",
        };
        let direction = if self.settings.tree_sort_desc {
            " (descending)"
        } else {
            ""
        };
        status::set_message(self, &format!("Notes sorted by {name}{direction}"));
    }

    /// View > Sort Notes > Folders First.
    pub fn toggle_folders_first(&mut self) {
        self.settings.folders_first = !self.settings.folders_first;
        self.save_settings();
        self.rebuild_tree();
        menu::sync_settings(self);
    }
}

impl App {
    /// The file a note was imported from, when it still has one recorded.
    ///
    /// `tree.json` can come from elsewhere (a restored backup), so the value
    /// is only trusted as a note file on a local drive: a network path would
    /// send the Windows login to that host, and Save Back must never write
    /// anything but a `.md` / `.markdown` / `.rtf` / `.txt` file.
    pub fn import_source(&self, id: &NoteId) -> Option<String> {
        let source = self.tree.get(id).and_then(|node| node.source.clone())?;
        let path = nodi_core::links::local_path(&source)?;
        // A comma or quote would become another Explorer argument
        // (`/select,a,b`); wildcards, a second colon (streams) or `..` make
        // the path mean something else than it shows.
        let odd = path
            .chars()
            .any(|ch| matches!(ch, ',' | '"' | '*' | '?' | '<' | '>' | '|'))
            || path[2..].contains(':')
            || path[2..].contains("\\\\")
            || path[2..].contains("//")
            || path.split(['\\', '/']).any(|part| part == "..");
        if odd {
            return None;
        }
        // Where it really points (junctions, `8.3` names) must still be a
        // local drive; that real path is what Explorer and Save Back get.
        let path = match std::fs::canonicalize(&path) {
            Ok(real) => {
                let real = real.to_string_lossy().into_owned();
                let plain = real.strip_prefix("\\\\?\\")?.to_owned();
                let bytes = plain.as_bytes();
                let local = bytes.len() > 2
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                    && bytes[2] == b'\\';
                if !local {
                    return None;
                }
                plain
            }
            // A file that is gone keeps its recorded (checked) path.
            Err(_) => path,
        };
        let extension = std::path::Path::new(&path)
            .extension()?
            .to_str()?
            .to_ascii_lowercase();
        matches!(extension.as_str(), "md" | "markdown" | "rtf" | "txt").then_some(path)
    }

    /// View > Refresh Notes (F5 in the tree): reads `tree.json` and the
    /// settings again — after files were copied in by hand or another tool
    /// changed them. Open notes stay as they are (they are saved first).
    pub fn refresh_from_disk(&mut self) {
        let Some(data) = self.data.clone() else {
            return;
        };
        self.flush_all();
        if let Some(writer) = &self.writer {
            writer.flush_blocking(std::time::Duration::from_secs(5));
        }
        match data.load_tree() {
            Ok(tree) => {
                self.tree = tree;
                self.multi.clear();
                self.rebuild_tree();
                self.apply_locks();
                status::set_message(self, "Notes Explorer refreshed from disk");
            }
            Err(err) => {
                status::set_message(self, &format!("Refresh failed: {err}"));
            }
        }
    }

    /// Tools > Restore From Backup Folder...: a folder made by File > Export >
    /// Complete Backup Copy (or any snapshot folder).
    pub fn restore_from_folder_ui(&mut self) {
        let Some(folder) = dialogs::open_folder(self.hwnd) else {
            return;
        };
        let folder = std::path::PathBuf::from(folder);
        if !nodi_core::backup::is_backup(&folder) {
            dialogs::info(
                self.hwnd,
                "That folder is not a Nodi backup (it has no tree.json).\n\n\
                 Choose a folder made by File > Export > Complete Backup Copy.",
            );
            return;
        }
        let question = format!(
            "Replace the current notes with the backup in\n{}?\n\n\
             Nodi snapshots the current state first, so this can be undone \
             (Tools > Restore Backup).",
            folder.display()
        );
        if dialogs::ask_yes_no(self.hwnd, &question) {
            self.restore_snapshot(&folder);
        }
    }
}
