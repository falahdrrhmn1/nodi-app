//! Same-page word autocomplete: while the user types, a small popup lists the
//! words already used in the current note that start with what was typed.
//!
//! Like Notepad++'s word completion it is on by default and can be turned off
//! in View > Same-page Autocomplete. Unlike the old Ctrl+Space command (which
//! completes the single best match), this shows a list: Up/Down choose,
//! Tab/Enter insert, Esc dismisses. The popup uses `WS_EX_NOACTIVATE` so the
//! keyboard focus never leaves the editor.

use std::cell::{Cell, RefCell};
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, POINT};
use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VK_DOWN, VK_ESCAPE, VK_RETURN, VK_TAB, VK_UP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SendMessageW, SetWindowPos, HWND_TOP, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, WM_GETFONT, WM_SETFONT, WS_BORDER, WS_POPUP, WS_VSCROLL,
};

use crate::win;
use crate::wnd::editor;

const LBS_NOTIFY: u32 = 0x0001;
const LBS_NOINTEGRALHEIGHT: u32 = 0x0100;
const LB_ADDSTRING: u32 = 0x0180;
const LB_RESETCONTENT: u32 = 0x0184;
const LB_SETCURSEL: u32 = 0x0186;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const MAX_ITEMS: usize = 8;
/// Shorter prefixes are too noisy to suggest from.
const MIN_PREFIX: usize = 3;

thread_local! {
    static POPUP: Cell<HWND> = const { Cell::new(ptr::null_mut()) };
    static PREFIX_START: Cell<i32> = const { Cell::new(0) };
    static INDEX: Cell<usize> = const { Cell::new(0) };
    static SWALLOW_CHAR: Cell<bool> = const { Cell::new(false) };
    static ITEMS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn is_word(unit: u16) -> bool {
    char::from_u32(unit as u32).is_some_and(|ch| ch.is_alphanumeric() || ch == '_' || ch == '-')
}

/// Hides and destroys the popup.
pub fn dismiss() {
    POPUP.with(|cell| {
        let popup = cell.replace(ptr::null_mut());
        if !popup.is_null() {
            // SAFETY: `popup` is one of our list boxes and is not used after this.
            unsafe {
                DestroyWindow(popup);
            }
        }
    });
    ITEMS.with(|items| items.borrow_mut().clear());
    SWALLOW_CHAR.with(|flag| flag.set(false));
}

/// True when the popup is currently shown.
pub fn visible() -> bool {
    !POPUP.with(Cell::get).is_null()
}

fn ensure_popup(frame: HWND) -> HWND {
    let existing = POPUP.with(Cell::get);
    if !existing.is_null() {
        return existing;
    }
    let class = win::wide("LISTBOX");
    // SAFETY: standard list-box window owned by the frame; created hidden and
    // positioned later with `SetWindowPos`.
    let popup = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            ptr::null(),
            WS_POPUP | WS_BORDER | WS_VSCROLL | LBS_NOTIFY | LBS_NOINTEGRALHEIGHT,
            0,
            0,
            10,
            10,
            frame,
            ptr::null_mut(),
            GetModuleHandleW(ptr::null()),
            ptr::null(),
        )
    };
    if popup.is_null() {
        return popup;
    }
    // Match the editor font so the list does not look foreign.
    let font = unsafe { SendMessageW(frame, WM_GETFONT, 0, 0) } as usize;
    unsafe {
        SendMessageW(popup, WM_SETFONT, font, 1);
    }
    POPUP.with(|cell| cell.set(popup));
    popup
}

fn cycle(delta: i32) {
    let popup = POPUP.with(Cell::get);
    if popup.is_null() {
        return;
    }
    let count = ITEMS.with(|items| items.borrow().len());
    if count == 0 {
        return;
    }
    let next = INDEX.with(|cell| {
        let current = cell.get() as i32;
        let next = (current + delta).rem_euclid(count as i32) as usize;
        cell.set(next);
        next
    });
    unsafe {
        SendMessageW(popup, LB_SETCURSEL, next, 0);
    }
}

fn accept(editor: HWND) {
    let item = ITEMS.with(|items| items.borrow().get(INDEX.with(Cell::get)).cloned());
    if let Some(item) = item {
        let start = PREFIX_START.with(Cell::get);
        let (from, _) = editor::selection(editor);
        editor::set_selection(editor, start, from);
        editor::replace_selection(editor, &item, true);
    }
    dismiss();
}

