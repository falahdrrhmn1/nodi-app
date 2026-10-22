//! Remote control for the end-to-end tests (`scripts/e2e.ps1`).
//!
//! Disabled unless `NODI_TEST_HOOKS=1` is set when Nodi starts. The test sends
//! `WM_COPYDATA` with [`MAGIC`] and a UTF-8 command line; the reply is written
//! to the file named by `NODI_TEST_REPLY`. Everything runs inside nodi.exe, so
//! no pointer has to cross a process boundary (Rich Edit messages above
//! `WM_USER` are not marshalled by Windows).

use std::mem::size_of;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_CHAR};

use nodi_core::tree::Position;

use crate::app::App;
use crate::wnd::{editkeys, editor, format, formula, image, link, menu, table, toolbar, tree};

/// `dwData` of every test command ('NODT').
pub const MAGIC: usize = 0x4E4F_4454;

thread_local! {
    static LAST_COMMAND: std::cell::Cell<(u16, u16)> = const { std::cell::Cell::new((0, 0)) };
}

/// Records the last `WM_COMMAND` (id, code) for the `lastcmd` query.
pub fn note_command(id: u16, code: u16) {
    LAST_COMMAND.with(|last| last.set((id, code)));
}

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
        ("accel", _) => format!("{:?}", app.accel),
        ("lastcmd", _) => LAST_COMMAND.with(|last| format!("{:?}", last.get())),
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
        ("move-rel", _) => move_rel(app, rest),
        ("cancel-rename", _) => {
            // SAFETY: reading the TreeView's label-edit control, if any.
            let edit = unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                    app.tree_view,
                    windows_sys::Win32::UI::Controls::TVM_GETEDITCONTROL,
                    0,
                    0,
                )
            } as HWND;
            if edit.is_null() {
                "no edit".to_owned()
            } else {
                // Esc cancels the inline label edit without renaming. Post it:
                // a synchronous key message from inside a WM_COPYDATA handler
                // can re-enter the tree's edit bookkeeping.
                // SAFETY: a key message to that edit control.
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
                        edit, 0x0100, 0x1B, 1,
                    );
                }
                "ok".to_owned()
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
        ("keys", Some(window)) => {
            // Types text one character at a time through WM_CHAR, so the
            // editor subclass sees spaces and tabs exactly like a keyboard.
            // `\n` becomes Enter, `\t` a Tab; `\\` escapes the next character.
            let mut chars = rest.chars().peekable();
            while let Some(ch) = chars.next() {
                match ch {
                    '\\' => {
                        let Some(literal) = chars.next() else { break };
                        send_char(window, literal as u32);
                    }
                    '\n' => send_key(window, 0x0D),
                    '\t' => send_key(window, 0x09),
                    ch => send_char(window, ch as u32),
                }
            }
            "ok".to_owned()
        }
        ("backspace", Some(window)) => {
            send_key(window, 0x08);
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
                let format = format::format_at(window, from, to);
                let colour = if format.dw_mask & 0x4000_0000 == 0 {
                    "unset".to_owned()
                } else if format.dw_effects & 0x4000_0000 != 0 {
                    "auto".to_owned()
                } else {
                    format!("#{:06x}", format.cr_text_color & 0x00FF_FFFF)
                };
                format!(
                    "bold={} italic={} underline={} strike={} face={} size={} hidden={} colour={colour}",
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
                    format.y_height,
                    u8::from(format.dw_effects & editor::CFE_HIDDEN != 0)
                )
            }
            None => "bad range".to_owned(),
        },
        ("hidden", Some(window)) => match parse_range(rest) {
            Some((from, to)) => format::hidden_at(window, from, to).to_string(),
            None => "bad range".to_owned(),
        },
        ("para", Some(window)) => {
            let format = editor::para_format(window);
            let (from, _) = editor::selection(window);
            let start = editor::paragraph_start(window, from);
            let end = editor::paragraph_end(window, from);
            format!(
                "start={} offset={} numbering={} level={} text={:?}",
                format.dx_start_indent,
                format.dx_offset,
                format.w_numbering,
                format::list_level(window),
                editor::text_range(window, start, end)
            )
        }
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
        ("tabpos", Some(window)) => {
            // Client x of character `n` in pixels and in twips. A tab stop every
            // 360 twips puts the first tab-indented character 360 twips in.
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
            let dpi = format::editor_dpi(window).max(1);
            let twips = (point.x as i64 * 1440 / dpi as i64) as i32;
            format!("px={} twips={twips}", point.x)
        }
        ("client", Some(window)) => {
            // Editor client width in pixels and twips (plain text width).
            let mut rect = windows_sys::Win32::Foundation::RECT::default();
            // SAFETY: `rect` is a valid out parameter.
            let ok = unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(window, &mut rect)
            };
            let dpi = format::editor_dpi(window).max(1);
            let twips = rect.right as i64 * 1440 / dpi as i64;
            let _ = ok;
            format!("px={} twips={twips}", rect.right)
        }
        ("picgoal", Some(window)) => {
            let found = image::pick_picture(window).or_else(|| {
                editor::stream_out_rtf(window)
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            });
            found
                .and_then(|rtf| nodi_core::rtf::picture_goal(&rtf))
                .map(|(width, height)| format!("{width} {height}"))
                .unwrap_or_else(|| "no picture".to_owned())
        }
        ("piccrop", Some(window)) => {
            let found = image::pick_picture(window).or_else(|| {
                editor::stream_out_rtf(window)
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            });
            found
                .and_then(|rtf| nodi_core::rtf::picture_crop(&rtf))
                .map(|(left, top, right, bottom)| format!("{left} {top} {right} {bottom}"))
                .unwrap_or_else(|| "no picture".to_owned())
        }
        ("autourl", Some(window)) => {
            let enable = rest.trim() != "off";
            // SAFETY: plain message to our own Rich Edit.
            unsafe {
                SendMessageW(window, 0x0400 + 91, usize::from(enable), 0);
            }
            if enable { "on" } else { "off" }.to_owned()
        }
        ("crop", Some(window)) => {
            let parts: Vec<i32> = rest
                .split_whitespace()
                .filter_map(|part| part.parse().ok())
                .collect();
            match parts.as_slice() {
                [left, top, right, bottom] => {
                    match image::crop_selection(window, *left, *top, *right, *bottom) {
                        Ok(()) => "ok".to_owned(),
                        Err(message) => message,
                    }
                }
                _ => "usage: crop left top right bottom (pixels)".to_owned(),
            }
        }
        ("findtext", Some(window)) => {
            let raw = editor::raw_text(window);
            let text = String::from_utf16_lossy(&raw);
            match text.find(rest) {
                Some(byte_index) => {
                    // Convert the UTF-8 byte offset of the match to UTF-16 units.
                    let utf16: usize = text
                        .chars()
                        .scan(0usize, |offset, ch| {
                            let here = *offset;
                            *offset += ch.len_utf8();
                            Some((here, ch.len_utf16()))
                        })
                        .take_while(|(offset, _)| *offset < byte_index)
                        .map(|(_, units)| units)
                        .sum();
                    editor::set_selection(window, utf16 as i32, utf16 as i32);
                    format!("{utf16}")
                }
                None => "not found".to_owned(),
            }
        }
        ("style", _) => toolbar::style_text(app),
        ("style-set", _) => match rest.trim().parse::<usize>() {
            Ok(level) if level <= 4 => {
                toolbar::apply_style(app, level);
                "ok".to_owned()
            }
            _ => "usage: style-set 0..4".to_owned(),
        },
        ("fontname", _) => toolbar::font_text(app),
        ("fontsize", _) => toolbar::size_text(app),
        ("trashitem", _) => tree::trash_label(app),
        ("combos", _) => {
            // Screen rects of the format combos: "WxH+X+Y" each, space separated.
            let mut parts = Vec::new();
            for combo in [app.style_combo, app.font_combo, app.size_combo] {
                let mut rect = windows_sys::Win32::Foundation::RECT::default();
                // SAFETY: `rect` is a valid out parameter; `combo` is ours.
                let ok = unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(combo, &mut rect)
                };
                if ok == 0 {
                    parts.push("none".to_owned());
                } else {
                    parts.push(format!(
                        "{}x{}+{}+{}",
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        rect.left,
                        rect.top
                    ));
                }
            }
            parts.join(" ")
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
        // Enter / Tab go through the same WM_CHAR path as real keys (the
        // editor subclass first, then Rich Edit's own handling).
        ("enter", Some(window)) => {
            // The full sequence a real key press produces.
            // SAFETY: keyboard messages to our own Rich Edit.
            unsafe {
                SendMessageW(window, 0x0100, 0x0D, 1); // WM_KEYDOWN VK_RETURN
                SendMessageW(window, WM_CHAR, 0x0D, 1);
                SendMessageW(window, 0x0101, 0x0D, 0xC000_0001u32 as isize); // WM_KEYUP
            }
            "ok".to_owned()
        }
        ("raw-key", Some(window)) => {
            // raw-key down|char|both <code>: Rich Edit without the subclass.
            let mut parts = rest.split_whitespace();
            let mode = parts.next().unwrap_or("both");
            let code: usize = parts
                .next()
                .and_then(|text| text.parse().ok())
                .unwrap_or(13);
            // SAFETY: plain keyboard messages to our own Rich Edit, bypassing
            // the subclass through DefSubclassProc's original window proc.
            unsafe {
                if mode != "char" {
                    SendMessageW(window, 0x0100, code, 1); // WM_KEYDOWN
                }
                if mode != "down" {
                    SendMessageW(window, WM_CHAR, code, 1);
                }
                if mode != "char" {
                    SendMessageW(window, 0x0101, code, 0xC000_0001u32 as isize);
                    // WM_KEYUP
                }
            }
            "ok".to_owned()
        }
        ("tab", Some(window)) => {
            send_key(window, 0x09);
            "ok".to_owned()
        }
        ("shift-tab", Some(window)) => {
            editkeys::tab_key(window, true);
            "ok".to_owned()
        }
        _ => format!("unknown command: {command}"),
    }
}

