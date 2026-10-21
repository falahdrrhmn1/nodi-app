//! Menu bar, command ids and the accelerator table.

use std::ptr;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_TAB;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CheckMenuItem, CreateAcceleratorTableW, CreateMenu, CreatePopupMenu, PostMessageW,
    FCONTROL, FSHIFT, FVIRTKEY, HACCEL, HMENU, MF_BYCOMMAND, MF_CHECKED, MF_GRAYED, MF_POPUP,
    MF_SEPARATOR, MF_STRING, MF_UNCHECKED, WM_CLOSE,
};

use crate::app::{self, App};
use crate::win;
use crate::wnd::{dialogs, editor, status};

pub const IDM_FILE_NEW: u16 = 100;
pub const IDM_FILE_SAVE: u16 = 101;
pub const IDM_FILE_CLOSE_TAB: u16 = 102;
pub const IDM_FILE_EXIT: u16 = 103;

pub const IDM_EDIT_UNDO: u16 = 110;
pub const IDM_EDIT_REDO: u16 = 111;
pub const IDM_EDIT_CUT: u16 = 112;
pub const IDM_EDIT_COPY: u16 = 113;
pub const IDM_EDIT_PASTE: u16 = 114;
pub const IDM_EDIT_SELECT_ALL: u16 = 115;

pub const IDM_FORMAT_BOLD: u16 = 120;
pub const IDM_FORMAT_ITALIC: u16 = 121;
pub const IDM_FORMAT_UNDERLINE: u16 = 122;
pub const IDM_FORMAT_STRIKEOUT: u16 = 123;

pub const IDM_VIEW_TOOLBAR: u16 = 130;
pub const IDM_VIEW_STATUS: u16 = 131;

pub const IDM_HELP_ABOUT: u16 = 140;

pub const IDM_TAB_NEXT: u16 = 150;
pub const IDM_TAB_PREV: u16 = 151;

/// Builds the menu bar. Items that are not implemented yet are visibly marked
/// and disabled, per `docs/ARCHITECTURE.md`.
pub fn create_menu() -> HMENU {
    // SAFETY: all strings are temporaries that live for the duration of each
    // call; menu handles are owned by us and attached to the frame window.
    unsafe {
        let bar = CreateMenu();
        if bar.is_null() {
            return ptr::null_mut();
        }

        let file = CreatePopupMenu();
        if !file.is_null() {
            append(file, MF_STRING, IDM_FILE_NEW, "&New Note\tCtrl+N");
            append(
                file,
                MF_STRING | MF_GRAYED,
                IDM_FILE_SAVE,
                "&Save (not yet)\tCtrl+S",
            );
            append(file, MF_STRING, IDM_FILE_CLOSE_TAB, "&Close Tab\tCtrl+W");
            AppendMenuW(file, MF_SEPARATOR, 0, ptr::null());
            append(file, MF_STRING, IDM_FILE_EXIT, "E&xit");
            AppendMenuW(bar, MF_POPUP, file as usize, win::wide("&File").as_ptr());
        }

        let edit = CreatePopupMenu();
        if !edit.is_null() {
            append(edit, MF_STRING, IDM_EDIT_UNDO, "&Undo\tCtrl+Z");
            append(edit, MF_STRING, IDM_EDIT_REDO, "&Redo\tCtrl+Y");
            AppendMenuW(edit, MF_SEPARATOR, 0, ptr::null());
            append(edit, MF_STRING, IDM_EDIT_CUT, "Cu&t\tCtrl+X");
            append(edit, MF_STRING, IDM_EDIT_COPY, "&Copy\tCtrl+C");
            append(edit, MF_STRING, IDM_EDIT_PASTE, "&Paste\tCtrl+V");
            AppendMenuW(edit, MF_SEPARATOR, 0, ptr::null());
            append(edit, MF_STRING, IDM_EDIT_SELECT_ALL, "Select &All\tCtrl+A");
            AppendMenuW(bar, MF_POPUP, edit as usize, win::wide("&Edit").as_ptr());
        }

        let format = CreatePopupMenu();
        if !format.is_null() {
            append(format, MF_STRING, IDM_FORMAT_BOLD, "&Bold\tCtrl+B");
            append(format, MF_STRING, IDM_FORMAT_ITALIC, "&Italic\tCtrl+I");
            append(
                format,
                MF_STRING,
                IDM_FORMAT_UNDERLINE,
                "&Underline\tCtrl+U",
            );
            append(format, MF_STRING, IDM_FORMAT_STRIKEOUT, "&Strikethrough");
            AppendMenuW(
                bar,
                MF_POPUP,
                format as usize,
                win::wide("F&ormat").as_ptr(),
            );
        }

        let view = CreatePopupMenu();
        if !view.is_null() {
            append(view, MF_STRING | MF_CHECKED, IDM_VIEW_TOOLBAR, "&Toolbar");
            append(view, MF_STRING | MF_CHECKED, IDM_VIEW_STATUS, "&Status Bar");
            AppendMenuW(bar, MF_POPUP, view as usize, win::wide("&View").as_ptr());
        }

        let help = CreatePopupMenu();
        if !help.is_null() {
            append(help, MF_STRING, IDM_HELP_ABOUT, "&About Nodi");
            AppendMenuW(bar, MF_POPUP, help as usize, win::wide("&Help").as_ptr());
        }

        bar
    }
}