/// Handles Up/Down/Enter/Tab/Esc while the popup is visible. Returns true when
/// the key was consumed.
pub fn handle_key(editor: HWND, key: u16) -> bool {
    if !visible() {
        return false;
    }
    match key {
        VK_ESCAPE => {
            dismiss();
            true
        }
        VK_RETURN | VK_TAB => {
            accept(editor);
            // Windows still translates the key into a character even though the
            // key-down was handled, so drop the following Tab/Enter.
            SWALLOW_CHAR.with(|flag| flag.set(true));
            true
        }
        VK_UP => {
            cycle(-1);
            true
        }
        VK_DOWN => {
            cycle(1);
            true
        }
        _ => false,
    }
}

/// The suggestions currently listed (used by the end-to-end tests).
pub fn items() -> Vec<String> {
    ITEMS.with(|items| items.borrow().clone())
}

/// True (once) when the Tab or Enter the user pressed to accept a suggestion
/// must not also reach the editor as a character.
pub fn take_swallow() -> bool {
    SWALLOW_CHAR.with(|flag| flag.replace(false))
}

/// Recomputes the suggestions from the word before the caret. Called after a
/// printable character was inserted.
pub fn update(editor: HWND, frame: HWND, enabled: bool) {
    if !enabled {
        dismiss();
        return;
    }
    let (from, to) = editor::selection(editor);
    if from != to {
        dismiss();
        return;
    }
    let units = editor::raw_text(editor);
    let caret = (from.max(0) as usize).min(units.len());
    let mut start = caret;
    while start > 0 && is_word(units[start - 1]) {
        start -= 1;
    }
    let prefix: String = units[start..caret]
        .iter()
        .filter_map(|unit| char::from_u32(*unit as u32))
        .collect();
    if prefix.chars().count() < MIN_PREFIX {
        dismiss();
        return;
    }
    let lowered = prefix.to_lowercase();
    let mut suggestions: Vec<String> = Vec::new();
    let mut index = 0;
    while index < units.len() {
        while index < units.len() && !is_word(units[index]) {
            index += 1;
        }
        let word_start = index;
        while index < units.len() && is_word(units[index]) {
            index += 1;
        }
        if word_start == index || word_start == start {
            continue;
        }
        let word: String = units[word_start..index]
            .iter()
            .filter_map(|unit| char::from_u32(*unit as u32))
            .collect();
        if word.to_lowercase().starts_with(&lowered)
            && word.chars().count() > prefix.chars().count()
            && !suggestions.iter().any(|existing| existing == &word)
        {
            suggestions.push(word);
            if suggestions.len() >= MAX_ITEMS {
                break;
            }
        }
    }
    if suggestions.is_empty() {
        dismiss();
        return;
    }
    suggestions.sort_by_key(|word| (word.chars().count(), word.to_lowercase()));
    show(editor, frame, suggestions, start as i32);
}

fn show(editor: HWND, frame: HWND, items: Vec<String>, prefix_start: i32) {
    let popup = ensure_popup(frame);
    if popup.is_null() {
        return;
    }
    PREFIX_START.with(|cell| cell.set(prefix_start));
    INDEX.with(|cell| cell.set(0));

    // SAFETY: standard list-box messages.
    unsafe {
        SendMessageW(popup, LB_RESETCONTENT, 0, 0);
        for item in &items {
            let wide = win::wide(item);
            SendMessageW(popup, LB_ADDSTRING, 0, wide.as_ptr() as isize);
        }
        SendMessageW(popup, LB_SETCURSEL, 0, 0);
    }
    ITEMS.with(|stored| *stored.borrow_mut() = items.clone());

    let dpi = unsafe { GetDpiForWindow(frame) }.max(96) as i32;
    let longest = items
        .iter()
        .map(|item| item.chars().count())
        .max()
        .unwrap_or(8);
    let width = ((longest as i32 * 7 + 24) * dpi / 96).clamp(140, 420);
    let item_height = (18 * dpi / 96).max(14);
    let height = item_height * items.len() as i32 + 6;
    let (caret_x, caret_y) = editor::char_point(editor, prefix_start);
    let mut point = POINT {
        x: caret_x,
        y: caret_y + item_height + 4,
    };
    // SAFETY: `point` is a valid in/out POINT.
    unsafe {
        ClientToScreen(editor, &mut point);
        SetWindowPos(
            popup,
            HWND_TOP,
            point.x,
            point.y,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}
