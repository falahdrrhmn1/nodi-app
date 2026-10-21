//! Toolbar with the standard system image list plus text buttons for B/I/U.

use std::mem::size_of;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::UI::Controls::{
    BTNS_AUTOSIZE, BTNS_BUTTON, BTNS_SEP, BTNS_SHOWTEXT, CCS_NODIVIDER, CCS_NOPARENTALIGN,
    CCS_NORESIZE, IDB_STD_SMALL_COLOR, I_IMAGENONE, STD_COPY, STD_CUT, STD_FILENEW, STD_FILESAVE,
    STD_PASTE, STD_REDOW, STD_UNDO, TBBUTTON, TBSTATE_ENABLED, TBSTYLE_EX_MIXEDBUTTONS,
    TBSTYLE_FLAT, TBSTYLE_LIST, TBSTYLE_TOOLTIPS, TB_ADDBUTTONSW, TB_ADDSTRINGW, TB_AUTOSIZE,
    TB_BUTTONSTRUCTSIZE, TB_LOADIMAGES, TB_SETEXTENDEDSTYLE, TOOLBARCLASSNAMEW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetWindowRect, SendMessageW, WS_CHILD, WS_VISIBLE,
};

use crate::app::App;
use crate::wnd::menu::{
    IDM_EDIT_COPY, IDM_EDIT_CUT, IDM_EDIT_PASTE, IDM_EDIT_REDO, IDM_EDIT_UNDO, IDM_FILE_NEW,
    IDM_FILE_SAVE, IDM_FORMAT_BOLD, IDM_FORMAT_ITALIC, IDM_FORMAT_UNDERLINE,
};

/// `HINST_COMMCTRL`, absent from windows-sys.
const HINST_COMMCTRL: isize = -1;

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
        for label in ["B", "I", "U"] {
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

        let buttons = [
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
            text(0, IDM_FORMAT_BOLD),
            text(1, IDM_FORMAT_ITALIC),
            text(2, IDM_FORMAT_UNDERLINE),
        ];
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
