//! Rich Edit (`RICHEDIT50W`, msftedit.dll) wrapper.
//!
//! `windows-sys` does not bind `richedit.h`, so the Rich Edit message numbers,
//! `CHARFORMAT2W` and `CHARRANGE` are declared here with the values from the
//! Windows SDK.

use std::mem::size_of;
use std::ptr;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::EM_SETMARGINS;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SendMessageW, ES_AUTOVSCROLL, ES_MULTILINE, ES_NOHIDESEL, ES_WANTRETURN, WM_COPY, WM_CUT,
    WM_PASTE, WS_CHILD, WS_TABSTOP, WS_VSCROLL,
};

use crate::app::{self, App};
use crate::win;
use crate::wnd::menu;
use crate::wnd::status;

pub const CLASS_NAME: &str = "RICHEDIT50W";

const WM_USER: u32 = 0x0400;

pub const EM_GETCHARFORMAT: u32 = WM_USER + 58;
pub const EM_SETCHARFORMAT: u32 = WM_USER + 68;
pub const EM_SETEVENTMASK: u32 = WM_USER + 69;
pub const EM_SETTEXTMODE: u32 = WM_USER + 89;
pub const EM_EXGETSEL: u32 = WM_USER + 52;
pub const EM_EXLIMITTEXT: u32 = WM_USER + 53;
pub const EM_EXLINEFROMCHAR: u32 = WM_USER + 54;
pub const EM_EXSETSEL: u32 = WM_USER + 55;
pub const EM_SETUNDOLIMIT: u32 = WM_USER + 82;
pub const EM_REDO: u32 = WM_USER + 84;
pub const EM_SETTYPOGRAPHYOPTIONS: u32 = WM_USER + 202;
/// Rich Edit keeps the Edit-control value for undo (`EM_UNDO`); there is no
/// `WM_USER`-based variant, unlike [`EM_REDO`].
pub const EM_UNDO: u32 = 0x00C7;

/// Standard edit-control message, also honoured by Rich Edit.
const EM_LINEINDEX_MSG: u32 = 0x00BB;
const EM_SETMODIFY: u32 = 0x00B9;

pub const SCF_SELECTION: usize = 0x0001;
pub const SCF_ALL: usize = 0x0004;
const SCF_DEFAULT: usize = 0x0000;

pub const CFM_BOLD: u32 = 0x0000_0001;
pub const CFM_ITALIC: u32 = 0x0000_0002;
pub const CFM_UNDERLINE: u32 = 0x0000_0004;
pub const CFM_STRIKEOUT: u32 = 0x0000_0008;
pub const CFM_FACE: u32 = 0x2000_0000;
pub const CFM_SIZE: u32 = 0x8000_0000;
pub const CFM_CHARSET: u32 = 0x0800_0000;

pub const CFE_BOLD: u32 = 0x0000_0001;
pub const CFE_ITALIC: u32 = 0x0000_0002;
pub const CFE_UNDERLINE: u32 = 0x0000_0004;
pub const CFE_STRIKEOUT: u32 = 0x0000_0008;

pub const ENM_CHANGE: usize = 0x0000_0001;
pub const ENM_SELCHANGE: usize = 0x0008_0000;
pub const ENM_LINK: usize = 0x0400_0000;

const TM_RICHTEXT: usize = 0x0002;
const TO_ADVANCEDTYPOGRAPHY: usize = 0x0001;
const EC_LEFTMARGIN: usize = 0x0001;
const EC_RIGHTMARGIN: usize = 0x0002;

/// `WM_COMMAND` notification code sent for `ENM_CHANGE`.
pub const EN_CHANGE_NOTIFY: u16 = 0x0300;
/// `WM_NOTIFY` code sent for `ENM_SELCHANGE`.
pub const EN_SELCHANGE_NOTIFY: u32 = 0x0702;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CharRange {
    pub cp_min: i32,
    pub cp_max: i32,
}

impl Default for CharRange {
    fn default() -> Self {
        // SAFETY: all-zero is a valid `CHARRANGE`.
        unsafe { std::mem::zeroed() }
    }
}

/// `CHARFORMAT2W` from `richedit.h` (116 bytes on both 32- and 64-bit).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CharFormat2W {
    pub cb_size: u32,
    pub dw_mask: u32,
    pub dw_effects: u32,
    pub y_height: i32,
    pub y_offset: i32,
    pub cr_text_color: u32,
    pub b_char_set: u8,
    pub b_pitch_and_family: u8,
    pub sz_face_name: [u16; 32],
    pub w_weight: u16,
    pub s_spacing: i16,
    pub cr_back_color: u32,
    pub lcid: u32,
    pub dw_cookie: u32,
    pub s_style: i16,
    pub w_kerning: u16,
    pub b_under_line_type: u8,
    pub b_animation: u8,
    pub b_rev_author: u8,
    pub b_underline_color: u8,
}

impl Default for CharFormat2W {
    fn default() -> Self {
        // SAFETY: all-zero is a valid `CHARFORMAT2W` (cbSize is set by callers).
        unsafe { std::mem::zeroed() }
    }
}

const _: () = assert!(size_of::<CharFormat2W>() == 116);

