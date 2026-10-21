//! Toolbar with the standard system image list plus text buttons for B/I/U.

use std::mem::size_of;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::UI::Controls::{
    BTNS_AUTOSIZE, BTNS_BUTTON, BTNS_SEP, BTNS_SHOWTEXT, CCS_NODIVIDER, CCS_NOPARENTALIGN,
    CCS_NORESIZE, IDB_STD_SMALL_COLOR, I_IMAGENONE, NMTTDISPINFOW, STD_COPY, STD_CUT, STD_FILENEW,
    STD_FILESAVE, STD_FIND, STD_PASTE, STD_REDOW, STD_UNDO, TBBUTTON, TBSTATE_ENABLED,
    TBSTYLE_EX_MIXEDBUTTONS, TBSTYLE_FLAT, TBSTYLE_LIST, TBSTYLE_TOOLTIPS, TB_ADDBUTTONSW,
    TB_ADDSTRINGW, TB_AUTOSIZE, TB_BUTTONSTRUCTSIZE, TB_LOADIMAGES, TB_SETEXTENDEDSTYLE,
    TOOLBARCLASSNAMEW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetWindowRect, SendMessageW, WS_CHILD, WS_VISIBLE,
};

use crate::app::App;
use crate::wnd::menu::*;

/// `HINST_COMMCTRL`, absent from windows-sys.
const HINST_COMMCTRL: isize = -1;

/// Text buttons, in order: (label, command).
const TEXT_BUTTONS: [(&str, u16); 13] = [
    ("B", IDM_FORMAT_BOLD),
    ("I", IDM_FORMAT_ITALIC),
    ("U", IDM_FORMAT_UNDERLINE),
    ("S", IDM_FORMAT_STRIKEOUT),
    ("H1", IDM_FORMAT_H1),
    ("H2", IDM_FORMAT_H2),
    ("\u{2022} List", IDM_FORMAT_BULLETS),
    ("1. List", IDM_FORMAT_NUMBERS),
    ("Image", IDM_INSERT_IMAGE),
    ("Table", IDM_INSERT_TABLE),
    ("Link", IDM_INSERT_LINK),
    ("Code", IDM_INSERT_CODE_BLOCK),
    ("\u{2211} Formula", IDM_INSERT_FORMULA),
];

/// A separator goes in front of these text buttons.
const GROUP_STARTS: [u16; 2] = [IDM_FORMAT_H1, IDM_INSERT_IMAGE];

/// `TTN_GETDISPINFOW` (`TTN_FIRST - 10`).
pub const TTN_GETDISPINFOW: u32 = 0u32.wrapping_sub(530);

/// Tooltip text for a toolbar command.
fn tip(command: u16) -> &'static str {
    match command {
        IDM_FILE_NEW => "New note (Ctrl+N)",
        IDM_FILE_SAVE => "Save now (Ctrl+S)",
        IDM_EDIT_CUT => "Cut (Ctrl+X)",
        IDM_EDIT_COPY => "Copy (Ctrl+C)",
        IDM_EDIT_PASTE => "Paste (Ctrl+V)",
        IDM_EDIT_UNDO => "Undo (Ctrl+Z)",
        IDM_EDIT_REDO => "Redo (Ctrl+Y)",
        IDM_FORMAT_BOLD => "Bold (Ctrl+B)",
        IDM_FORMAT_ITALIC => "Italic (Ctrl+I)",
        IDM_FORMAT_UNDERLINE => "Underline (Ctrl+U)",
        IDM_FORMAT_STRIKEOUT => "Strikethrough (Alt+Shift+5)",
        IDM_FORMAT_H1 => "Heading 1 (Ctrl+Alt+1)",
        IDM_FORMAT_H2 => "Heading 2 (Ctrl+Alt+2)",
        IDM_FORMAT_BULLETS => "Bulleted list (Ctrl+Shift+8)",
        IDM_FORMAT_NUMBERS => "Numbered list (Ctrl+Shift+7)",
        IDM_INSERT_IMAGE => "Insert image (Ctrl+Shift+I) — or paste a screenshot",
        IDM_INSERT_TABLE => "Insert table (Ctrl+Shift+T)",
        IDM_INSERT_LINK => "Insert link (Ctrl+K)",
        IDM_INSERT_CODE_BLOCK => "Code block (Ctrl+Shift+C)",
        IDM_INSERT_FORMULA => "Formula (Alt+=)",
        IDM_SEARCH_FIND => "Find in note (Ctrl+F)",
        _ => "",
    }
}