/// WM_KEYDOWN + WM_CHAR + WM_KEYUP, like `TranslateMessage` produces.
fn send_key(window: HWND, code: u32) {
    // SAFETY: keyboard messages to our own Rich Edit.
    unsafe {
        SendMessageW(window, 0x0100, code as usize, 1);
        SendMessageW(window, WM_CHAR, code as usize, 1);
        SendMessageW(window, 0x0101, code as usize, 0xC000_0001u32 as isize);
    }
}

/// A printable character: only WM_CHAR matters (and our space-marker hook).
fn send_char(window: HWND, code: u32) {
    // SAFETY: a character message to our own Rich Edit.
    unsafe {
        SendMessageW(window, WM_CHAR, code as usize, 1);
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

/// `move-rel <source title> before|after|inside <target title>` drives the
/// same model call as a tree drag & drop.
fn move_rel(app: &mut App, rest: &str) -> String {
    const MARKERS: [(&str, Position); 3] = [
        (" before ", Position::Before),
        (" after ", Position::After),
        (" inside ", Position::Inside),
    ];
    let Some((source_title, position, target_title)) =
        MARKERS.iter().find_map(|(needle, position)| {
            let at = rest.find(needle)?;
            Some((
                rest[..at].trim(),
                *position,
                rest[at + needle.len()..].trim(),
            ))
        })
    else {
        return "usage: move-rel <source> before|after|inside <target>".to_owned();
    };
    let find = |tree: &nodi_core::Tree, title: &str| {
        tree.iter_ordered()
            .into_iter()
            .find(|node| !node.trashed && node.title == title)
            .map(|node| node.id.clone())
    };
    let Some(source) = find(&app.tree, source_title) else {
        return "source not found".to_owned();
    };
    let Some(target) = find(&app.tree, target_title) else {
        return "target not found".to_owned();
    };
    match app.tree.move_relative(&source, &target, position) {
        Ok(()) => {
            app.tree_save();
            app.selected = Some(source);
            app.rebuild_tree();
            "ok".to_owned()
        }
        Err(err) => format!("error: {err}"),
    }
}