/// Creates a Rich Edit child for a new tab. Returns `None` if Windows refused
/// to create the window (e.g. msftedit.dll failed to load).
pub fn create(app: &App) -> Option<HWND> {
    let class = win::wide(CLASS_NAME);
    let style = WS_CHILD
        | WS_TABSTOP
        | WS_VSCROLL
        | (ES_MULTILINE | ES_AUTOVSCROLL | ES_NOHIDESEL | ES_WANTRETURN) as u32;

    // SAFETY: `class` outlives the call; all handles belong to `app`.
    let window = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::CreateWindowExW(
            0,
            class.as_ptr(),
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
    if window.is_null() {
        return None;
    }

    // SAFETY: `window` is a freshly created Rich Edit control.
    unsafe {
        SendMessageW(window, EM_SETTEXTMODE, TM_RICHTEXT, 0);
        SendMessageW(window, EM_SETUNDOLIMIT, 100, 0);
        SendMessageW(window, EM_SETTYPOGRAPHYOPTIONS, TO_ADVANCEDTYPOGRAPHY, 0);
        SendMessageW(window, EM_EXLIMITTEXT, 0, 0x7FFF_FFFF);
        SendMessageW(
            window,
            EM_SETEVENTMASK,
            0,
            (ENM_CHANGE | ENM_SELCHANGE | ENM_LINK) as isize,
        );
        apply_default_format(window);

        let margin = app.scale(6);
        SendMessageW(
            window,
            EM_SETMARGINS,
            EC_LEFTMARGIN | EC_RIGHTMARGIN,
            ((margin as isize) << 16) | margin as isize,
        );
        SendMessageW(window, EM_SETMODIFY, 0, 0);
    }

    Some(window)
}

fn apply_default_format(window: HWND) {
    let mut format = CharFormat2W {
        cb_size: size_of::<CharFormat2W>() as u32,
        dw_mask: CFM_FACE | CFM_SIZE | CFM_CHARSET,
        y_height: 220, // 11 pt, in twips
        b_char_set: 1, // DEFAULT_CHARSET
        ..CharFormat2W::default()
    };
    let face: Vec<u16> = "Segoe UI".encode_utf16().collect();
    for (slot, unit) in format.sz_face_name.iter_mut().zip(face) {
        *slot = unit;
    }
    format.sz_face_name[31] = 0;

    // SAFETY: the format lives on the stack for the duration of the call.
    unsafe {
        SendMessageW(
            window,
            EM_SETCHARFORMAT,
            SCF_ALL | SCF_DEFAULT,
            &format as *const CharFormat2W as isize,
        );
    }
}

pub fn line_col(editor: HWND) -> (i32, i32) {
    let mut range = CharRange::default();
    // SAFETY: `range` is a valid CHARRANGE for the duration of the calls.
    unsafe {
        SendMessageW(
            editor,
            EM_EXGETSEL,
            0,
            &mut range as *mut CharRange as isize,
        );
        let line = SendMessageW(editor, EM_EXLINEFROMCHAR, 0, range.cp_min as isize) as i32;
        let line = line.max(0);
        let line_start = SendMessageW(editor, EM_LINEINDEX_MSG, line as usize, 0) as i32;
        let column = range.cp_min - line_start.max(0) + 1;
        (line + 1, column.max(1))
    }
}

/// Runs an Edit-menu command against the active editor.
pub fn command(app: &mut App, id: u16) {
    let Some(editor) = app.active_editor() else {
        status::set_message(app, "No note open");
        return;
    };

    // SAFETY: `editor` is a live Rich Edit control owned by this process.
    unsafe {
        match id {
            menu::IDM_EDIT_UNDO => {
                SendMessageW(editor, EM_UNDO, 0, 0);
            }
            menu::IDM_EDIT_REDO => {
                SendMessageW(editor, EM_REDO, 0, 0);
            }
            menu::IDM_EDIT_CUT => {
                SendMessageW(editor, WM_CUT, 0, 0);
            }
            menu::IDM_EDIT_COPY => {
                SendMessageW(editor, WM_COPY, 0, 0);
            }
            menu::IDM_EDIT_PASTE => {
                SendMessageW(editor, WM_PASTE, 0, 0);
            }
            menu::IDM_EDIT_SELECT_ALL => {
                let range = CharRange {
                    cp_min: 0,
                    cp_max: -1,
                };
                SendMessageW(editor, EM_EXSETSEL, 0, &range as *const CharRange as isize);
            }
            _ => {}
        }
        SetFocus(editor);
    }

    status::update_caret(app);
}

/// Toggles bold/italic/underline/strikethrough on the current selection (or on
/// the insertion point when there is no selection).
pub fn toggle_effect(app: &mut App, id: u16) {
    let (mask, effect) = match id {
        menu::IDM_FORMAT_BOLD => (CFM_BOLD, CFE_BOLD),
        menu::IDM_FORMAT_ITALIC => (CFM_ITALIC, CFE_ITALIC),
        menu::IDM_FORMAT_UNDERLINE => (CFM_UNDERLINE, CFE_UNDERLINE),
        menu::IDM_FORMAT_STRIKEOUT => (CFM_STRIKEOUT, CFE_STRIKEOUT),
        _ => return,
    };

    let Some(editor) = app.active_editor() else {
        status::set_message(app, "No note open");
        return;
    };

    let mut format = CharFormat2W {
        cb_size: size_of::<CharFormat2W>() as u32,
        dw_mask: mask,
        ..CharFormat2W::default()
    };

    // SAFETY: `format` is stack allocated; the control is our child window.
    unsafe {
        SendMessageW(
            editor,
            EM_GETCHARFORMAT,
            SCF_SELECTION,
            &mut format as *mut CharFormat2W as isize,
        );
        if format.dw_effects & effect != 0 {
            format.dw_effects &= !effect;
        } else {
            format.dw_effects |= effect;
        }
        format.dw_mask = mask;
        SendMessageW(
            editor,
            EM_SETCHARFORMAT,
            SCF_SELECTION,
            &format as *const CharFormat2W as isize,
        );
        SetFocus(editor);
    }

    app::mark_dirty(app, editor);
}
