//! Keyboard behaviour of the note editor, layered over Rich Edit with a window
//! subclass:
//!
//! * Enter keeps the indentation (tabs/spaces) of the current line, like
//!   Notepad++. Shift+Enter is a plain line break.
//! * Tab on a multi-line selection indents every line; Shift+Tab outdents the
//!   current line or every selected line. Backspace removes one indent level
//!   when the indent is a tab (Rich Edit's own behaviour).
//! * Ctrl+V / Shift+Insert paste a clipboard picture (Rich Edit alone only
//!   pastes text/RTF), Ctrl+Shift+V pastes plain text.
//!
//! Inside tables and bulleted/numbered lists Enter and Tab keep Rich Edit's
//! native meaning (new list item, next cell).

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_INSERT, VK_MENU, VK_RETURN, VK_SHIFT,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    PostMessageW, WM_CHAR, WM_COMMAND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_PASTE,
};

use crate::wnd::{editor, format, image};

const SUBCLASS_ID: usize = 0x4E4F_4445; // 'NODE'

thread_local! {
    /// Set when a Ctrl+V keydown was fully handled here, so the WM_CHAR 0x16
    /// that `TranslateMessage` generates for it is swallowed too.
    static SWALLOW_SYN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn install(editor: HWND, frame: HWND) {
    // SAFETY: `editor` is our Rich Edit; the frame handle is the ref data.
    unsafe {
        SetWindowSubclass(editor, Some(subclass), SUBCLASS_ID, frame as usize);
    }
}

fn key_down(key: u16) -> bool {
    // SAFETY: plain key state query.
    unsafe { GetKeyState(i32::from(key)) < 0 }
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    frame: usize,
) -> LRESULT {
    let ctrl = key_down(VK_CONTROL);
    let shift = key_down(VK_SHIFT);
    let alt = key_down(VK_MENU);
    match msg {
        WM_CHAR => {
            let ch = wparam as u32;
            if ch == 0x16 && SWALLOW_SYN.with(|flag| flag.replace(false)) {
                return 0;
            }
            // Enter is handled on WM_KEYDOWN (Rich Edit inserts the paragraph
            // there and ignores the WM_CHAR CR); Tab arrives as WM_CHAR.
            if ch == 0x09 && !ctrl && !alt && tab_key(hwnd, shift) {
                return 0;
            }
        }
        WM_KEYDOWN => {
            let key = wparam as u16;
            if key == VK_RETURN && !ctrl && !alt && !shift && newline_keep_indent(hwnd) {
                return 0;
            }
            let paste =
                (ctrl && !alt && key == u16::from(b'V')) || (shift && !ctrl && key == VK_INSERT);
            if paste {
                let plain = ctrl && shift;
                if paste_special(hwnd, frame as HWND, plain) {
                    SWALLOW_SYN.with(|flag| flag.set(ctrl));
                    return 0;
                }
            }
        }
        WM_PASTE => {
            if paste_special(hwnd, frame as HWND, false) {
                return 0;
            }
        }
        WM_LBUTTONDBLCLK => {
            // Double-clicking a formula opens it for editing.
            let result = DefSubclassProc(hwnd, msg, wparam, lparam);
            if crate::wnd::formula::selected_latex(hwnd).is_some() {
                PostMessageW(
                    frame as HWND,
                    WM_COMMAND,
                    usize::from(crate::wnd::menu::IDM_INSERT_FORMULA),
                    0,
                );
            }
            return result;
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// Enter: new paragraph that starts with the same leading whitespace as the
/// current one. Returns false to let Rich Edit handle the key natively.
pub fn newline_keep_indent(editor: HWND) -> bool {
    // A code block is a one-cell table: indent there like a code editor.
    if editor::in_table_or_list(editor) && !format::in_code(editor) {
        return false;
    }
    // Enter at the end of a heading continues with body text, like Word.
    if format::heading_level_at(editor) > 0 {
        let (from, _) = editor::selection(editor);
        if from == editor::paragraph_end(editor, from) {
            editor::replace_selection(editor, "\r", true);
            format::heading(editor, 0);
            return true;
        }
    }
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let head = editor::text_range(editor, start, from);
    let indent: String = head
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect();
    // In a code block Rich Edit's own Enter at the end of the cell would add
    // a table row; always insert a paragraph inside the cell instead.
    if indent.is_empty() && !format::in_code(editor) {
        return false;
    }
    format::insert_text(editor, &format!("\r{indent}"));
    true
}

/// Tab / Shift+Tab. Returns false to let Rich Edit insert a plain tab.
pub fn tab_key(editor: HWND, outdent: bool) -> bool {
    let in_code_block = editor::in_table(editor) && format::in_code(editor);
    if editor::in_table(editor) && !in_code_block {
        return false;
    }
    let (from, to) = editor::selection(editor);
    let first = editor::paragraph_start(editor, from);
    // A selection that ends right at the start of a line does not include it.
    let last_pos = if to > from && editor::paragraph_start(editor, to) == to {
        to - 1
    } else {
        to
    };
    let multi_line = editor::paragraph_end(editor, first) < last_pos;
    if !outdent && !multi_line {
        if in_code_block {
            // Rich Edit would jump to the next cell (or add a row) instead.
            format::insert_text(editor, "\t");
            return true;
        }
        return false;
    }

    // Paragraph starts inside the block, edited back to front so earlier
    // offsets stay valid.
    let block_end = editor::paragraph_end(editor, last_pos.max(first));
    let units = editor::text_units(editor, first, block_end);
    let mut starts = vec![first];
    for (index, unit) in units.iter().enumerate() {
        if *unit == 0x000D {
            starts.push(first + index as i32 + 1);
        }
    }

    let mut shift_from = 0;
    let mut shift_total = 0;
    for &start in starts.iter().rev() {
        let delta = if outdent {
            let head = editor::text_units(editor, start, start + 4);
            let remove = match head.first() {
                Some(0x0009) => 1,
                _ => head.iter().take_while(|unit| **unit == 0x0020).count() as i32,
            };
            if remove == 0 {
                continue;
            }
            editor::set_selection(editor, start, start + remove);
            editor::replace_selection(editor, "", true);
            -remove
        } else {
            editor::set_selection(editor, start, start);
            format::insert_text(editor, "\t");
            1
        };
        shift_total += delta;
        if start == first {
            shift_from = delta;
        }
    }

    if multi_line {
        let new_end = editor::paragraph_end(editor, (block_end + shift_total).max(first));
        editor::set_selection(editor, first, new_end);
    } else {
        let caret = (from + shift_from).max(first);
        editor::set_selection(editor, caret, caret);
    }
    true
}

/// Handles the paste cases Rich Edit cannot: a picture on the clipboard, and
/// Ctrl+Shift+V (plain text). Returns false for an ordinary paste.
fn paste_special(editor: HWND, frame: HWND, plain: bool) -> bool {
    if plain {
        if let Some(text) = image::clipboard_text(frame) {
            editor::replace_selection(
                editor,
                &text.replace("\r\n", "\r").replace('\n', "\r"),
                true,
            );
        }
        return true;
    }
    if !image::clipboard_is_image_only() {
        return false;
    }
    match image::clipboard_image(frame) {
        Some(bytes) => {
            if let Err(message) = image::insert_bytes(editor, &bytes) {
                crate::wnd::dialogs::error(frame, &message);
            }
            true
        }
        None => false,
    }
}
