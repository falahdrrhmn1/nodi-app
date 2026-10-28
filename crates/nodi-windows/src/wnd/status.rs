//! Status bar with five parts: message | save state | caret | counts | mode.
//!
//! In a dark theme the parts are owner-drawn from `App::status_text`, because
//! the native status bar ignores `WM_CTLCOLOR*` and has no custom-draw support.

use std::ptr;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    DrawTextW, SelectObject, SetBkMode, SetTextColor, DT_LEFT, DT_SINGLELINE, DT_VCENTER, HGDIOBJ,
    TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, SBARS_SIZEGRIP, SB_SETPARTS, SB_SETTEXTW, STATUSCLASSNAMEW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetWindowRect, SendMessageW, WM_SIZE, WS_CHILD, WS_VISIBLE,
};

use crate::app::App;
use crate::win;
use crate::wnd::editor;
use crate::wnd::theme;

pub const PART_MESSAGE: usize = 0;
pub const PART_STATE: usize = 1;
pub const PART_CARET: usize = 2;
pub const PART_COUNTS: usize = 3;
pub const PART_MODE: usize = 4;
pub const PART_COUNT: usize = 5;

/// `SB_SETTEXT` flag: the parent draws this part (`WM_DRAWITEM`).
const SBT_OWNERDRAW: usize = 0x1000;

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

pub fn init(app: &mut App) {
    set(app, PART_MESSAGE, "Ready");
    // The real state comes from the active tab (`App::refresh_active_ui`);
    // never claim "Saved" before a save was acknowledged.
    set(app, PART_STATE, "");
    set(app, PART_CARET, "Ln 1, Col 1");
    set(app, PART_COUNTS, "");
    set(app, PART_MODE, "Rich Text");
}

pub fn set(app: &mut App, part: usize, text: &str) {
    if part < app.status_text.len() {
        app.status_text[part] = text.to_owned();
    }
    if app.status.is_null() {
        return;
    }
    let buffer = win::wide(text);
    let flags = part | if app.palette.dark { SBT_OWNERDRAW } else { 0 };
    // SAFETY: `buffer` is NUL terminated and lives for the duration of the
    // call; `status` is the live status bar.
    unsafe {
        SendMessageW(app.status, SB_SETTEXTW, flags, buffer.as_ptr() as isize);
    }
}

/// Re-sends every part after a theme change (the owner-draw flag changes).
pub fn reapply(app: &mut App) {
    let texts = app.status_text.clone();
    for (part, text) in texts.iter().enumerate() {
        set(app, part, text);
    }
}

pub fn set_message(app: &mut App, text: &str) {
    set(app, PART_MESSAGE, text);
}

/// Part 2 shows exactly one of `Modified`, `Saving...`, `Saved`,
/// `Save Failed` for the active tab.
pub fn set_save_state(app: &mut App, state: &str) {
    set(app, PART_STATE, state);
}

pub fn clear_caret(app: &mut App) {
    set(app, PART_CARET, "");
}

pub fn update_caret(app: &mut App) {
    let text = match app.active_editor() {
        Some(editor_window) => {
            let (line, column) = editor::line_col(editor_window);
            format!("Ln {line}, Col {column}")
        }
        None => String::new(),
    };
    set(app, PART_CARET, &text);
}

/// Word and character count for the status bar (`selection` or whole note).
pub fn set_counts(app: &mut App, words: usize, chars: usize, selection: bool) {
    let suffix = if selection { " (sel)" } else { "" };
    set(
        app,
        PART_COUNTS,
        &format!("{words} words · {chars} chars{suffix}"),
    );
}

/// Recomputes part 4 from the active editor: whole note, or the selection.
pub fn update_counts(app: &mut App) {
    let Some(window) = app.active_editor() else {
        set_counts(app, 0, 0, false);
        return;
    };
    let (from, to) = editor::selection(window);
    let selection = from != to;
    let text = if selection {
        editor::text_range(window, from, to)
    } else {
        editor::plain_text(window)
    };
    let (words, chars) = count_text(&text);
    set_counts(app, words, chars, selection);
}

/// Words and characters in `text`, ignoring line breaks and the hidden
/// Markdown markers (`# `, `## ` …) that headings keep in the buffer.
fn count_text(text: &str) -> (usize, usize) {
    let mut words = 0;
    let mut chars = 0;
    for line in text.split('\r') {
        let visible = visible_line(line);
        words += visible.split_whitespace().count();
        chars += visible
            .chars()
            .filter(|ch| !matches!(ch, '\r' | '\n'))
            .count();
    }
    (words, chars)
}

/// A line without a leading heading marker (`### `).
fn visible_line(line: &str) -> &str {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if (1..=4).contains(&hashes) && line.chars().nth(hashes) == Some(' ') {
        &line[hashes + 1..]
    } else {
        line
    }
}

/// Draws one owner-drawn part when the status bar is dark.
pub fn draw_item(app: &App, item: &DRAWITEMSTRUCT) -> bool {
    if item.hwndItem != app.status {
        return false;
    }
    let Some(text) = app.status_text.get(item.itemID as usize) else {
        return false;
    };
    // SAFETY: owner-draw hands us a valid HDC and rect for this part.
    unsafe {
        theme::fill(item.hDC, &item.rcItem, app.palette.window_bg);
        let mut top = item.rcItem;
        top.bottom = top.top + 1;
        theme::fill(item.hDC, &top, app.palette.border);

        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, app.palette.window_text);
        let previous: HGDIOBJ = SelectObject(item.hDC, app.ui_font);
        let mut rect = item.rcItem;
        rect.left += app.scale(6);
        rect.right -= app.scale(2);
        DrawTextW(
            item.hDC,
            win::wide(text).as_ptr(),
            -1,
            &mut rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(item.hDC, previous);
    }
    true
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
    // Parts: message | save state | Ln/Col | counts | mode. The counts part
    // holds the word/character counts plus the selection counts.
    let mode = app.scale(90);
    let counts = app.scale(300);
    let caret = app.scale(110);
    let save = app.scale(100);
    let min_width = app.scale(40);
    let mut edges = [
        width - mode - counts - caret - save,
        width - mode - counts - caret,
        width - mode - counts,
        width - mode,
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
