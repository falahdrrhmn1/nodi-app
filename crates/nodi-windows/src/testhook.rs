//! Remote control for the end-to-end tests (`scripts/e2e.ps1`).
//!
//! Disabled unless `NODI_TEST_HOOKS=1` is set when Nodi starts. The test sends
//! `WM_COPYDATA` with [`MAGIC`] and a UTF-8 command line; the reply is written
//! to the file named by `NODI_TEST_REPLY`. Everything runs inside nodi.exe, so
//! no pointer has to cross a process boundary (Rich Edit messages above
//! `WM_USER` are not marshalled by Windows).

use std::mem::size_of;

use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW;

use crate::app::App;
use crate::wnd::{editkeys, editor, formula, image, link, menu, table, tree};

/// `dwData` of every test command ('NODT').
pub const MAGIC: usize = 0x4E4F_4454;

/// True when the process was started with `NODI_TEST_HOOKS=1`.
pub fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("NODI_TEST_HOOKS").is_some_and(|value| value == "1"))
}

/// Handles one `WM_COPYDATA`. Returns false when it was not a test command.
///
/// # Safety
/// `data` must be the `COPYDATASTRUCT` pointer Windows passed in `lParam`.
pub unsafe fn handle(app: &mut App, data: *const COPYDATASTRUCT) -> bool {
    if data.is_null() || (*data).dwData != MAGIC || (*data).lpData.is_null() {
        return false;
    }
    let bytes = std::slice::from_raw_parts((*data).lpData as *const u8, (*data).cbData as usize);
    let line = String::from_utf8_lossy(bytes).into_owned();
    let reply = run(app, &line);
    if let Some(path) = std::env::var_os("NODI_TEST_REPLY") {
        let _ = std::fs::write(path, reply.as_bytes());
    }
    true
}

