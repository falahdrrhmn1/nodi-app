//! The note tree: nodes, folders, ordering, moves and the trash.
//!
//! Both folders and notes may have children (a note with sub-notes is a
//! first-class concept). The tree is stored as a flat map and serialised as
//! `{ "version": 1, "nodes": [ ... ] }` into `tree.json`.
//!
//! Invariants enforced by the mutation methods:
//! * a node can never become its own ancestor (no cycles),
//! * sibling `order` values are unique and dense inside a parent,
//! * a trashed node is never returned by [`Tree::children`].

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::id::{now_secs, NoteId};

/// On-disk format version for `tree.json`.
pub const TREE_FORMAT_VERSION: u64 = 1;

/// What a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Folder,
    Note,
}

/// One entry of the tree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: NoteId,
    pub kind: NodeKind,
    pub title: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub parent: Option<NoteId>,
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub trashed: bool,
    /// Unix seconds.
    #[serde(default)]
    pub created: u64,
    /// Unix seconds.
    #[serde(default)]
    pub modified: u64,
    /// The title still follows the note's first line (a new note that was
    /// never renamed by hand). See [`auto_title`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_title: bool,
    /// Locked in the Notes Explorer: no editing, renaming, moving or
    /// trashing until unlocked. A locked folder locks everything inside it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// The file this note was imported from (never written to unless the
    /// user chooses Save Back to Original File).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// How the Notes Explorer orders siblings (View > Sort Notes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortOrder {
    /// The order the user arranged (drag & drop, Move Up/Down).
    #[default]
    Manual,
    /// By title, numbers compared as numbers (`2` before `10`, `00-…` first).
    Name,
    Created,
    Modified,
}

/// Compares titles the way people read numbered names: case-insensitive, and
/// runs of digits by value (`file2` < `file10`, `01-a` < `02-b`).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut left = a.chars().peekable();
    let mut right = b.chars().peekable();
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut first = String::new();
                while let Some(ch) = left.peek().copied().filter(char::is_ascii_digit) {
                    first.push(ch);
                    left.next();
                }
                let mut second = String::new();
                while let Some(ch) = right.peek().copied().filter(char::is_ascii_digit) {
                    second.push(ch);
                    right.next();
                }
                let first_value = first.trim_start_matches('0');
                let second_value = second.trim_start_matches('0');
                let order = first_value
                    .len()
                    .cmp(&second_value.len())
                    .then_with(|| first_value.cmp(second_value));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                left.next();
                right.next();
            }
        }
    }
}

/// One Notes Explorer change that Ctrl+Z in the tree can take back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeUndo {
    /// Moved to the trash: these nodes come back.
    Trash { ids: Vec<NoteId>, title: String },
    /// Moved (drag & drop, Move Up/Down/To…): the previous place.
    Move {
        id: NoteId,
        title: String,
        parent: Option<NoteId>,
        index: usize,
    },
    /// Renamed: the previous title (and whether it followed the text).
    Rename {
        id: NoteId,
        title: String,
        auto_title: bool,
        new_title: String,
    },
    /// One command on several selected nodes; undone together, last first.
    Batch {
        changes: Vec<TreeUndo>,
        title: String,
    },
}

impl TreeUndo {
    /// The confirmation question shown before undoing.
    pub fn question(&self, tree: &Tree) -> String {
        match self {
            TreeUndo::Trash { title, ids } => {
                let more = ids.len().saturating_sub(1);
                if more == 0 {
                    format!("Undo: restore \"{title}\" from the Trash?")
                } else {
                    format!("Undo: restore \"{title}\" and {more} sub-note(s) from the Trash?")
                }
            }
            TreeUndo::Move { title, parent, .. } => {
                let place = parent
                    .as_ref()
                    .and_then(|id| tree.get(id))
                    .map(|node| format!("\"{}\"", node.title))
                    .unwrap_or_else(|| "the top level".to_owned());
                format!("Undo: move \"{title}\" back to {place}?")
            }
            TreeUndo::Rename {
                title, new_title, ..
            } => format!("Undo: rename \"{new_title}\" back to \"{title}\"?"),
            TreeUndo::Batch { changes, title } => {
                format!("Undo: {title} ({} items)?", changes.len())
            }
        }
    }
}

/// Longest automatic title, in characters.
pub const AUTO_TITLE_MAX: usize = 60;

