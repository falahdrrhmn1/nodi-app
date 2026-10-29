//! Always-current Markdown mirror for AI agents.
//!
//! Nodi keeps RTF as the source of truth. After every successful save the
//! background writer also regenerates `data/markdown/`:
//!
//! ```text
//! data/markdown/
//!   INDEX.md                    outline with relative links
//!   index.json                  machine-readable index
//!   Folder/Note.md              one file per note, front matter + Markdown
//!   Folder/Note_files/*.png     that note's pictures
//!   .trash/<timestamp>/...      mirror files whose note was renamed/trashed
//! ```
//!
//! The mirror is derived data: it is written, never read back, and nothing in
//! it is ever deleted. Renames, moves and trash move the stale file into
//! `.trash/` instead, so a bad rename cannot lose the only readable copy.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::id::now_secs;
use crate::markdown;
use crate::storage::DataDir;
use crate::tree::{Node, NodeKind, Tree};
use crate::NoteId;

/// Where one note lives inside the mirror.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Planned {
    /// Safe folder names below `data/markdown/`.
    pub dirs: Vec<String>,
    /// Safe file name (`<title>.md`).
    pub file: String,
    /// `dirs/file`, with `/` separators (for links and `index.json`).
    pub relative: String,
}

/// Result of a mirror rebuild.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub notes: usize,
    pub moved_to_trash: usize,
    pub index: PathBuf,
    pub json: PathBuf,
}

/// Regenerates every mirror file for `tree` (reads the note RTF from disk).
pub fn rebuild(data: &DataDir, tree: &Tree) -> io::Result<Report> {
    let ordered = plan_ordered(tree);
    let mut report = Report::default();
    for (id, planned) in &ordered {
        let rtf = data.load_note(id);
        write_note_file(&data.markdown_dir(), tree, id, planned, rtf.as_deref())?;
        report.notes += 1;
    }
    report.index = write_index(data, tree, &ordered)?;
    report.json = write_index_json(data, tree, &ordered)?;
    report.moved_to_trash = move_stale_to_trash(data, &ordered)?;
    Ok(report)
}

/// Exports the note or folder `root` and everything below it to `dest` as
/// Markdown files (front matter, pictures in `<title>_files`), in the same
/// layout as the mirror. Nothing else in `dest` is touched; existing files
/// with the same names are overwritten. Returns the number of notes written.
/// File formats written by [`export_many`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Formats {
    /// `<title>.md` with a `<title>_files` folder for pictures.
    pub markdown: bool,
    /// `<title>.rtf`: the note exactly as Nodi stores it (opens in Word).
    pub rtf: bool,
    /// `<title>.txt`: plain text, UTF-8.
    pub txt: bool,
}

/// Exports notes into `dest`, one file per note per format, with folders
/// mirroring the Notes Explorer. `roots` are the selected nodes (each with
/// everything below it); `None` exports every note. Returns the number of
/// notes written.
pub fn export_many(
    data: &DataDir,
    tree: &Tree,
    roots: Option<&[NoteId]>,
    dest: &Path,
    formats: Formats,
) -> io::Result<usize> {
    let mut sub = tree.clone();
    if let Some(roots) = roots {
        for (index, root) in roots.iter().enumerate() {
            sub.move_node(root, None, index)
                .map_err(|err| io::Error::new(io::ErrorKind::NotFound, format!("{err:?}")))?;
        }
        let others: Vec<NoteId> = sub
            .children(None)
            .iter()
            .filter(|node| !roots.contains(&node.id))
            .map(|node| node.id.clone())
            .collect();
        for id in others {
            sub.trash(&id);
        }
    }
    std::fs::create_dir_all(dest)?;
    let mut written = 0;
    for (id, planned) in plan_ordered(&sub) {
        let rtf = data.load_note(&id);
        if formats.markdown {
            write_note_file(dest, &sub, &id, &planned, rtf.as_deref())?;
        }
        let mut path = dest.to_path_buf();
        for dir in &planned.dirs {
            path.push(dir);
        }
        std::fs::create_dir_all(&path)?;
        let stem = planned.file.trim_end_matches(".md");
        if formats.rtf {
            std::fs::write(
                path.join(format!("{stem}.rtf")),
                rtf.as_deref().unwrap_or(b"{\\rtf1\\ansi }"),
            )?;
        }
        if formats.txt {
            let text = data.load_note_text(&id).unwrap_or_default();
            let mut bytes = b"\xEF\xBB\xBF".to_vec();
            bytes.extend_from_slice(
                text.replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .replace('\n', "\r\n")
                    .as_bytes(),
            );
            std::fs::write(path.join(format!("{stem}.txt")), bytes)?;
        }
        written += 1;
    }
    Ok(written)
}

