//! Status bar with five parts: message | save state | caret | encoding | mode.

use std::ptr;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::UI::Controls::{
    SBARS_SIZEGRIP, SB_SETPARTS, SB_SETTEXTW, STATUSCLASSNAMEW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetWindowRect, SendMessageW, WM_SIZE, WS_CHILD, WS_VISIBLE,
};

use crate::app::App;
use crate::win;
use crate::wnd::editor;

pub const PART_MESSAGE: usize = 0;
pub const PART_STATE: usize = 1;
pub const PART_CARET: usize = 2;
pub const PART_ENCODING: usize = 3;
pub const PART_MODE: usize = 4;
const PART_COUNT: usize = 5;

pub fn create(app: &App) -> HWND {
    let style = WS_CHILD | WS_VISIBLE | SBARS_SIZEGRIP;

    // SAFETY: class name constant; parent handle belongs to `app`.
    unsafe {
        CreateWindowExW(
            0,
            STATUSCLASSNAMEW,
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
    }
}

pub fn init(app: &App) {
    set(app, PART_MESSAGE, "Ready");
    // The real state comes from the active tab (`App::refresh_active_ui`);
    // never claim "Saved" before a save was acknowledged.
    set(app, PART_STATE, "");
    set(app, PART_CARET, "Ln 1, Col 1");
    set(app, PART_ENCODING, "UTF-8");
    set(app, PART_MODE, "Rich Text");
}

pub fn set(app: &App, part: usize, text: &str) {
    if app.status.is_null() {
        return;
    }
    let buffer = win::wide(text);
    // SAFETY: `buffer` is NUL terminated and lives for the duration of the
    // call; `status` is the live status bar.
    unsafe {
        SendMessageW(
            app.status,
            SB_SETTEXTW,
            part & 0xFF,
            buffer.as_ptr() as isize,
        );
    }
}

pub fn set_message(app: &App, text: &str) {
    set(app, PART_MESSAGE, text);
}

/// Part 2 shows exactly one of `Modified`, `Saving...`, `Saved`,
/// `Save Failed` for the active tab.
pub fn set_save_state(app: &App, state: &str) {
    set(app, PART_STATE, state);
}

pub fn clear_caret(app: &App) {
    set(app, PART_CARET, "");
}

pub fn update_caret(app: &App) {
    match app.active_editor() {
        Some(editor_window) => {
            let (line, column) = editor::line_col(editor_window);
            set(app, PART_CARET, &format!("Ln {line}, Col {column}"));
        }
        None => clear_caret(app),
    }
}

/// Lets the status bar size itself to the frame (it computes its own font
/// height) and returns that height.
pub fn measure_height(app: &App) -> i32 {
    if app.status.is_null() {
        return app.scale(22);
    }
    // SAFETY: sending `WM_SIZE` to a status bar asks it to fit the parent.
    unsafe {
        SendMessageW(app.status, WM_SIZE, 0, 0);
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(app.status, &mut rect) == 0 {
            return app.scale(22);
        }
        (rect.bottom - rect.top).max(app.scale(20))
    }
}

/// Right-aligned parts with fixed widths, message part takes the remainder.
pub fn layout_parts(app: &App, width: i32) {
    if app.status.is_null() {
        return;
    }
    let step = app.scale(104);
    let min_width = app.scale(40);
    let mut edges = [
        width - step * 4,
        width - step * 3,
        width - step * 2,
        width - step,
        -1,
    ];

    let mut previous = 0;
    for edge in &mut edges {
        if *edge == -1 {
            break;
        }
        if *edge < previous + min_width {
            *edge = previous + min_width;
        }
        previous = *edge;
    }

    // SAFETY: `edges` is a 5-element i32 array as `SB_SETPARTS` expects.
    unsafe {
        SendMessageW(app.status, SB_SETPARTS, PART_COUNT, edges.as_ptr() as isize);
    }
}