/// Title for a note whose title follows its text: the first non-empty
/// paragraph without its heading `#`s or list bullet, whitespace collapsed,
/// at most [`AUTO_TITLE_MAX`] characters; `New Note` when there is no text.
pub fn auto_title(text: &str) -> String {
    for line in text.split(['\r', '\n', '\u{000B}']) {
        let cleaned: String = line
            .chars()
            .filter(|ch| !matches!(*ch, '\u{FFFC}' | '\u{FFF9}' | '\u{FFFB}' | '\u{0007}'))
            .collect();
        let mut rest = cleaned.trim();
        rest = rest.trim_start_matches('#').trim_start();
        for bullet in ['•', '◦', '▪', '-', '*'] {
            if let Some(after) = rest.strip_prefix(bullet) {
                if after.starts_with([' ', '\t']) {
                    rest = after.trim_start();
                }
            }
        }
        let words: Vec<&str> = rest.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        let joined = words.join(" ");
        return joined
            .chars()
            .take(AUTO_TITLE_MAX)
            .collect::<String>()
            .trim_end()
            .to_owned();
    }
    "New Note".to_owned()
}

/// Errors from tree operations that can fail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeError {
    /// The node to operate on does not exist.
    NotFound(NoteId),
    /// The requested parent does not exist.
    ParentNotFound(NoteId),
    /// The move would make the node its own ancestor.
    Cycle,
}

impl fmt::Display for TreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TreeError::NotFound(id) => write!(f, "node not found: {id}"),
            TreeError::ParentNotFound(id) => write!(f, "parent not found: {id}"),
            TreeError::Cycle => f.write_str("a node cannot be moved inside itself"),
        }
    }
}

impl std::error::Error for TreeError {}

/// Where a dragged node lands relative to the node under the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Position {
    /// Just above the target, as its previous sibling.
    Before,
    /// Just below the target, as its next sibling.
    After,
    /// As the last child of the target.
    Inside,
}

/// The whole note tree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    nodes: HashMap<NoteId, Node>,
}

impl Tree {
    /// An empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of nodes, trashed included.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when the tree has no nodes at all.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Looks a node up.
    pub fn get(&self, id: &NoteId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Looks a node up mutably. Prefer the dedicated methods: they keep the
    /// tree consistent.
    pub fn get_mut(&mut self, id: &NoteId) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    /// True when `id` exists.
    pub fn contains(&self, id: &NoteId) -> bool {
        self.nodes.contains_key(id)
    }

    /// Iterates over every node (trashed included) in unspecified order.
    pub fn iter(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    /// Iterates over every node (trashed included) in tree order: parents
    /// before children, siblings by `order`.
    pub fn iter_ordered(&self) -> Vec<&Node> {
        let children = self.child_map();
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut stack: Vec<&Node> = children.get(&None).cloned().unwrap_or_default();
        stack.reverse();
        while let Some(node) = stack.pop() {
            out.push(node);
            if let Some(kids) = children.get(&Some(&node.id)) {
                stack.extend(kids.iter().rev().copied());
            }
        }
        out
    }

    /// Adds a node and returns its id. A `parent` that does not exist is
    /// treated as "root" so a UI bug can never lose a node.
    pub fn new_node(
        &mut self,
        kind: NodeKind,
        title: impl Into<String>,
        parent: Option<NoteId>,
    ) -> NoteId {
        let parent = parent.filter(|p| self.nodes.contains_key(p));
        let order = self.next_order(parent.as_ref());
        let now = now_secs();
        let id = NoteId::generate();
        let node = Node {
            id: id.clone(),
            kind,
            title: title.into(),
            icon: None,
            parent,
            order,
            trashed: false,
            created: now,
            modified: now,
            auto_title: false,
            locked: false,
            source: None,
        };
        self.nodes.insert(id.clone(), node);
        id
    }

    /// Lets the note's title follow its first line (or stop doing so).
    pub fn set_auto_title(&mut self, id: &NoteId, auto: bool) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.auto_title = auto;
        }
    }

    /// For a note with an automatic title: retitles it from `text` and returns
    /// the new title when it changed. Manual titles are never touched.
    pub fn follow_text(&mut self, id: &NoteId, text: &str) -> Option<String> {
        let node = self.nodes.get_mut(id)?;
        if !node.auto_title {
            return None;
        }
        let title = auto_title(text);
        if node.title == title {
            return None;
        }
        node.title = title.clone();
        node.modified = now_secs();
        Some(title)
    }

