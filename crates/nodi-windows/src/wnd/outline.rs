//! View > Outline (Ctrl+Alt+O): a popup menu of the note's `# … ####`
//! headings; choosing one moves the caret to that line.
//!
//! The headings are read from the editor's plain text: the `# ` markers are
//! hidden formatting, so the buffer text is the single source of truth and the
//! menu works even while the markers are not painted.

use std::ptr;

use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetWindowRect, TrackPopupMenu,
    MF_STRING, TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_TOPALIGN,
};

use crate::app::App;
use crate::win;
use crate::wnd::{editor, status};

/// One heading: label, caret offset and nesting level.
pub struct Heading {
    pub label: String,
    pub offset: i32,
    pub level: usize,
}

/// Extracts the headings of `text` (Rich Edit line separator `\r`).
pub fn headings(text: &str) -> Vec<Heading> {
    let mut found = Vec::new();
    let mut offset = 0usize;
    for line in text.split('\r') {
        let level = line.chars().take_while(|ch| *ch == '#').count();
        if (1..=4).contains(&level) && line.chars().nth(level) == Some(' ') {
            let title = line[level + 1..].trim();
            if !title.is_empty() {
                found.push(Heading {
                    label: format!("{}{}", "    ".repeat(level - 1), title),
                    offset: offset as i32,
                    level,
                });
            }
        }
        offset += line.chars().map(|ch| ch.len_utf16()).sum::<usize>() + 1;
    }
    found
}

/// View > Outline: shows the popup and jumps to the chosen heading.
pub fn show(app: &mut App) {
    let Some(window) = app.active_editor() else {
        status::set_message(app, "No note open");
        return;
    };
    let window_text = editor::plain_text(window);
    let entries = headings(&window_text);
    if entries.is_empty() {
        status::set_message(app, "This note has no headings (#, ##, ###, ####)");
        return;
    }

    let menu = build_menu(&entries);
    if menu.is_null() {
        return;
    }
    let (x, y) = anchor(app, window);
    // SAFETY: `menu` is ours; `TPM_RETURNCMD` returns the id without sending
    // `WM_COMMAND`, and the popup is destroyed below.
    let command = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTALIGN | TPM_TOPALIGN,
            x,
            y,
            0,
            app.hwnd,
            ptr::null(),
        )
    };
    // SAFETY: the menu was created just above and is not attached anywhere.
    unsafe {
        DestroyMenu(menu);
    }

    let index = command as usize;
    if index == 0 || index > entries.len() {
        return;
    }
    let offset = entries[index - 1].offset;
    editor::set_selection(window, offset, offset);
    editor::scroll_caret(window);
    status::update_caret(app);
    status::update_counts(app);
    // SAFETY: focusing our own editor.
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(window);
    }
}

fn build_menu(entries: &[Heading]) -> windows_sys::Win32::UI::WindowsAndMessaging::HMENU {
    // SAFETY: the menu is ours; every label is a temporary that lives for the
    // duration of `AppendMenuW` (which copies it).
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return menu;
        }
        for (index, entry) in entries.iter().enumerate() {
            let label = if entry.level == 1 {
                entry.label.clone()
            } else {
                format!("{}  ({})", entry.label.trim(), entry.level)
            };
            AppendMenuW(menu, MF_STRING, index + 1, win::wide(&label).as_ptr());
        }
        menu
    }
}

/// Where the popup opens: at the caret, else the mouse, else the editor centre.
fn anchor(app: &App, window: HWND) -> (i32, i32) {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: plain out-parameter calls; `point` is on this stack.
    unsafe {
        let mut caret = POINT { x: 0, y: 0 };
        if windows_sys::Win32::UI::WindowsAndMessaging::GetCaretPos(&mut caret) != 0
            && ClientToScreen(window, &mut caret) != 0
            && caret.x != 0
            && caret.y != 0
        {
            return (caret.x, caret.y);
        }
        if GetCursorPos(&mut point) != 0 {
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(window, &mut rect) != 0
                && point.x >= rect.left
                && point.x <= rect.right
                && point.y >= rect.top
                && point.y <= rect.bottom
            {
                return (point.x, point.y);
            }
        }
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(window, &mut rect) != 0 {
            return (rect.left + app.scale(40), rect.top + app.scale(40));
        }
    }
    (0, 0)
}
