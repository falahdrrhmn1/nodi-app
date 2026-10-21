//! Search inside the active note with the standard Windows Find / Replace
//! dialogs (modeless, like Notepad), F3 / Shift+F3 to repeat, and
//! "Search in All Notes" over the plain-text sidecars.

use std::cell::{Cell, RefCell};
use std::mem::size_of;
use std::ptr;
use std::time::Duration;

use nodi_core::search;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::Dialogs::{
    FindTextW, ReplaceTextW, FINDREPLACEW, FR_DIALOGTERM, FR_DOWN, FR_FINDNEXT, FR_MATCHCASE,
    FR_REPLACE, FR_REPLACEALL, FR_WHOLEWORD,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, RegisterWindowMessageW, SendMessageW,
};

use crate::app::App;
use crate::win;
use crate::wnd::editor::{self, CharRange};
use crate::wnd::form::{Field, Form};
use crate::wnd::{dialogs, status};

const EM_FINDTEXTEXW: u32 = 0x0400 + 124;
const EM_SCROLLCARET: u32 = 0x00B7;
const BUFFER: usize = 512;

/// `FINDREPLACEW` plus the buffers it points into; boxed so the addresses
/// stay put while the modeless dialog is open.
struct Data {
    request: FINDREPLACEW,
    find: [u16; BUFFER],
    replace: [u16; BUFFER],
}

thread_local! {
    static DIALOG: Cell<HWND> = const { Cell::new(ptr::null_mut()) };
    static DATA: RefCell<Option<Box<Data>>> = const { RefCell::new(None) };
    /// Last search: text, match case, whole word.
    static LAST: RefCell<Option<(String, bool, bool)>> = const { RefCell::new(None) };
}

/// The open Find/Replace dialog (for `IsDialogMessage` in the message loop).
pub fn dialog() -> HWND {
    DIALOG.with(Cell::get)
}

/// Registered `FINDMSGSTRING` message id.
pub fn message_id() -> u32 {
    thread_local! {
        static ID: Cell<u32> = const { Cell::new(0) };
    }
    ID.with(|id| {
        if id.get() == 0 {
            let name = win::wide("commdlg_FindReplace");
            // SAFETY: NUL-terminated name.
            id.set(unsafe { RegisterWindowMessageW(name.as_ptr()) });
        }
        id.get()
    })
}

/// Search > Find... / Replace...
pub fn open(app: &mut App, replace: bool) {
    let current = dialog();
    if !current.is_null() {
        // SAFETY: closing our own modeless dialog; FR_DIALOGTERM cleans up.
        unsafe {
            DestroyWindow(current);
        }
        DIALOG.with(|cell| cell.set(ptr::null_mut()));
    }
    let seed = app
        .active_editor()
        .map(|editor| {
            let (from, to) = editor::selection(editor);
            editor::text_range(editor, from, to)
        })
        .filter(|text| !text.is_empty() && !text.contains('\r'))
        .or_else(|| LAST.with(|last| last.borrow().as_ref().map(|(text, _, _)| text.clone())))
        .unwrap_or_default();

    let mut data = Box::new(Data {
        // SAFETY: all-zero is a valid FINDREPLACEW before it is filled in.
        request: unsafe { std::mem::zeroed() },
        find: [0; BUFFER],
        replace: [0; BUFFER],
    });
    for (slot, unit) in data
        .find
        .iter_mut()
        .zip(seed.encode_utf16().take(BUFFER - 1))
    {
        *slot = unit;
    }
    data.request.lStructSize = size_of::<FINDREPLACEW>() as u32;
    data.request.hwndOwner = app.hwnd;
    data.request.Flags = FR_DOWN;
    data.request.lpstrFindWhat = data.find.as_mut_ptr();
    data.request.wFindWhatLen = BUFFER as u16;
    data.request.lpstrReplaceWith = data.replace.as_mut_ptr();
    data.request.wReplaceWithLen = BUFFER as u16;

    // SAFETY: `data` is boxed and kept alive in DATA until FR_DIALOGTERM.
    let handle = unsafe {
        if replace {
            ReplaceTextW(&mut data.request)
        } else {
            FindTextW(&mut data.request)
        }
    };
    if handle.is_null() {
        return;
    }
    DATA.with(|slot| *slot.borrow_mut() = Some(data));
    DIALOG.with(|cell| cell.set(handle));
}