    /// Renames a node by hand: its title stops following the first line.
    /// Returns false when the node does not exist.
    pub fn rename(&mut self, id: &NoteId, title: impl Into<String>) -> bool {
        let title = title.into();
        match self.nodes.get_mut(id) {
            Some(node) => {
                node.auto_title = false;
                if node.title != title {
                    node.title = title;
                    node.modified = now_secs();
                }
                true
            }
            None => false,
        }
    }

    /// Sets (or clears) the node icon. Returns false when the node is missing.
    pub fn set_icon(&mut self, id: &NoteId, icon: Option<&str>) -> bool {
        match self.nodes.get_mut(id) {
            Some(node) => {
                node.icon = icon.map(str::to_owned);
                true
            }
            None => false,
        }
    }

    /// Moves `id` under `new_parent` at position `index` (clamped) among its
    /// new siblings. `None` means the root.
    pub fn move_node(
        &mut self,
        id: &NoteId,
        new_parent: Option<NoteId>,
        index: usize,
    ) -> Result<(), TreeError> {
        if !self.nodes.contains_key(id) {
            return Err(TreeError::NotFound(id.clone()));
        }
        if let Some(parent) = &new_parent {
            if parent == id {
                return Err(TreeError::Cycle);
            }
            if !self.nodes.contains_key(parent) {
                return Err(TreeError::ParentNotFound(parent.clone()));
            }
            // Walk up: if we meet `id`, the move would create a cycle.
            let mut cursor = Some(parent.clone());
            let mut steps = 0usize;
            while let Some(current) = cursor {
                if &current == id {
                    return Err(TreeError::Cycle);
                }
                steps += 1;
                if steps > self.nodes.len() {
                    return Err(TreeError::Cycle);
                }
                cursor = self
                    .nodes
                    .get(&current)
                    .and_then(|node| node.parent.clone());
            }
        }

        let old_parent = self.nodes.get(id).and_then(|node| node.parent.clone());
        let target_parent = new_parent.clone();

        let mut siblings: Vec<NoteId> = self
            .all_children(new_parent.as_ref())
            .into_iter()
            .map(|node| node.id.clone())
            .filter(|sibling| sibling != id)
            .collect();
        let at = index.min(siblings.len());
        siblings.insert(at, id.clone());

        for (position, sibling) in siblings.iter().enumerate() {
            if let Some(node) = self.nodes.get_mut(sibling) {
                node.order = position as i64;
            }
        }
        if let Some(node) = self.nodes.get_mut(id) {
            node.parent = target_parent.clone();
        }
        if old_parent != target_parent {
            // Keep the sibling order of the parent we left dense as well.
            self.renumber(old_parent.as_ref());
        }
        Ok(())
    }

    /// Moves `id` next to `target` ([`Position::Before`] / [`Position::After`])
    /// or as its last child ([`Position::Inside`]). Dropping a node inside its
    /// own subtree is refused with [`TreeError::Cycle`], and a trashed target
    /// is treated as missing (it is not part of the visible tree).
    ///
    /// The whole sibling strip is renumbered densely, trashed siblings
    /// included, so a moved node never leaves holes or duplicate orders.
    pub fn move_relative(
        &mut self,
        id: &NoteId,
        target: &NoteId,
        position: Position,
    ) -> Result<(), TreeError> {
        if !self.nodes.contains_key(id) {
            return Err(TreeError::NotFound(id.clone()));
        }
        let target_node = match self.nodes.get(target) {
            Some(node) if !node.trashed => node,
            _ => return Err(TreeError::NotFound(target.clone())),
        };
        if position == Position::Inside {
            return self.move_node(id, Some(target.clone()), usize::MAX);
        }
        let parent = target_node.parent.clone();
        // `move_node` counts the index after taking `id` out, so translate the
        // index in the full strip.
        let siblings = self.all_children(parent.as_ref());
        let Some(target_index) = siblings.iter().position(|node| &node.id == target) else {
            return Err(TreeError::NotFound(target.clone()));
        };
        let mut index = match position {
            Position::Before => target_index,
            Position::After => target_index + 1,
            Position::Inside => unreachable!(),
        };
        if siblings
            .iter()
            .position(|node| &node.id == id)
            .is_some_and(|own| own < index)
        {
            index -= 1;
        }
        self.move_node(id, parent, index)
    }

