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
        };
        self.nodes.insert(id.clone(), node);
        id
    }

    /// Renames a node. Returns false when the node does not exist.
    pub fn rename(&mut self, id: &NoteId, title: impl Into<String>) -> bool {
        let title = title.into();
        match self.nodes.get_mut(id) {
            Some(node) => {
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