pub fn export(data: &DataDir, tree: &Tree, root: &NoteId, dest: &Path) -> io::Result<usize> {
    let mut sub = tree.clone();
    sub.move_node(root, None, 0)
        .map_err(|err| io::Error::new(io::ErrorKind::NotFound, format!("{err:?}")))?;
    let others: Vec<NoteId> = sub
        .children(None)
        .iter()
        .filter(|node| node.id != *root)
        .map(|node| node.id.clone())
        .collect();
    for id in others {
        sub.trash(&id);
    }
    std::fs::create_dir_all(dest)?;
    let mut written = 0;
    for (id, planned) in plan_ordered(&sub) {
        let rtf = data.load_note(&id);
        write_note_file(dest, &sub, &id, &planned, rtf.as_deref())?;
        written += 1;
    }
    Ok(written)
}

/// Where each visible note's mirror file was written by the last sync.
pub type Layout = HashMap<NoteId, Planned>;

/// Brings the mirror in line with `tree` after a tree change (new note,
/// rename, move, trash) without converting every note again: only notes whose
/// planned path differs from `previous`, or whose file is missing, are
/// regenerated from their RTF. With `trust_existing` (startup), a mirror file
/// that is newer than its note's RTF is kept as it is too. Stale files go to
/// `.trash/` as in [`rebuild`]. Returns the new layout.
pub fn sync(
    data: &DataDir,
    tree: &Tree,
    previous: &Layout,
    trust_existing: bool,
) -> io::Result<Layout> {
    let ordered = plan_ordered(tree);
    let root = data.markdown_dir();
    for (id, planned) in &ordered {
        let file = planned_path(&root, planned);
        let up_to_date = file.is_file()
            && (previous.get(id) == Some(planned)
                || (trust_existing && newer_than(&file, &data.note_path(id))));
        if !up_to_date {
            let rtf = data.load_note(id);
            write_note_file(&data.markdown_dir(), tree, id, planned, rtf.as_deref())?;
        }
    }
    write_index(data, tree, &ordered)?;
    write_index_json(data, tree, &ordered)?;
    move_stale_to_trash(data, &ordered)?;
    Ok(ordered.into_iter().collect())
}

/// The layout [`rebuild`] leaves behind for `tree`.
pub fn layout(tree: &Tree) -> Layout {
    plan_ordered(tree).into_iter().collect()
}

fn planned_path(root: &Path, planned: &Planned) -> PathBuf {
    let mut path = root.to_path_buf();
    for dir in &planned.dirs {
        path.push(dir);
    }
    path.join(&planned.file)
}

/// `true` when `file` was modified at or after `source` (or `source` is gone).
fn newer_than(file: &Path, source: &Path) -> bool {
    let modified = |path: &Path| std::fs::metadata(path).and_then(|meta| meta.modified());
    match (modified(file), modified(source)) {
        (Ok(file), Ok(source)) => file >= source,
        (Ok(_), Err(_)) => true,
        _ => false,
    }
}

/// Writes the mirror of one note from the RTF that was just saved, then
/// refreshes the two index files. Does nothing when the note is not part of
/// the visible tree (e.g. it was trashed before the save landed).
pub fn write_one(data: &DataDir, tree: &Tree, id: &NoteId, rtf: &[u8]) -> io::Result<()> {
    write_many(data, tree, &[(id, rtf)])
}