    /// Moves `id` one place up (`delta < 0`) or down among its *visible*
    /// siblings, skipping trashed ones. Returns false at either end.
    pub fn move_among_siblings(&mut self, id: &NoteId, delta: i32) -> bool {
        let Some(parent) = self.nodes.get(id).map(|node| node.parent.clone()) else {
            return false;
        };
        let visible: Vec<NoteId> = self
            .children(parent.as_ref())
            .into_iter()
            .map(|node| node.id.clone())
            .collect();
        let Some(position) = visible.iter().position(|sibling| sibling == id) else {
            return false;
        };
        let target = position as i64 + i64::from(delta.signum());
        if target < 0 || target as usize >= visible.len() {
            return false;
        }
        // Index in the full (trash-including) sibling list, as `move_node`
        // counts it after taking `id` out.
        let neighbour = &visible[target as usize];
        let all: Vec<NoteId> = self
            .all_children(parent.as_ref())
            .into_iter()
            .map(|node| node.id.clone())
            .filter(|sibling| sibling != id)
            .collect();
        let Some(neighbour_index) = all.iter().position(|sibling| sibling == neighbour) else {
            return false;
        };
        let index = if delta < 0 {
            neighbour_index
        } else {
            neighbour_index + 1
        };
        self.move_node(id, parent, index).is_ok()
    }

    fn renumber(&mut self, parent: Option<&NoteId>) {
        let siblings: Vec<NoteId> = self
            .all_children(parent)
            .into_iter()
            .map(|node| node.id.clone())
            .collect();
        for (position, sibling) in siblings.iter().enumerate() {
            if let Some(node) = self.nodes.get_mut(sibling) {
                node.order = position as i64;
            }
        }
    }

    /// Visible children of `parent` (`None` = root), sorted by `order`.
    /// Trashed nodes are excluded.
    pub fn children(&self, parent: Option<&NoteId>) -> Vec<&Node> {
        self.all_children(parent)
            .into_iter()
            .filter(|node| !node.trashed)
            .collect()
    }

    /// Moves `id` and its whole subtree to the trash. Returns false when the
    /// node does not exist.
    pub fn trash(&mut self, id: &NoteId) -> bool {
        if !self.nodes.contains_key(id) {
            return false;
        }
        let mut ids = vec![id.clone()];
        ids.extend(self.descendants(id));
        for node_id in ids {
            if let Some(node) = self.nodes.get_mut(&node_id) {
                node.trashed = true;
            }
        }
        true
    }

    /// Restores `id` and its whole subtree. When the parent is gone or still
    /// trashed the node is restored to the root so it can never be invisible.
    pub fn restore(&mut self, id: &NoteId) -> bool {
        if !self.nodes.contains_key(id) {
            return false;
        }
        let mut ids = vec![id.clone()];
        ids.extend(self.descendants(id));
        for node_id in ids {
            if let Some(node) = self.nodes.get_mut(&node_id) {
                node.trashed = false;
            }
        }

        let parent_is_unusable = match self.nodes.get(id).and_then(|n| n.parent.clone()) {
            None => false,
            Some(parent) => match self.nodes.get(&parent) {
                None => true,
                Some(node) => node.trashed,
            },
        };
        if parent_is_unusable {
            let order = self.next_order(None);
            if let Some(node) = self.nodes.get_mut(id) {
                node.parent = None;
                node.order = order;
            }
        }
        true
    }

    /// Permanently removes a *trashed* subtree and returns the ids that were
    /// removed (the node itself first). Purging a live node is refused: empty
    /// result.
    pub fn purge(&mut self, id: &NoteId) -> Vec<NoteId> {
        match self.nodes.get(id) {
            Some(node) if node.trashed => {}
            _ => return Vec::new(),
        }
        let mut ids = vec![id.clone()];
        ids.extend(self.descendants(id));
        for node_id in &ids {
            self.nodes.remove(node_id);
        }
        ids
    }