fn append(menu: HMENU, flags: u32, id: u16, label: &str) {
    let text = win::wide(label);
    // SAFETY: `text` outlives the call; `menu` is a menu we created.
    unsafe {
        AppendMenuW(menu, flags, id as usize, text.as_ptr());
    }
}

pub fn create_accel() -> HACCEL {
    let accels = [
        accel(FCONTROL, b'N' as u16, IDM_FILE_NEW),
        accel(FCONTROL, b'S' as u16, IDM_FILE_SAVE),
        accel(FCONTROL, b'W' as u16, IDM_FILE_CLOSE_TAB),
        accel(FCONTROL, b'Z' as u16, IDM_EDIT_UNDO),
        accel(FCONTROL, b'Y' as u16, IDM_EDIT_REDO),
        accel(FCONTROL, b'B' as u16, IDM_FORMAT_BOLD),
        accel(FCONTROL, b'I' as u16, IDM_FORMAT_ITALIC),
        accel(FCONTROL, b'U' as u16, IDM_FORMAT_UNDERLINE),
        accel(FCONTROL, VK_TAB, IDM_TAB_NEXT),
        accel(FCONTROL | FSHIFT, VK_TAB, IDM_TAB_PREV),
    ];
    // SAFETY: `accels` is a valid array for the duration of the call.
    unsafe { CreateAcceleratorTableW(accels.as_ptr(), accels.len() as i32) }
}

fn accel(
    modifiers: u8,
    key: u16,
    command: u16,
) -> windows_sys::Win32::UI::WindowsAndMessaging::ACCEL {
    windows_sys::Win32::UI::WindowsAndMessaging::ACCEL {
        fVirt: modifiers | FVIRTKEY,
        key,
        cmd: command,
    }
}

pub fn on_command(app: &mut App, id: u16) {
    match id {
        IDM_FILE_NEW => app::new_tab(app),
        IDM_FILE_SAVE => {
            status::set_message(app, "Save is not implemented yet (Phase 3)");
        }
        IDM_FILE_CLOSE_TAB => app::close_active_tab(app),
        IDM_FILE_EXIT => {
            // SAFETY: posting to our own top-level window.
            unsafe {
                PostMessageW(app.hwnd, WM_CLOSE, 0, 0);
            }
        }
        IDM_EDIT_UNDO | IDM_EDIT_REDO | IDM_EDIT_CUT | IDM_EDIT_COPY | IDM_EDIT_PASTE
        | IDM_EDIT_SELECT_ALL => editor::command(app, id),
        IDM_FORMAT_BOLD | IDM_FORMAT_ITALIC | IDM_FORMAT_UNDERLINE | IDM_FORMAT_STRIKEOUT => {
            editor::toggle_effect(app, id);
        }
        IDM_VIEW_TOOLBAR => {
            app.toolbar_visible = !app.toolbar_visible;
            check_view_item(app, id, app.toolbar_visible);
            crate::wnd::layout(app);
        }
        IDM_VIEW_STATUS => {
            app.status_visible = !app.status_visible;
            check_view_item(app, id, app.status_visible);
            crate::wnd::layout(app);
        }
        IDM_HELP_ABOUT => dialogs::about(app.hwnd),
        IDM_TAB_NEXT => app::cycle_tab(app, 1),
        IDM_TAB_PREV => app::cycle_tab(app, -1),
        _ => {}
    }
}

fn check_view_item(app: &App, id: u16, checked: bool) {
    if app.menu.is_null() {
        return;
    }
    // SAFETY: `app.menu` is the menu bar attached to our frame window.
    unsafe {
        CheckMenuItem(
            app.menu,
            id as u32,
            MF_BYCOMMAND | if checked { MF_CHECKED } else { MF_UNCHECKED },
        );
    }
}