/// Handles the registered `FINDMSGSTRING` message from the dialog.
pub fn on_message(app: &mut App) {
    let Some((flags, find, replace)) = DATA.with(|slot| {
        slot.borrow().as_ref().map(|data| {
            (
                data.request.Flags,
                from_buffer(&data.find),
                from_buffer(&data.replace),
            )
        })
    }) else {
        return;
    };
    if flags & FR_DIALOGTERM != 0 {
        DIALOG.with(|cell| cell.set(ptr::null_mut()));
        DATA.with(|slot| *slot.borrow_mut() = None);
        if let Some(editor) = app.active_editor() {
            // SAFETY: focusing our own editor.
            unsafe {
                SetFocus(editor);
            }
        }
        return;
    }
    let case = flags & FR_MATCHCASE != 0;
    let word = flags & FR_WHOLEWORD != 0;
    LAST.with(|last| *last.borrow_mut() = Some((find.clone(), case, word)));
    let Some(editor) = app.active_editor() else {
        return;
    };
    if flags & FR_REPLACEALL != 0 {
        let count = replace_all(editor, &find, &replace, case, word);
        status::set_message(app, &format!("Replaced {count} occurrence(s)"));
        if count > 0 {
            app.mark_dirty(editor);
        }
    } else if flags & FR_REPLACE != 0 {
        if selection_matches(editor, &find, case) {
            editor::replace_selection(editor, &replace, true);
            app.mark_dirty(editor);
        }
        report(app, find_next(editor, &find, true, case, word));
    } else if flags & FR_FINDNEXT != 0 {
        let down = flags & FR_DOWN != 0;
        report(app, find_next(editor, &find, down, case, word));
    }
}

/// F3 / Shift+F3.
pub fn find_again(app: &mut App, forward: bool) {
    let last = LAST.with(|last| last.borrow().clone());
    let Some((text, case, word)) = last else {
        open(app, false);
        return;
    };
    if let Some(editor) = app.active_editor() {
        report(app, find_next(editor, &text, forward, case, word));
    }
}

fn report(app: &App, outcome: Outcome) {
    let message = match outcome {
        Outcome::Found => "Ready",
        Outcome::Wrapped => "Search wrapped around the note",
        Outcome::NotFound => "Text not found",
    };
    status::set_message(app, message);
}