    /// Copies a single node (never its children) next to itself. Returns the
    /// new id, or `None` when the source does not exist.
    pub fn duplicate(&mut self, id: &NoteId) -> Option<NoteId> {
        let original = self.nodes.get(id)?.clone();
        let parent = match &original.parent {
            Some(parent) => match self.nodes.get(parent) {
                Some(node) if !node.trashed => Some(parent.clone()),
                _ => None,
            },
            None => None,
        };
        // A copy of a trashed node must be visible, so it goes to the root
        // unless its parent is alive.
        let parent = if original.trashed { None } else { parent };
        let order = self.next_order(parent.as_ref());
        let now = now_secs();
        let new_id = NoteId::generate();
        let copy = Node {
            id: new_id.clone(),
            kind: original.kind,
            title: format!("{} (copy)", original.title),
            icon: original.icon,
            parent,
            order,
            trashed: false,
            created: now,
            modified: now,
            auto_title: false,
            locked: false,
            source: None,
        };
        self.nodes.insert(new_id.clone(), copy);
        Some(new_id)
    }

    /// All descendants of `id` in depth-first tree order (trashed included,
    /// the node itself excluded).
    pub fn descendants(&self, id: &NoteId) -> Vec<NoteId> {
        let mut out = Vec::new();
        if !self.nodes.contains_key(id) {
            return out;
        }
        let children = self.child_map();
        let mut stack: Vec<&NoteId> = children
            .get(&Some(id))
            .map(|kids| kids.iter().rev().map(|node| &node.id).collect())
            .unwrap_or_default();
        while let Some(current) = stack.pop() {
            out.push(current.clone());
            if let Some(kids) = children.get(&Some(current)) {
                stack.extend(kids.iter().rev().map(|node| &node.id));
            }
        }
        out
    }

    /// Ancestors of `id`, root first, excluding `id` itself.
    pub fn ancestors(&self, id: &NoteId) -> Vec<NoteId> {
        let mut out = Vec::new();
        let mut cursor = self.nodes.get(id).and_then(|node| node.parent.clone());
        let mut steps = 0usize;
        while let Some(current) = cursor {
            steps += 1;
            if steps > self.nodes.len() {
                break; // corrupt chain, stop instead of spinning
            }
            out.push(current.clone());
            cursor = self
                .nodes
                .get(&current)
                .and_then(|node| node.parent.clone());
        }
        out.reverse();
        out
    }

    /// Titles from the root down to `id` inclusive. Empty when `id` is unknown.
    pub fn path_titles(&self, id: &NoteId) -> Vec<String> {
        let Some(node) = self.nodes.get(id) else {
            return Vec::new();
        };
        let mut titles: Vec<String> = self
            .ancestors(id)
            .into_iter()
            .filter_map(|ancestor| self.nodes.get(&ancestor).map(|n| n.title.clone()))
            .collect();
        titles.push(node.title.clone());
        titles
    }

    /// Parent and position among the visible siblings of `id`.
    pub fn place(&self, id: &NoteId) -> Option<(Option<NoteId>, usize)> {
        let node = self.nodes.get(id)?;
        let index = self
            .children(node.parent.as_ref())
            .iter()
            .position(|sibling| sibling.id == *id)
            .unwrap_or(usize::MAX);
        Some((node.parent.clone(), index))
    }

    /// Moves `id` (and its subtree) to the trash and returns how to undo it:
    /// only the nodes this call trashed come back, not ones that were already
    /// in the trash.
    pub fn trash_undoable(&mut self, id: &NoteId) -> Option<TreeUndo> {
        let title = self.nodes.get(id)?.title.clone();
        let mut ids = vec![id.clone()];
        ids.extend(self.descendants(id));
        ids.retain(|node| self.nodes.get(node).is_some_and(|n| !n.trashed));
        self.trash(id);
        Some(TreeUndo::Trash { ids, title })
    }

    /// The undo record for moving `id`, taken before the move.
    pub fn move_undo(&self, id: &NoteId) -> Option<TreeUndo> {
        let (parent, index) = self.place(id)?;
        Some(TreeUndo::Move {
            id: id.clone(),
            title: self.nodes.get(id)?.title.clone(),
            parent,
            index,
        })
    }

    /// The undo record for renaming `id`, taken before the rename.
    pub fn rename_undo(&self, id: &NoteId, new_title: &str) -> Option<TreeUndo> {
        let node = self.nodes.get(id)?;
        Some(TreeUndo::Rename {
            id: id.clone(),
            title: node.title.clone(),
            auto_title: node.auto_title,
            new_title: new_title.to_owned(),
        })
    }