/// Fills an `NMTTDISPINFOW` for a toolbar button.
///
/// # Safety
/// `lparam` must be the `NMTTDISPINFOW` pointer of a `TTN_GETDISPINFOW`.
pub unsafe fn tooltip(lparam: isize) {
    let info = &mut *(lparam as *mut NMTTDISPINFOW);
    let text = tip(info.hdr.idFrom as u16);
    info.szText = [0; 80];
    for (slot, unit) in info.szText.iter_mut().zip(text.encode_utf16().take(79)) {
        *slot = unit;
    }
    info.lpszText = info.szText.as_mut_ptr();
}

pub fn create(app: &App) -> HWND {
    let style = WS_CHILD
        | WS_VISIBLE
        | TBSTYLE_FLAT
        | TBSTYLE_LIST
        | TBSTYLE_TOOLTIPS
        | (CCS_NODIVIDER | CCS_NOPARENTALIGN | CCS_NORESIZE) as u32;

    // SAFETY: constant class name and parent handles owned by `app`.
    let toolbar = unsafe {
        CreateWindowExW(
            0,
            TOOLBARCLASSNAMEW,
            ptr::null(),
            style,
            0,
            0,
            0,
            0,
            app.hwnd,
            ptr::null_mut(),
            app.hinstance,
            ptr::null(),
        )
    };
    if toolbar.is_null() {
        return toolbar;
    }

    // SAFETY: `toolbar` is a live toolbar control; every message below is a
    // documented toolbar message with stack/static arguments.
    unsafe {
        SendMessageW(toolbar, TB_BUTTONSTRUCTSIZE, size_of::<TBBUTTON>(), 0);
        SendMessageW(
            toolbar,
            TB_SETEXTENDEDSTYLE,
            0,
            TBSTYLE_EX_MIXEDBUTTONS as isize,
        );
        SendMessageW(
            toolbar,
            TB_LOADIMAGES,
            IDB_STD_SMALL_COLOR as usize,
            HINST_COMMCTRL,
        );

        let mut strings = Vec::new();
        for label in TEXT_BUTTONS.iter().map(|(label, _)| *label) {
            strings.extend(label.encode_utf16());
            strings.push(0);
        }
        strings.push(0);
        let first_text = SendMessageW(toolbar, TB_ADDSTRINGW, 0, strings.as_ptr() as isize) as i32;

        let icon = |bitmap: u32, command: u16| TBBUTTON {
            iBitmap: bitmap as i32,
            idCommand: command as i32,
            fsState: TBSTATE_ENABLED as u8,
            fsStyle: (BTNS_BUTTON | BTNS_AUTOSIZE) as u8,
            bReserved: [0; 6],
            dwData: 0,
            iString: 0,
        };
        let text = |index: i32, command: u16| TBBUTTON {
            iBitmap: I_IMAGENONE,
            idCommand: command as i32,
            fsState: TBSTATE_ENABLED as u8,
            fsStyle: (BTNS_BUTTON | BTNS_AUTOSIZE | BTNS_SHOWTEXT) as u8,
            bReserved: [0; 6],
            dwData: 0,
            iString: (first_text + index) as isize,
        };
        let separator = TBBUTTON {
            iBitmap: 0,
            idCommand: 0,
            fsState: 0,
            fsStyle: BTNS_SEP as u8,
            bReserved: [0; 6],
            dwData: 0,
            iString: 0,
        };

        let mut buttons = vec![
            icon(STD_FILENEW, IDM_FILE_NEW),
            icon(STD_FILESAVE, IDM_FILE_SAVE),
            separator,
            icon(STD_CUT, IDM_EDIT_CUT),
            icon(STD_COPY, IDM_EDIT_COPY),
            icon(STD_PASTE, IDM_EDIT_PASTE),
            separator,
            icon(STD_UNDO, IDM_EDIT_UNDO),
            icon(STD_REDOW, IDM_EDIT_REDO),
            separator,
        ];
        for (index, (_, command)) in TEXT_BUTTONS.iter().enumerate() {
            if GROUP_STARTS.contains(command) {
                buttons.push(separator);
            }
            buttons.push(text(index as i32, *command));
        }
        buttons.push(separator);
        buttons.push(icon(STD_FIND, IDM_SEARCH_FIND));
        SendMessageW(
            toolbar,
            TB_ADDBUTTONSW,
            buttons.len(),
            buttons.as_ptr() as isize,
        );
        SendMessageW(toolbar, TB_AUTOSIZE, 0, 0);
    }

    toolbar
}

/// Recomputes the toolbar height after a font/DPI change.
pub fn refresh_height(app: &mut App) {
    if app.toolbar.is_null() {
        return;
    }
    // SAFETY: `toolbar` is a live toolbar window.
    unsafe {
        SendMessageW(app.toolbar, TB_AUTOSIZE, 0, 0);
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(app.toolbar, &mut rect) != 0 {
            let height = rect.bottom - rect.top;
            app.toolbar_height = height.max(app.scale(28));
        }
    }
}