fn from_buffer(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

#[repr(C)]
struct FindTextEx {
    range: CharRange,
    text: *const u16,
    found: CharRange,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Found,
    Wrapped,
    NotFound,
}

fn search(editor: HWND, text: &[u16], from: i32, to: i32, flags: u32) -> Option<CharRange> {
    let mut request = FindTextEx {
        range: CharRange {
            cp_min: from,
            cp_max: to,
        },
        text: text.as_ptr(),
        found: CharRange::default(),
    };
    // SAFETY: `request` and `text` (NUL terminated) outlive the call.
    let index = unsafe {
        SendMessageW(
            editor,
            EM_FINDTEXTEXW,
            flags as usize,
            &mut request as *mut FindTextEx as isize,
        )
    };
    (index >= 0).then_some(request.found)
}

/// Selects the next (or previous) occurrence, wrapping around once.
pub fn find_next(editor: HWND, text: &str, down: bool, case: bool, word: bool) -> Outcome {
    if text.is_empty() {
        return Outcome::NotFound;
    }
    let wide = win::wide(text);
    let mut flags = 0;
    if down {
        flags |= FR_DOWN;
    }
    if case {
        flags |= FR_MATCHCASE;
    }
    if word {
        flags |= FR_WHOLEWORD;
    }
    let (from, to) = editor::selection(editor);
    let length = editor::text_length(editor);
    let (first, wrap) = if down {
        ((to, -1), (0, -1))
    } else {
        ((from, 0), (length, 0))
    };
    let (found, wrapped) = match search(editor, &wide, first.0, first.1, flags) {
        Some(range) => (Some(range), false),
        None => (search(editor, &wide, wrap.0, wrap.1, flags), true),
    };
    match found {
        Some(range) => {
            editor::set_selection(editor, range.cp_min, range.cp_max);
            // SAFETY: plain message to our editor.
            unsafe {
                SendMessageW(editor, EM_SCROLLCARET, 0, 0);
            }
            if wrapped {
                Outcome::Wrapped
            } else {
                Outcome::Found
            }
        }
        None => Outcome::NotFound,
    }
}

fn selection_matches(editor: HWND, text: &str, case: bool) -> bool {
    let (from, to) = editor::selection(editor);
    let selected = editor::text_range(editor, from, to);
    if case {
        selected == text
    } else {
        selected.to_lowercase() == text.to_lowercase()
    }
}

/// Replaces every occurrence; returns how many were replaced.
pub fn replace_all(editor: HWND, find: &str, replace: &str, case: bool, word: bool) -> usize {
    if find.is_empty() {
        return 0;
    }
    let wide = win::wide(find);
    let mut flags = FR_DOWN;
    if case {
        flags |= FR_MATCHCASE;
    }
    if word {
        flags |= FR_WHOLEWORD;
    }
    let replacement_len = replace.encode_utf16().count() as i32;
    let mut position = 0;
    let mut count = 0;
    while let Some(range) = search(editor, &wide, position, -1, flags) {
        editor::set_selection(editor, range.cp_min, range.cp_max);
        editor::replace_selection(editor, replace, true);
        position = range.cp_min + replacement_len;
        count += 1;
        if count > 100_000 {
            break;
        }
    }
    count
}

/// Search > Search in All Notes...
pub fn search_all(app: &mut App) {
    let last = LAST.with(|last| last.borrow().as_ref().map(|(text, _, _)| text.clone()));
    let Some(values) = Form::new("Search in All Notes")
        .field(Field::new("Find text:", last.unwrap_or_default()))
        .show(app.hwnd)
    else {
        return;
    };
    let query = values[0].trim().to_owned();
    if query.is_empty() {
        return;
    }
    // Make sure the sidecars on disk reflect what is in the editors.
    app.flush_all();
    if let Some(writer) = &app.writer {
        let _ = writer.flush_blocking(Duration::from_secs(3));
    }
    let Some(data) = app.data.clone() else {
        return;
    };
    let results = search::search_text(&data, &app.tree, &query);
    if results.is_empty() {
        dialogs::info(app.hwnd, &format!("No note contains \"{query}\"."));
        return;
    }
    let entries: Vec<_> = results
        .iter()
        .map(|(id, hits)| {
            let title = app
                .tree
                .get(id)
                .map(|node| node.title.clone())
                .unwrap_or_default();
            (
                id.clone(),
                format!(
                    "{title}    ({hits} match{})",
                    if *hits == 1 { "" } else { "es" }
                ),
            )
        })
        .collect();
    let caption = format!("{} note(s) contain \"{query}\"", results.len());
    if let Some(id) = dialogs::pick_note(app.hwnd, &caption, "Filter results:", &entries) {
        app.selected = Some(id.clone());
        app.open_note(id);
        LAST.with(|last| *last.borrow_mut() = Some((query.clone(), false, false)));
        if let Some(editor) = app.active_editor() {
            editor::set_selection(editor, 0, 0);
            report(app, find_next(editor, &query, true, false, false));
        }
    }
}