    /// Takes a change back. A move goes back to the old parent and position;
    /// when that parent is gone (purged) the node goes to the top level.
    pub fn undo(&mut self, change: &TreeUndo) -> Result<(), TreeError> {
        match change {
            TreeUndo::Trash { ids, .. } => {
                for id in ids {
                    if let Some(node) = self.nodes.get_mut(id) {
                        node.trashed = false;
                    }
                }
                Ok(())
            }
            TreeUndo::Move {
                id, parent, index, ..
            } => {
                let parent = parent.clone().filter(|p| self.nodes.contains_key(p));
                self.move_node(id, parent, *index)
            }
            TreeUndo::Rename {
                id,
                title,
                auto_title,
                ..
            } => {
                if !self.rename(id, title.clone()) {
                    return Err(TreeError::NotFound(id.clone()));
                }
                self.set_auto_title(id, *auto_title);
                Ok(())
            }
            TreeUndo::Batch { changes, .. } => {
                let mut result = Ok(());
                for change in changes.iter().rev() {
                    if let Err(err) = self.undo(change) {
                        result = Err(err);
                    }
                }
                result
            }
        }
    }

    /// Remembers the file a node was imported from.
    pub fn set_source(&mut self, id: &NoteId, source: Option<String>) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.source = source;
        }
    }

    /// Locks or unlocks one node.
    pub fn set_locked(&mut self, id: &NoteId, locked: bool) -> bool {
        match self.nodes.get_mut(id) {
            Some(node) => {
                node.locked = locked;
                true
            }
            None => false,
        }
    }

    /// True when `id` or any of its ancestors is locked.
    pub fn is_locked(&self, id: &NoteId) -> bool {
        let mut current = self.nodes.get(id);
        let mut guard = 0;
        while let Some(node) = current {
            if node.locked {
                return true;
            }
            guard += 1;
            if guard > 4096 {
                return false;
            }
            current = node
                .parent
                .as_ref()
                .and_then(|parent| self.nodes.get(parent));
        }
        false
    }

    /// True when `id` or anything below it is locked (a subtree that must not
    /// be trashed or moved as a whole).
    pub fn subtree_locked(&self, id: &NoteId) -> bool {
        self.is_locked(id)
            || self
                .descendants(id)
                .iter()
                .any(|child| self.nodes.get(child).is_some_and(|node| node.locked))
    }

    /// Visible children of `parent` in the Notes Explorer's display order.
    pub fn sorted_children(
        &self,
        parent: Option<&NoteId>,
        order: SortOrder,
        descending: bool,
        folders_first: bool,
    ) -> Vec<&Node> {
        let mut children = self.children(parent);
        if order == SortOrder::Manual {
            return children;
        }
        children.sort_by(|a, b| {
            let by_key = match order {
                SortOrder::Manual => Ordering::Equal,
                SortOrder::Name => natural_cmp(&a.title, &b.title),
                SortOrder::Created => a.created.cmp(&b.created),
                SortOrder::Modified => a.modified.cmp(&b.modified),
            }
            .then_with(|| a.order.cmp(&b.order));
            let by_key = if descending { by_key.reverse() } else { by_key };
            if folders_first {
                let folder = |node: &Node| u8::from(node.kind != NodeKind::Folder);
                folder(a).cmp(&folder(b)).then(by_key)
            } else {
                by_key
            }
        });
        children
    }

    /// True when `node` is a descendant of `ancestor` (or equal).
    pub fn is_within(&self, node: &NoteId, ancestor: &NoteId) -> bool {
        node == ancestor || self.descendants(ancestor).contains(node)
    }

    /// True when `candidate` is a descendant of `ancestor`.
    pub fn is_descendant(&self, candidate: &NoteId, ancestor: &NoteId) -> bool {
        candidate != ancestor && self.ancestors(candidate).contains(ancestor)
    }

    /// Direct children of every parent, sorted, in one pass. Used by the
    /// tree-order traversals so they stay O(n log n) instead of O(n²).
    fn child_map(&self) -> HashMap<Option<&NoteId>, Vec<&Node>> {
        let mut map: HashMap<Option<&NoteId>, Vec<&Node>> = HashMap::new();
        for node in self.nodes.values() {
            map.entry(node.parent.as_ref()).or_default().push(node);
        }
        for children in map.values_mut() {
            children.sort_by(|a, b| {
                a.order
                    .cmp(&b.order)
                    .then_with(|| a.id.as_str().cmp(b.id.as_str()))
            });
        }
        map
    }

    /// All direct children of `parent`, trashed included, sorted by `order`.
    fn all_children(&self, parent: Option<&NoteId>) -> Vec<&Node> {
        let mut children: Vec<&Node> = self
            .nodes
            .values()
            .filter(|node| node.parent.as_ref() == parent)
            .collect();
        children.sort_by(|a, b| {
            a.order
                .cmp(&b.order)
                .then_with(|| a.id.as_str().cmp(b.id.as_str()))
        });
        children
    }

    fn next_order(&self, parent: Option<&NoteId>) -> i64 {
        self.all_children(parent)
            .iter()
            .map(|node| node.order)
            .max()
            .map_or(0, |max| max.saturating_add(1))
    }

    /// Drops parent links that point at missing nodes and breaks cycles. Used
    /// when loading a file that might have been edited by hand.
    fn sanitize(&mut self) {
        let ids: Vec<NoteId> = self.nodes.keys().cloned().collect();
        let mut fixes: Vec<NoteId> = Vec::new();

        for id in &ids {
            if let Some(parent) = self.nodes.get(id).and_then(|node| node.parent.clone()) {
                if !self.nodes.contains_key(&parent) {
                    fixes.push(id.clone());
                }
            }
        }

        for id in &ids {
            let mut seen: HashSet<NoteId> = HashSet::new();
            let mut cursor = self.nodes.get(id).and_then(|node| node.parent.clone());
            let mut steps = 0usize;
            while let Some(parent) = cursor {
                steps += 1;
                if !seen.insert(parent.clone()) || steps > ids.len() {
                    fixes.push(id.clone());
                    break;
                }
                cursor = self.nodes.get(&parent).and_then(|node| node.parent.clone());
            }
        }

        for id in fixes {
            if let Some(node) = self.nodes.get_mut(&id) {
                node.parent = None;
            }
        }
    }

    /// Serialises to the `tree.json` shape.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Parses the `tree.json` shape, rejecting unknown versions and repairing
    /// broken parent links.
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }
}