fn run(app: &mut App, line: &str) -> String {
    let (command, rest) = line.split_once(' ').unwrap_or((line, ""));
    let editor = app.active_editor();
    match (command, editor) {
        ("state", _) => app.active_state().unwrap_or("").to_owned(),
        ("tabs", _) => {
            let titles: Vec<&str> = app.tabs.iter().map(|tab| tab.title.as_str()).collect();
            format!(
                "{}\n{}",
                app.active.map_or(-1, |index| index as i64),
                titles.join("|")
            )
        }
        ("cmd", _) => match rest.trim().parse::<u16>() {
            Ok(id) => {
                menu::on_command(app, id);
                "ok".to_owned()
            }
            Err(_) => "bad id".to_owned(),
        },
        ("select-title", _) => {
            let found = app
                .tree
                .iter_ordered()
                .into_iter()
                .find(|node| !node.trashed && node.title == rest)
                .map(|node| node.id.clone());
            match found {
                Some(id) => {
                    app.context_note = None;
                    tree::select_item(app, &id);
                    "ok".to_owned()
                }
                None => "not found".to_owned(),
            }
        }
        ("rename-selected", _) => match app.selected.clone() {
            Some(id) => {
                app.rename_node(id, rest.to_owned());
                "ok".to_owned()
            }
            None => "no selection".to_owned(),
        },
        (_, None) => "no editor".to_owned(),
        ("text", Some(window)) => editor::plain_text(window),
        ("rtf", Some(window)) => editor::stream_out_rtf(window)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default(),
        ("type", Some(window)) => {
            let text = crate::win::wide(&rest.replace("\\n", "\r").replace("\\t", "\t"));
            // SAFETY: `text` is NUL terminated; `1` keeps the change undoable.
            unsafe {
                SendMessageW(window, EM_REPLACESEL, 1, text.as_ptr() as isize);
            }
            "ok".to_owned()
        }
        ("select", Some(window)) => match parse_range(rest) {
            Some((from, to)) => {
                editor::set_selection(window, from, to);
                "ok".to_owned()
            }
            None => "bad range".to_owned(),
        },
        ("caret", Some(window)) => {
            let (line, column) = editor::line_col(window);
            format!("{line},{column}")
        }
        ("fmt", Some(window)) => match parse_range(rest) {
            Some((from, to)) => {
                editor::set_selection(window, from, to);
                let format = editor::selection_format(window);
                format!(
                    "bold={} italic={} underline={} strike={} face={} size={}",
                    u8::from(format.dw_effects & editor::CFE_BOLD != 0),
                    u8::from(format.dw_effects & editor::CFE_ITALIC != 0),
                    u8::from(format.dw_effects & editor::CFE_UNDERLINE != 0),
                    u8::from(format.dw_effects & editor::CFE_STRIKEOUT != 0),
                    String::from_utf16_lossy(
                        &format
                            .sz_face_name
                            .iter()
                            .copied()
                            .take_while(|unit| *unit != 0)
                            .collect::<Vec<u16>>()
                    ),
                    format.y_height
                )
            }
            None => "bad range".to_owned(),
        },
        ("pictures", Some(window)) => editor::picture_count(window).to_string(),
        ("units", Some(window)) => editor::text_units(window, 0, editor::text_length(window))
            .iter()
            .map(|unit| format!("{unit:04X}"))
            .collect::<Vec<_>>()
            .join(" "),
        ("drag", Some(window)) => {
            // drag x1 y1 x2 y2 (editor client pixels), left button.
            let points: Vec<i32> = rest
                .split_whitespace()
                .filter_map(|part| part.parse().ok())
                .collect();
            if points.len() != 4 {
                return "usage: drag x1 y1 x2 y2".to_owned();
            }
            let pack = |x: i32, y: i32| ((y as isize) << 16) | (x as isize & 0xFFFF);
            // SAFETY: plain mouse messages to our own Rich Edit.
            unsafe {
                SendMessageW(window, 0x0201, 1, pack(points[0], points[1]));
                for step in 1..=10 {
                    let x = points[0] + (points[2] - points[0]) * step / 10;
                    let y = points[1] + (points[3] - points[1]) * step / 10;
                    SendMessageW(window, 0x0200, 1, pack(x, y));
                }
                SendMessageW(window, 0x0202, 0, pack(points[2], points[3]));
            }
            "ok".to_owned()
        }
        ("pos", Some(window)) => {
            // Client position of character `n` (EM_POSFROMCHAR).
            let index: usize = rest.trim().parse().unwrap_or(0);
            let mut point = windows_sys::Win32::Foundation::POINTL { x: 0, y: 0 };
            // SAFETY: `point` is a valid out parameter.
            unsafe {
                SendMessageW(
                    window,
                    0x00D6,
                    &mut point as *mut _ as usize,
                    index as isize,
                );
            }
            format!("{} {}", point.x, point.y)
        }
        ("sel", Some(window)) => {
            let (from, to) = editor::selection(window);
            format!("{from} {to} len={}", editor::text_length(window))
        }
        ("sel-rtf", Some(window)) => editor::selection_rtf(window)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default(),
        ("insert-rtf", Some(window)) => editor::insert_rtf(window, rest.as_bytes()).to_string(),
        ("insert-image", Some(window)) => match std::fs::read(rest.trim()) {
            Ok(bytes) => match image::insert_bytes(window, &bytes) {
                Ok(()) => "ok".to_owned(),
                Err(message) => message,
            },
            Err(err) => err.to_string(),
        },
        ("table", Some(window)) => match parse_range(rest) {
            Some((rows, cols)) => match table::insert(window, rows as usize, cols as usize) {
                Ok(()) => {
                    app.mark_dirty(window);
                    "ok".to_owned()
                }
                Err(message) => message,
            },
            None => "usage: table rows cols".to_owned(),
        },
        ("formula", Some(window)) => match formula::render(rest) {
            Ok(rendered) => match formula::insert_rendered(window, &rendered, rest) {
                Ok(()) => {
                    app.mark_dirty(window);
                    "ok".to_owned()
                }
                Err(message) => message,
            },
            Err(message) => format!("error: {message}"),
        },
        ("formula-source", Some(window)) => formula::selected_latex(window).unwrap_or_default(),
        ("row-below", Some(window)) => match table::insert_row(window, true) {
            Ok(()) => "ok".to_owned(),
            Err(message) => message,
        },
        ("picture-chunks", Some(window)) => image::pick_picture(window)
            .and_then(|rtf| nodi_core::rtf::picture_blob(&rtf))
            .map(|blob| {
                let mut names = Vec::new();
                let mut pos = 8;
                while pos + 8 <= blob.len() {
                    let length = u32::from_be_bytes([
                        blob[pos],
                        blob[pos + 1],
                        blob[pos + 2],
                        blob[pos + 3],
                    ]) as usize;
                    names.push(String::from_utf8_lossy(&blob[pos + 4..pos + 8]).into_owned());
                    pos += 12 + length;
                }
                format!("{} bytes: {}", blob.len(), names.join(","))
            })
            .unwrap_or_else(|| "no picture".to_owned()),
        ("link", Some(window)) => match rest.split_once('|') {
            Some((text, target)) => {
                let ok = editor::insert_rtf(window, link::link_rtf(text, target).as_bytes());
                app.mark_dirty(window);
                ok.to_string()
            }
            None => "usage: link text|target".to_owned(),
        },
        ("link-target", Some(window)) => {
            let (from, to) = editor::selection(window);
            link::target_of(
                window,
                editor::CharRange {
                    cp_min: from,
                    cp_max: to,
                },
            )
        }
        ("markdown", Some(window)) => editor::stream_out_rtf(window)
            .map(|rtf| {
                nodi_core::markdown::rtf_to_markdown(&String::from_utf8_lossy(&rtf), "files")
                    .markdown
            })
            .unwrap_or_default(),
        ("enter", Some(window)) => {
            if !editkeys::newline_keep_indent(window) {
                editor::replace_selection(window, "\r", true);
            }
            "ok".to_owned()
        }
        ("tab", Some(window)) => {
            if !editkeys::tab_key(window, false) {
                crate::wnd::format::insert_text(window, "\t");
            }
            "ok".to_owned()
        }
        ("shift-tab", Some(window)) => {
            editkeys::tab_key(window, true);
            "ok".to_owned()
        }
        _ => format!("unknown command: {command}"),
    }
}

/// `EM_REPLACESEL` (an Edit-control message Rich Edit also implements).
const EM_REPLACESEL: u32 = 0x00C2;

fn parse_range(text: &str) -> Option<(i32, i32)> {
    let mut parts = text.split_whitespace();
    let from = parts.next()?.parse().ok()?;
    let to = parts.next()?.parse().ok()?;
    Some((from, to))
}

const _: () = assert!(size_of::<COPYDATASTRUCT>() == 24);