/// Writes the mirror of multiple notes from the RTFs that were just saved, then
/// refreshes the two index files once for the batch.
pub fn write_many(data: &DataDir, tree: &Tree, notes: &[(&NoteId, &[u8])]) -> io::Result<()> {
    if notes.is_empty() {
        return Ok(());
    }
    let ordered = plan_ordered(tree);
    for (id, rtf) in notes {
        if let Some((_, planned)) = ordered.iter().find(|(note, _)| note == *id) {
            write_note_file(&data.markdown_dir(), tree, id, planned, Some(rtf))?;
        }
    }
    write_index(data, tree, &ordered)?;
    write_index_json(data, tree, &ordered)?;
    Ok(())
}

/// Moves every file and picture folder in the mirror that no longer belongs to
/// a visible note into `data/markdown/.trash/<timestamp>/`. Never deletes.
fn move_stale_to_trash(data: &DataDir, ordered: &[(NoteId, Planned)]) -> io::Result<usize> {
    let root = data.markdown_dir();
    if !root.is_dir() {
        return Ok(0);
    }
    let mut expected_files: HashSet<String> = HashSet::new();
    let mut expected_dirs: HashSet<String> = HashSet::new();
    for (_, planned) in ordered {
        let mut path = root.clone();
        for dir in &planned.dirs {
            path.push(dir);
        }
        expected_files.insert(key(&path.join(&planned.file)));
        let stem = planned.file.trim_end_matches(".md");
        expected_dirs.insert(key(&path.join(format!("{stem}_files"))));
    }

    let mut stale: Vec<PathBuf> = Vec::new();
    collect_stale(&root, &expected_files, &expected_dirs, &mut stale)?;
    if stale.is_empty() {
        return Ok(0);
    }

    let trash = unique_trash_dir(data);
    let mut moved = 0usize;
    for path in stale {
        let relative = path.strip_prefix(&root).unwrap_or(&path);
        let target = trash.join(relative);
        if let Some(parent) = target.parent() {
            fs_create(parent)?;
        }
        if std::fs::rename(&path, &target).is_ok() {
            moved += 1;
        }
    }
    Ok(moved)
}

fn collect_stale(
    dir: &Path,
    expected_files: &HashSet<String>,
    expected_dirs: &HashSet<String>,
    out: &mut Vec<PathBuf>,
) -> io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if file_type.is_dir() {
            if name == ".trash" {
                continue;
            }
            if name.ends_with("_files") && !expected_dirs.contains(&key(&path)) {
                out.push(path);
                continue;
            }
            collect_stale(&path, expected_files, expected_dirs, out)?;
            continue;
        }
        if name.eq_ignore_ascii_case("INDEX.md") || name.eq_ignore_ascii_case("index.json") {
            continue;
        }
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            && !expected_files.contains(&key(&path))
        {
            out.push(path);
        }
    }
    Ok(())
}

fn unique_trash_dir(data: &DataDir) -> PathBuf {
    let base = data.markdown_dir().join(".trash");
    let stamp = now_secs();
    let mut candidate = base.join(stamp.to_string());
    let mut attempt = 1usize;
    while candidate.exists() {
        candidate = base.join(format!("{stamp}-{attempt}"));
        attempt += 1;
    }
    candidate
}

fn fs_create(path: &Path) -> io::Result<()> {
    std::fs::create_dir_all(path)
}

// ------------------------------------------------------------------ planning

/// Visible nodes in tree order with the path their mirror file will use.
fn plan_ordered(tree: &Tree) -> Vec<(NoteId, Planned)> {
    let mut used: HashMap<String, HashSet<String>> = HashMap::new();
    let mut out = Vec::new();
    walk(tree, None, &[], &mut used, &mut out);
    out
}

fn walk(
    tree: &Tree,
    parent: Option<&NoteId>,
    dirs: &[String],
    used: &mut HashMap<String, HashSet<String>>,
    out: &mut Vec<(NoteId, Planned)>,
) {
    let key = dirs.join("/").to_lowercase();
    for node in tree.children(parent) {
        if node.trashed {
            continue;
        }
        let base = unique(used, &key, &safe_segment(&node.title));
        match node.kind {
            NodeKind::Folder => {
                let mut child_dirs = dirs.to_vec();
                child_dirs.push(base);
                walk(tree, Some(&node.id), &child_dirs, used, out);
            }
            NodeKind::Note => {
                let file = format!("{base}.md");
                let mut relative_parts = dirs.to_vec();
                relative_parts.push(file.clone());
                out.push((
                    node.id.clone(),
                    Planned {
                        dirs: dirs.to_vec(),
                        file,
                        relative: relative_parts.join("/"),
                    },
                ));
                let has_children = tree
                    .children(Some(&node.id))
                    .iter()
                    .any(|child| !child.trashed);
                if has_children {
                    let mut child_dirs = dirs.to_vec();
                    child_dirs.push(base);
                    walk(tree, Some(&node.id), &child_dirs, used, out);
                }
            }
        }
    }
}

