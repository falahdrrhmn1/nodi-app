//! File > Import Files / Import Folder, files dropped on the window, and
//! File > Export Note As / Export Folder.
//!
//! Imports never touch the originals: a worker thread plans and converts the
//! files (`nodi_core::import`), the UI thread then creates the notes under a
//! new "Imported <date>" folder and hands their content to the writer thread
//! like any other save.

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
    },
    Folder {
        title: String,
        children: Vec<Item>,
    },
}

/// Everything the worker produced.
pub struct Converted {
    pub items: Vec<Item>,
    /// `(path, reason)` of files that were left out or failed.
    pub skipped: Vec<(PathBuf, String)>,
}

/// File > Import Files...
pub fn import_files(app: &mut App) {
    let paths = dialogs::open_note_files(app.hwnd);
    if !paths.is_empty() {
        start(app, paths);
    }
}

/// File > Import Folder...
pub fn import_folder(app: &mut App) {
    if let Some(folder) = dialogs::open_folder(app.hwnd) {
        start(app, vec![PathBuf::from(folder)]);
    }
}

/// Plans and converts `paths` on a worker thread; the result comes back as
/// [`WM_APP_IMPORT_DONE`].
pub fn start(app: &mut App, paths: Vec<PathBuf>) {
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
            let raw = Box::into_raw(Box::new(Converted { items, skipped })) as isize;
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

/// UI thread: creates the converted notes under "Imported <date>".
pub fn finish(app: &mut App, converted: Converted) {
    let mut count = 0usize;
    if !converted.items.is_empty() {
        let stamp = local_stamp();
        let folder = app
            .tree
            .new_node(NodeKind::Folder, format!("Imported {stamp}"), None);
        add(app, &converted.items, &folder, &mut count);
        app.tree_save();
        app.selected = Some(folder.clone());
        app.expand(Some(folder));
        app.rebuild_tree();
    }
    let mut message = format!("{count} note(s) imported");
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

fn add(app: &mut App, items: &[Item], parent: &NoteId, count: &mut usize) {
    for item in items {
        match item {
            Item::Note { title, rtf, txt } => {
                let id = app
                    .tree
                    .new_node(NodeKind::Note, title.as_str(), Some(parent.clone()));
                if let Some(writer) = &app.writer {
                    writer.submit(Job::Note(SaveRequest {
                        note: id,
                        seq: 1,
                        rtf: rtf.clone(),
                        txt: txt.clone(),
                    }));
                }
                *count += 1;
            }
            Item::Folder { title, children } => {
                let id = app
                    .tree
                    .new_node(NodeKind::Folder, title.as_str(), Some(parent.clone()));
                add(app, children, &id, count);
            }
        }
    }
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

/// File > Export Folder...: the selected note or folder and everything below
/// it, as Markdown files in a folder you choose.
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
