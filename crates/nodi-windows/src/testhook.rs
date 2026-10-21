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
use crate::wnd::{editor, menu, tree};

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
        ("objects", Some(window)) => editor::object_count(window).to_string(),
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