/// Serialisation helper: sorted, deterministic node order.
#[derive(Serialize)]
struct TreeFileRef<'a> {
    version: u64,
    nodes: Vec<&'a Node>,
}

#[derive(Deserialize)]
struct TreeFile {
    version: u64,
    #[serde(default)]
    nodes: Vec<Node>,
}

impl Serialize for Tree {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut nodes: Vec<&Node> = self.nodes.values().collect();
        nodes.sort_by(|a, b| {
            let key_a = (
                a.parent.as_ref().map(NoteId::as_str),
                a.order,
                a.id.as_str(),
            );
            let key_b = (
                b.parent.as_ref().map(NoteId::as_str),
                b.order,
                b.id.as_str(),
            );
            key_a.cmp(&key_b)
        });
        TreeFileRef {
            version: TREE_FORMAT_VERSION,
            nodes,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Tree {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let file = TreeFile::deserialize(deserializer)?;
        if file.version != TREE_FORMAT_VERSION {
            return Err(serde::de::Error::custom(format!(
                "unsupported tree.json version {} (expected {})",
                file.version, TREE_FORMAT_VERSION
            )));
        }
        let mut tree = Tree::new();
        for node in file.nodes {
            tree.nodes.insert(node.id.clone(), node);
        }
        tree.sanitize();
        Ok(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A root folder with two notes, plus two more root notes.
    fn sample() -> (Tree, NoteId, NoteId, NoteId, NoteId, NoteId) {
        let mut tree = Tree::new();
        let a = tree.new_node(NodeKind::Folder, "A", None);
        let b = tree.new_node(NodeKind::Note, "B", None);
        let c = tree.new_node(NodeKind::Note, "C", None);
        let a1 = tree.new_node(NodeKind::Note, "A1", Some(a.clone()));
        let a2 = tree.new_node(NodeKind::Note, "A2", Some(a.clone()));
        (tree, a, b, c, a1, a2)
    }

    fn titles(tree: &Tree, parent: Option<&NoteId>) -> Vec<String> {
        tree.children(parent)
            .into_iter()
            .map(|node| node.title.clone())
            .collect()
    }

    /// Every parent's strip is numbered 0..n without holes or duplicates.
    fn assert_dense(tree: &Tree) {
        let mut parents: Vec<Option<NoteId>> = vec![None];
        parents.extend(tree.iter().map(|node| Some(node.id.clone())));
        for parent in parents {
            let strip = tree.all_children(parent.as_ref());
            let orders: Vec<i64> = strip.iter().map(|node| node.order).collect();
            let expected: Vec<i64> = (0..strip.len() as i64).collect();
            assert_eq!(orders, expected, "orders are not dense");
        }
    }

    #[test]
    fn moves_before_and_after_siblings() {
        let (mut tree, a, b, c, _, _) = sample();
        tree.move_relative(&c, &a, Position::Before).unwrap();
        assert_eq!(titles(&tree, None), ["C", "A", "B"]);
        tree.move_relative(&c, &b, Position::After).unwrap();
        assert_eq!(titles(&tree, None), ["A", "B", "C"]);
        tree.move_relative(&c, &a, Position::Before).unwrap();
        assert_eq!(titles(&tree, None), ["C", "A", "B"]);
        assert_dense(&tree);
    }

    #[test]
    fn moves_inside_as_last_child() {
        let (mut tree, a, b, c, _, _) = sample();
        tree.move_relative(&b, &a, Position::Inside).unwrap();
        assert_eq!(tree.get(&b).unwrap().parent, Some(a.clone()));
        assert_eq!(titles(&tree, Some(&a)), ["A1", "A2", "B"]);
        tree.move_relative(&b, &c, Position::Inside).unwrap();
        assert_eq!(titles(&tree, None), ["A", "C"]);
        assert_eq!(titles(&tree, Some(&a)), ["A1", "A2"]);
        assert_eq!(titles(&tree, Some(&c)), ["B"]);
        // A folder can go inside a note too (notes may have sub-notes).
        tree.move_relative(&a, &c, Position::Inside).unwrap();
        assert_eq!(titles(&tree, None), ["C"]);
        assert_eq!(titles(&tree, Some(&c)), ["B", "A"]);
        assert_dense(&tree);
    }

    #[test]
    fn refuses_cycles() {
        let (mut tree, a, _, _, a1, _) = sample();
        assert_eq!(
            tree.move_relative(&a, &a1, Position::Inside),
            Err(TreeError::Cycle)
        );
        assert_eq!(
            tree.move_relative(&a, &a1, Position::Before),
            Err(TreeError::Cycle)
        );
        assert_eq!(
            tree.move_relative(&a, &a, Position::Inside),
            Err(TreeError::Cycle)
        );
        // The tree did not change.
        assert_eq!(titles(&tree, None), ["A", "B", "C"]);
        assert_eq!(titles(&tree, Some(&a)), ["A1", "A2"]);
    }

    #[test]
    fn refuses_trashed_targets() {
        let (mut tree, _, b, c, _, _) = sample();
        tree.trash(&b);
        assert_eq!(
            tree.move_relative(&c, &b, Position::Inside),
            Err(TreeError::NotFound(b.clone()))
        );
        assert_eq!(
            tree.move_relative(&c, &b, Position::After),
            Err(TreeError::NotFound(b.clone()))
        );
        let missing = NoteId::generate();
        assert_eq!(
            tree.move_relative(&c, &missing, Position::Before),
            Err(TreeError::NotFound(missing))
        );
        assert_dense(&tree);
    }

    #[test]
    fn moves_around_trashed_siblings_densely() {
        let (mut tree, a, b, c, _, _) = sample();
        tree.trash(&b);
        tree.move_relative(&c, &a, Position::Before).unwrap();
        assert_eq!(titles(&tree, None), ["C", "A"]);
        // The trashed sibling keeps a slot in the full strip, without holes.
        let strip: Vec<String> = tree
            .all_children(None)
            .into_iter()
            .map(|node| node.title.clone())
            .collect();
        assert_eq!(strip, ["C", "A", "B"]);
        assert!(tree.get(&b).unwrap().trashed);
        assert_dense(&tree);
    }

    #[test]
    fn relative_to_itself_is_a_no_op() {
        let (mut tree, a, b, _, _, _) = sample();
        tree.move_relative(&b, &b, Position::Before).unwrap();
        tree.move_relative(&b, &b, Position::After).unwrap();
        assert_eq!(titles(&tree, None), ["A", "B", "C"]);
        assert_eq!(tree.get(&a).unwrap().parent, None);
        assert_dense(&tree);
    }
}
