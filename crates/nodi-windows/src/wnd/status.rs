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

/// Timer id of the debounced count update.
pub const COUNTS_TIMER: usize = usize::MAX - 3;
/// Counting walks the whole note, so it waits until typing pauses briefly.
const COUNTS_DELAY_MS: u32 = 120;

/// Counts for the status bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub lines: usize,
    pub words: usize,
    pub chars: usize,
}

/// Restarts the count timer (EN_CHANGE / EN_SELCHANGE).
pub fn schedule_counts(app: &App) {
    // SAFETY: (re)starting a timer on our own window.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetTimer(
            app.hwnd,
            COUNTS_TIMER,
            COUNTS_DELAY_MS,
            None,
        );
    }
}

/// Status text: `Lines N · Words N · Chars N`, plus the selection's counts.
pub fn counts_text(note: Counts, selection: Option<Counts>) -> String {
    let mut text = format!(
        "Lines {} · Words {} · Chars {}",
        note.lines, note.words, note.chars
    );
    if let Some(sel) = selection {
        text.push_str(&format!(
            " | Sel: {} lines, {} words, {} chars",
            sel.lines, sel.words, sel.chars
        ));
    }
    text
}

/// Recomputes the counts part from the active editor: the whole note, and
/// the selection when there is one.
pub fn update_counts(app: &mut App) {
    let Some(window) = app.active_editor() else {
        set(app, PART_COUNTS, "");
        return;
    };
    let (from, to) = editor::selection(window);
    let note = count_text(&editor::plain_text(window));
    let selection = (from != to).then(|| count_text(&editor::text_range(window, from, to)));
    set(app, PART_COUNTS, &counts_text(note, selection));
}

/// Lines (paragraphs), words and characters in `text`, ignoring line breaks
/// and the hidden Markdown markers (`# `, `## ` …) that headings keep in the
/// buffer. An empty text has 0 lines; a trailing break adds no line.
pub fn count_text(text: &str) -> Counts {
    let mut counts = Counts::default();
    let trimmed = text.strip_suffix('\r').unwrap_or(text);
    if trimmed.is_empty() {
        return counts;
    }
    for line in trimmed.split(['\r', '\u{000B}']) {
        counts.lines += 1;
        let visible = visible_line(line);
        counts.words += visible.split_whitespace().count();
        counts.chars += visible
            .chars()
            .filter(|ch| !matches!(ch, '\r' | '\n' | '\u{FFFC}'))
            .count();
    }
    counts
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
    let counts = app.scale(380);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_lines_words_and_chars() {
        let counts = count_text("## Judul\rsatu dua\r\rtiga\r");
        assert_eq!(
            counts,
            Counts {
                lines: 4,
                words: 4,
                chars: 17
            }
        );
        assert_eq!(count_text(""), Counts::default());
        assert_eq!(
            count_text("a\u{FFFC}b"),
            Counts {
                lines: 1,
                words: 1,
                chars: 2
            }
        );
    }

    #[test]
    fn status_text_lists_the_selection_too() {
        let note = Counts {
            lines: 3,
            words: 5,
            chars: 20,
        };
        let sel = Counts {
            lines: 1,
            words: 2,
            chars: 8,
        };
        assert_eq!(counts_text(note, None), "Lines 3 · Words 5 · Chars 20");
        assert_eq!(
            counts_text(note, Some(sel)),
            "Lines 3 · Words 5 · Chars 20 | Sel: 1 lines, 2 words, 8 chars"
        );
    }
}