/// Allocates a unique name in one directory; collisions get ` (2)`, ` (3)`…
fn unique(used: &mut HashMap<String, HashSet<String>>, dir: &str, base: &str) -> String {
    let set = used.entry(dir.to_owned()).or_default();
    let mut candidate = base.to_owned();
    let mut attempt = 2usize;
    while set.contains(&candidate.to_lowercase()) {
        candidate = format!("{base} ({attempt})");
        attempt += 1;
    }
    set.insert(candidate.to_lowercase());
    candidate
}

/// Characters Windows rejects, plus trailing dots/spaces and device names.
fn safe_segment(title: &str) -> String {
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
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']);
    let mut name = if trimmed.is_empty() {
        "note".to_owned()
    } else {
        trimmed.chars().take(80).collect()
    };
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = name.split('.').next().unwrap_or(&name).to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        name.insert(0, '_');
    }
    name
}

fn key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

// ------------------------------------------------------------------- writing

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|f| format!("{}{}", f.to_string_lossy(), suffix))
        .unwrap_or_else(|| format!("file{suffix}"));
    path.with_file_name(name)
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> io::Result<bool> {
    if path.exists() {
        if let Ok(existing) = std::fs::read(path) {
            if existing == bytes {
                return Ok(false);
            }
        }
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let tmp = sibling(path, ".tmp");
    let outcome = (|| -> io::Result<()> {
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if outcome.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    outcome?;
    Ok(true)
}

fn write_note_file(
    root: &Path,
    tree: &Tree,
    id: &NoteId,
    planned: &Planned,
    rtf: Option<&[u8]>,
) -> io::Result<()> {
    let Some(node) = tree.get(id) else {
        return Ok(());
    };
    let asset_dir = format!("{}_files", planned.file.trim_end_matches(".md"));
    let export = match rtf {
        Some(bytes) if !bytes.is_empty() => {
            markdown::rtf_to_markdown(&String::from_utf8_lossy(bytes), &asset_dir)
        }
        _ => markdown::Export {
            markdown: String::new(),
            assets: Vec::new(),
            lossy: Vec::new(),
        },
    };

    let mut path = root.to_path_buf();
    for dir in &planned.dirs {
        path.push(dir);
    }
    path.push(&planned.file);

    let body = format!(
        "{}\n{}\n",
        front_matter(tree, node, &planned.relative),
        export.markdown.trim_end()
    );
    write_if_changed(&path, body.as_bytes())?;

    if !export.assets.is_empty() {
        let folder = path.parent().unwrap_or(Path::new(".")).join(&asset_dir);
        std::fs::create_dir_all(&folder)?;
        for (file, bytes) in &export.assets {
            write_if_changed(&folder.join(file), bytes)?;
        }
    }
    Ok(())
}

fn front_matter(tree: &Tree, node: &Node, _relative: &str) -> String {
    let raw_path = tree.path_titles(&node.id).join("/");
    format!(
        "---\nid: {}\ntitle: {}\nparent_id: {}\npath: {}\ncreated: {}\nmodified: {}\ntags: []\n---",
        node.id.as_str(),
        yaml_string(&node.title),
        match &node.parent {
            Some(parent) => yaml_string(parent.as_str()),
            None => "null".to_owned(),
        },
        yaml_string(&raw_path),
        node.created,
        node.modified,
    )
}

fn yaml_string(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "");
    format!("\"{escaped}\"")
}

fn write_index(data: &DataDir, tree: &Tree, ordered: &[(NoteId, Planned)]) -> io::Result<PathBuf> {
    let mut text = String::from(
        "# Nodi notes — Markdown mirror\n\n\
         This folder is a read-only Markdown mirror of a Nodi note tree, kept up to date\n\
         for AI agents. Nodi regenerates it after every save; edit notes in Nodi, not here.\n\n\
         ## How this folder is organised\n\n\
         - One `.md` file per note, inside folders named after the note tree.\n\
         - Every file starts with YAML front matter: `id`, `title`, `parent_id`, `path`,\n\
           `created`, `modified`, `tags`.\n\
         - A note's pictures live in `<note name>_files/` next to the file.\n\
         - `index.json` lists every note with its parent, children, modified time and file.\n\n\
         ## Notes\n\n",
    );
    let mut lookup: HashMap<NoteId, &Planned> = HashMap::new();
    for (id, planned) in ordered {
        lookup.insert(id.clone(), planned);
    }
    write_index_branch(&mut text, tree, None, 0, &lookup, &mut Vec::new());
    let path = data.markdown_dir().join("INDEX.md");
    write_if_changed(&path, text.as_bytes())?;
    Ok(path)
}

fn write_index_branch<'a>(
    text: &mut String,
    tree: &'a Tree,
    parent: Option<&NoteId>,
    depth: usize,
    lookup: &HashMap<NoteId, &Planned>,
    seen: &mut Vec<&'a NoteId>,
) {
    for node in tree.children(parent) {
        if node.trashed || seen.contains(&&node.id) {
            continue;
        }
        seen.push(&node.id);
        let indent = "  ".repeat(depth);
        if let Some(planned) = lookup.get(&node.id) {
            text.push_str(&format!(
                "{indent}- [{}]({}) — modified {}, id `{}`\n",
                node.title.replace('[', "\\[").replace(']', "\\]"),
                encode_link(&planned.relative),
                node.modified,
                node.id.as_str()
            ));
        } else {
            text.push_str(&format!("{indent}- {}/\n", node.title));
        }
        write_index_branch(text, tree, Some(&node.id), depth + 1, lookup, seen);
    }
}

fn encode_link(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for ch in path.chars() {
        match ch {
            ' ' => out.push_str("%20"),
            '#' => out.push_str("%23"),
            '%' => out.push_str("%25"),
            '?' => out.push_str("%3F"),
            _ => out.push(ch),
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct IndexEntry {
    id: String,
    title: String,
    path: String,
    parent: Option<String>,
    children: Vec<String>,
    modified: u64,
    md_file: String,
}

#[derive(Serialize, Deserialize)]
struct IndexFile {
    generated: u64,
    count: usize,
    notes: Vec<IndexEntry>,
}

fn write_index_json(
    data: &DataDir,
    tree: &Tree,
    ordered: &[(NoteId, Planned)],
) -> io::Result<PathBuf> {
    let mut notes = Vec::with_capacity(ordered.len());
    for (id, planned) in ordered {
        let Some(node) = tree.get(id) else {
            continue;
        };
        let children = tree
            .children(Some(id))
            .into_iter()
            .filter(|child| !child.trashed && child.kind == NodeKind::Note)
            .map(|child| child.id.as_str().to_owned())
            .collect();
        notes.push(IndexEntry {
            id: node.id.as_str().to_owned(),
            title: node.title.clone(),
            path: tree.path_titles(id).join("/"),
            parent: node
                .parent
                .as_ref()
                .map(|parent| parent.as_str().to_owned()),
            children,
            modified: node.modified,
            md_file: planned.relative.clone(),
        });
    }
    let path = data.markdown_dir().join("index.json");
    if let Ok(existing_bytes) = std::fs::read(&path) {
        if let Ok(existing) = serde_json::from_slice::<IndexFile>(&existing_bytes) {
            if existing.notes == notes {
                return Ok(path);
            }
        }
    }
    let file = IndexFile {
        generated: now_secs(),
        count: notes.len(),
        notes,
    };
    let json = serde_json::to_string_pretty(&file)
        .map_err(|err| io::Error::other(format!("cannot serialise index.json: {err}")))?;
    write_if_changed(&path, json.as_bytes())?;
    Ok(path)
}
