//! Import (File menu, Notes Explorer right-click, files dropped on the window)
//! and Export (one note, the selection, everything, a complete backup copy).
//!
//! Imports never touch the originals: a worker thread plans and converts the
//! files (`nodi_core::import`), the UI thread then creates the notes — under a
//! new "Imported <date>" folder, or inside the right-clicked node — and hands
//! their content to the writer thread like any other save. Each imported note
//! remembers its file, so it can be shown, or written back on request (Save
//! Back to Original File, which keeps a copy of the old file first).

use std::path::{Path, PathBuf};

use nodi_core::autosave::{Job, SaveRequest};
use nodi_core::import::{self, Planned};
use nodi_core::{NodeKind, NoteId};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Shell::{DragFinish, DragQueryFileW, DragQueryPoint, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

use crate::app::App;
use crate::wnd::{dialogs, editor, image, status};

/// Posted by the import worker with a boxed [`Converted`].
pub const WM_APP_IMPORT_DONE: u32 = WM_APP + 2;

/// One converted node.
pub enum Item {
    Note {
        title: String,
        rtf: Vec<u8>,
        txt: String,
        source: PathBuf,
    },
    Folder {
        title: String,
        children: Vec<Item>,
    },
}

/// Where imported notes go.
#[derive(Clone, Debug)]
pub enum Target {
    /// A new top-level "Imported <date>" folder.
    NewFolder,
    /// Directly inside this node (`None` = the top level).
    Into(Option<NoteId>),
}

/// Everything the worker produced.
pub struct Converted {
    pub items: Vec<Item>,
    /// `(path, reason)` of files that were left out or failed.
    pub skipped: Vec<(PathBuf, String)>,
    pub target: Target,
}

/// File > Import > Files...
pub fn import_files(app: &mut App) {
    let paths = dialogs::open_note_files(app.hwnd);
    if !paths.is_empty() {
        start_into(app, paths, Target::NewFolder);
    }
}

/// File > Import > Folder...
pub fn import_folder(app: &mut App) {
    if let Some(folder) = dialogs::open_folder(app.hwnd) {
        start_into(app, vec![PathBuf::from(folder)], Target::NewFolder);
    }
}

/// Notes Explorer right-click > Import Files Here / Import Folder Here.
pub fn import_here(app: &mut App, folder: bool) {
    let parent = app.context_note.clone();
    if let Some(parent) = &parent {
        if !app.check_unlocked(parent) {
            return;
        }
    }
    let paths = if folder {
        dialogs::open_folder(app.hwnd)
            .map(|folder| vec![PathBuf::from(folder)])
            .unwrap_or_default()
    } else {
        dialogs::open_note_files(app.hwnd)
    };
    if !paths.is_empty() {
        start_into(app, paths, Target::Into(parent));
    }
}

/// Files dropped on the window go into a new "Imported" folder.
pub fn start(app: &mut App, paths: Vec<PathBuf>) {
    start_into(app, paths, Target::NewFolder);
}

/// Plans and converts `paths` on a worker thread; the result comes back as
/// [`WM_APP_IMPORT_DONE`].
pub fn start_into(app: &mut App, paths: Vec<PathBuf>, target: Target) {
    let width = app
        .active_editor()
        .map_or(9000, crate::wnd::format::full_width_twips);
    let hwnd = app.hwnd as isize;
    status::set_message(app, "Importing...");
    let spawned = std::thread::Builder::new()
        .name("nodi-import".to_owned())
        .spawn(move || {
            let plan = import::plan(&paths);
            let mut skipped = plan.skipped;
            let items = convert(&plan.items, width, &mut skipped);
            let raw = Box::into_raw(Box::new(Converted {
                items,
                skipped,
                target,
            })) as isize;
            // SAFETY: posting to the frame; on failure the box is reclaimed.
            unsafe {
                if PostMessageW(hwnd as HWND, WM_APP_IMPORT_DONE, 0, raw) == 0 {
                    drop(Box::from_raw(raw as *mut Converted));
                }
            }
        });
    if spawned.is_err() {
        status::set_message(app, "Import failed: no worker thread");
    }
}

fn convert(planned: &[Planned], width: i32, skipped: &mut Vec<(PathBuf, String)>) -> Vec<Item> {
    planned
        .iter()
        .filter_map(|item| match item {
            Planned::Note { title, path } => match import::convert_file(path, width) {
                Ok((rtf, txt)) => Some(Item::Note {
                    title: title.clone(),
                    rtf,
                    txt,
                    source: path.clone(),
                }),
                Err(err) => {
                    skipped.push((path.clone(), err.to_string()));
                    None
                }
            },
            Planned::Folder { title, children } => {
                let children = convert(children, width, skipped);
                (!children.is_empty()).then(|| Item::Folder {
                    title: title.clone(),
                    children,
                })
            }
        })
        .collect()
}

/// UI thread: creates the converted notes where the import was aimed.
pub fn finish(app: &mut App, converted: Converted) {
    let mut count = 0usize;
    if !converted.items.is_empty() {
        let parent = match &converted.target {
            Target::NewFolder => {
                let stamp = local_stamp();
                Some(
                    app.tree
                        .new_node(NodeKind::Folder, format!("Imported {stamp}"), None),
                )
            }
            // A target that vanished meanwhile: the top level.
            Target::Into(parent) => parent
                .clone()
                .filter(|id| app.tree.get(id).is_some_and(|node| !node.trashed)),
        };
        let first = add(app, &converted.items, parent.as_ref(), &mut count);
        app.tree_save();
        app.selected = parent.clone().or(first);
        if parent.is_some() {
            app.expand(parent);
        }
        app.rebuild_tree();
    }
    let mut message =
        format!("{count} note(s) imported — copies; the original files were not changed");
    if !converted.skipped.is_empty() {
        message.push_str(&format!(", {} skipped", converted.skipped.len()));
    }
    status::set_message(app, &message);
    if !converted.skipped.is_empty() {
        let list: Vec<String> = converted
            .skipped
            .iter()
            .take(15)
            .map(|(path, reason)| format!("{}: {reason}", path.display()))
            .collect();
        let more = converted.skipped.len().saturating_sub(15);
        let tail = if more > 0 {
            format!("\n… and {more} more")
        } else {
            String::new()
        };
        dialogs::info(
            app.hwnd,
            &format!("{message}.\n\nNot imported:\n{}{tail}", list.join("\n")),
        );
    }
}

/// Creates `items` below `parent`; returns the first node created.
fn add(
    app: &mut App,
    items: &[Item],
    parent: Option<&NoteId>,
    count: &mut usize,
) -> Option<NoteId> {
    let mut first = None;
    for item in items {
        match item {
            Item::Note {
                title,
                rtf,
                txt,
                source,
            } => {
                let id = app
                    .tree
                    .new_node(NodeKind::Note, title.as_str(), parent.cloned());
                app.tree
                    .set_source(&id, Some(source.to_string_lossy().into_owned()));
                if let Some(writer) = &app.writer {
                    writer.submit(Job::Note(SaveRequest {
                        note: id.clone(),
                        seq: 1,
                        rtf: rtf.clone(),
                        txt: txt.clone(),
                    }));
                }
                *count += 1;
                first.get_or_insert(id);
            }
            Item::Folder { title, children } => {
                let id = app
                    .tree
                    .new_node(NodeKind::Folder, title.as_str(), parent.cloned());
                add(app, children, Some(&id), count);
                first.get_or_insert(id);
            }
        }
    }
    first
}

/// Files dropped on the window: pictures dropped on the note go into it,
/// everything else is imported.
pub fn on_drop(app: &mut App, drop: HDROP) {
    let mut files = Vec::new();
    // SAFETY: `drop` is the handle Windows passed with WM_DROPFILES; it is
    // released with DragFinish below.
    let point = unsafe {
        let count = DragQueryFileW(drop, u32::MAX, std::ptr::null_mut(), 0);
        for index in 0..count {
            let length = DragQueryFileW(drop, index, std::ptr::null_mut(), 0) as usize;
            let mut buffer = vec![0u16; length + 1];
            DragQueryFileW(drop, index, buffer.as_mut_ptr(), buffer.len() as u32);
            files.push(PathBuf::from(String::from_utf16_lossy(&buffer[..length])));
        }
        let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
        DragQueryPoint(drop, &mut point);
        DragFinish(drop);
        point
    };
    let over_editor = app.active_editor().is_some_and(|window| {
        let mut rect = windows_sys::Win32::Foundation::RECT::default();
        let mut client = point;
        // SAFETY: coordinate conversion between our own windows.
        unsafe {
            windows_sys::Win32::Graphics::Gdi::ClientToScreen(app.hwnd, &mut client);
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(window, &mut rect);
        }
        client.x >= rect.left
            && client.x < rect.right
            && client.y >= rect.top
            && client.y < rect.bottom
    });
    let (pictures, others): (Vec<PathBuf>, Vec<PathBuf>) =
        files.into_iter().partition(|path| is_picture(path));
    if over_editor {
        if let Some(window) = app.active_editor() {
            for picture in &pictures {
                if let Ok(bytes) = std::fs::read(picture) {
                    if let Err(message) = image::insert_bytes(window, &bytes) {
                        dialogs::error(app.hwnd, &message);
                    }
                }
            }
        }
    }
    if !others.is_empty() {
        start(app, others);
    }
}

/// `2026-10-28 19:05` in local time.
fn local_stamp() -> String {
    let mut time = windows_sys::Win32::Foundation::SYSTEMTIME::default();
    // SAFETY: fills a plain struct we own.
    unsafe { windows_sys::Win32::System::SystemInformation::GetLocalTime(&mut time) };
    format!(
        "{:04}-{:02}-{:02} {:02}.{:02}",
        time.wYear, time.wMonth, time.wDay, time.wHour, time.wMinute
    )
}

fn is_picture(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            ["png", "jpg", "jpeg", "gif", "bmp"]
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
}

/// File > Export Note As...: the active note as `.md` (with a picture
/// folder), `.rtf` or `.txt`, chosen by the file type.
pub fn export_note(app: &mut App) {
    let Some(index) = app.active else {
        status::set_message(app, "Open a note first");
        return;
    };
    let title = app.tabs[index].title.clone();
    let window = app.tabs[index].editor;
    let name = crate::app::safe_file_name(&title);
    let Some(path) = dialogs::save_file_typed(
        app.hwnd,
        &name,
        &[
            ("Markdown (*.md)", "md"),
            ("Rich Text (*.rtf)", "rtf"),
            ("Plain text (*.txt)", "txt"),
        ],
    ) else {
        return;
    };
    let path = PathBuf::from(path);
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("md")
        .to_ascii_lowercase();
    let result = match extension.as_str() {
        "rtf" => editor::stream_out_rtf(window)
            .ok_or_else(|| "the note could not be read".to_owned())
            .and_then(|rtf| std::fs::write(&path, rtf).map_err(|err| err.to_string())),
        "txt" => {
            let text = editor::plain_text(window).replace('\r', "\r\n");
            let mut bytes = b"\xEF\xBB\xBF".to_vec();
            bytes.extend_from_slice(text.as_bytes());
            std::fs::write(&path, bytes).map_err(|err| err.to_string())
        }
        _ => {
            crate::app::export_markdown_to(app, &path);
            return;
        }
    };
    match result {
        Ok(()) => status::set_message(app, &format!("Exported to {}", path.display())),
        Err(message) => dialogs::error(app.hwnd, &format!("Export failed:\n{message}")),
    }
}

/// Lets every pending save reach the disk, so an export has the latest text.
fn settle(app: &mut App) {
    app.flush_all();
    if let Some(writer) = &app.writer {
        writer.flush_blocking(std::time::Duration::from_secs(5));
    }
}

/// Asks which formats to write; `None` when cancelled or nothing ticked.
fn ask_formats(app: &App, what: &str) -> Option<nodi_core::mirror::Formats> {
    let values = crate::wnd::form::Form::new("Export")
        .note(&format!(
            "{what}\n\nEach note becomes one file per format, in folders like the \
             Notes Explorer. Nothing in Nodi changes."
        ))
        .checkbox("Markdown (.md) — for Obsidian, GitHub, AI tools", true)
        .checkbox(
            "Rich Text (.rtf) — opens in Word with all formatting",
            false,
        )
        .checkbox("Plain text (.txt)", false)
        .show(app.hwnd)?;
    let formats = nodi_core::mirror::Formats {
        markdown: values.first().is_some_and(|value| value == "1"),
        rtf: values.get(1).is_some_and(|value| value == "1"),
        txt: values.get(2).is_some_and(|value| value == "1"),
    };
    (formats.markdown || formats.rtf || formats.txt).then_some(formats)
}

fn run_export(app: &mut App, roots: Option<Vec<NoteId>>, what: &str) {
    let Some(formats) = ask_formats(app, what) else {
        return;
    };
    let Some(data) = app.data.clone() else {
        return;
    };
    let Some(folder) = dialogs::open_folder(app.hwnd) else {
        return;
    };
    settle(app);
    match nodi_core::mirror::export_many(
        &data,
        &app.tree,
        roots.as_deref(),
        Path::new(&folder),
        formats,
    ) {
        Ok(count) => {
            let message = format!("{count} note(s) exported to {folder}");
            status::set_message(app, &message);
            dialogs::info(
                app.hwnd,
                &format!("{message}.\n\nNothing in Nodi was changed."),
            );
        }
        Err(err) => dialogs::error(app.hwnd, &format!("Export failed:\n{err}")),
    }
}

/// Notes Explorer right-click > Export... / File > Export > Selected: the
/// selected notes and folders with everything inside them.
pub fn export_selection(app: &mut App) {
    let ids = app.selection_ids();
    if ids.is_empty() {
        status::set_message(app, "Select notes or folders in the Notes Explorer first");
        return;
    }
    let what = if ids.len() == 1 {
        let title = app
            .tree
            .get(&ids[0])
            .map(|node| node.title.clone())
            .unwrap_or_default();
        format!("Export \"{title}\" and everything inside it.")
    } else {
        format!(
            "Export the {} selected items and everything inside them.",
            ids.len()
        )
    };
    run_export(app, Some(ids), &what);
}

/// File > Export > All Notes...
pub fn export_all(app: &mut App) {
    run_export(app, None, "Export every note (the Trash is left out).");
}

/// File > Export > Complete Backup Copy...: the whole notebook in Nodi's own
/// format, restorable with Tools > Restore From Backup Folder.
pub fn export_backup(app: &mut App) {
    let Some(data) = app.data.clone() else {
        return;
    };
    let Some(folder) = dialogs::open_folder(app.hwnd) else {
        return;
    };
    settle(app);
    match nodi_core::backup::copy_to(&data, Path::new(&folder)) {
        Ok(dir) => {
            let message = format!("Complete backup copy written to {}", dir.display());
            status::set_message(app, &message);
            dialogs::info(
                app.hwnd,
                &format!(
                    "{message}.\n\nTo bring it back later: Tools > Restore From Backup Folder..."
                ),
            );
        }
        Err(err) => dialogs::error(app.hwnd, &format!("Backup copy failed:\n{err}")),
    }
}

/// Right-click > Show Original File: Explorer with the imported file selected.
pub fn show_original(app: &mut App) {
    let Some(source) = app
        .context_note
        .as_ref()
        .and_then(|id| app.import_source(id))
    else {
        return;
    };
    if !Path::new(&source).exists() {
        dialogs::info(
            app.hwnd,
            &format!("The original file is no longer there:\n{source}"),
        );
        return;
    }
    let _ = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{source}"))
        .spawn();
}

/// Right-click > Save Back to Original File...: writes the note over the file
/// it was imported from, in that file's format, after copying the old file to
/// `data\recovery\originals`.
pub fn save_back(app: &mut App) {
    let Some(id) = app.context_note.clone() else {
        return;
    };
    let Some(source) = app.import_source(&id) else {
        return;
    };
    let Some(data) = app.data.clone() else {
        return;
    };
    let path = PathBuf::from(&source);
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let kind = match extension.as_str() {
        "md" | "markdown" => "Markdown",
        "rtf" => "Rich Text",
        _ => "plain text",
    };
    let question = format!(
        "Write this note over the original file?\n\n{source}\n\nIt is saved as {kind}. \
         The current file is copied to Nodi's data\\recovery\\originals folder first."
    );
    if !dialogs::ask_yes_no(app.hwnd, &question) {
        return;
    }
    settle(app);
    let result = (|| -> std::io::Result<()> {
        if path.exists() {
            let keep = data
                .recovery_dir()
                .join("originals")
                .join(nodi_core::backup::timestamp_name(nodi_core::id::now_secs()));
            std::fs::create_dir_all(&keep)?;
            let name = path.file_name().unwrap_or_default();
            std::fs::copy(&path, keep.join(name))?;
        }
        let rtf = data.load_note(&id).unwrap_or_default();
        match extension.as_str() {
            "md" | "markdown" => {
                let stem = path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "note".to_owned());
                let assets = format!("{stem}_files");
                let export =
                    nodi_core::markdown::rtf_to_markdown(&String::from_utf8_lossy(&rtf), &assets);
                std::fs::write(&path, export.markdown.as_bytes())?;
                if !export.assets.is_empty() {
                    let folder = path.parent().unwrap_or(Path::new(".")).join(&assets);
                    std::fs::create_dir_all(&folder)?;
                    for (file, bytes) in &export.assets {
                        std::fs::write(folder.join(file), bytes)?;
                    }
                }
            }
            "rtf" => std::fs::write(&path, &rtf)?,
            _ => {
                let text = data.load_note_text(&id).unwrap_or_default();
                let text = text
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .replace('\n', "\r\n");
                std::fs::write(&path, text.as_bytes())?;
            }
        }
        Ok(())
    })();
    match result {
        Ok(()) => status::set_message(app, &format!("Saved back to {source}")),
        Err(err) => dialogs::error(app.hwnd, &format!("Could not write the file:\n{err}")),
    }
}

/// File > Export Folder (older menu entry and test hook): the selected note
/// or folder and everything below it, as Markdown files in a folder you
/// choose.
pub fn export_folder(app: &mut App) {
    let Some(root) = app.context_note.clone().or_else(|| app.selected.clone()) else {
        status::set_message(app, "Select a note or folder in the Notes Explorer first");
        return;
    };
    let Some(data) = app.data.clone() else {
        return;
    };
    let Some(folder) = dialogs::open_folder(app.hwnd) else {
        return;
    };
    // Unsaved edits first, so the export has them.
    app.flush_all();
    if let Some(writer) = &app.writer {
        writer.flush_blocking(std::time::Duration::from_secs(5));
    }
    match nodi_core::mirror::export(&data, &app.tree, &root, Path::new(&folder)) {
        Ok(count) => {
            status::set_message(app, &format!("{count} note(s) exported to {folder}"));
        }
        Err(err) => dialogs::error(app.hwnd, &format!("Export failed:\n{err}")),
    }
}
