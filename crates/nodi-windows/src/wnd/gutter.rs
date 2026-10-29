//! View > Line Numbers: a narrow strip between the Notes Explorer and the
//! note that numbers its lines like Notepad++ — one number per paragraph,
//! shown on the first row of a wrapped paragraph. A click selects the line.
//!
//! The paragraph starts of the active note are cached and recomputed only
//! after an edit; a short timer repaints the strip when the note scrolls
//! (Rich Edit does not report wheel or keyboard scrolling).

use std::cell::RefCell;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint, FillRect, InvalidateRect,
    SelectObject, SetBkMode, SetTextColor, DT_RIGHT, DT_SINGLELINE, DT_TOP, HGDIOBJ, PAINTSTRUCT,
    TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, KillTimer, LoadCursorW, RegisterClassExW,
    SendMessageW, SetTimer, IDC_ARROW, WM_LBUTTONDOWN, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_CHILD,
    WS_VISIBLE,
};

use crate::app::App;
use crate::win;
use crate::wnd::editor;

const CLASS: &str = "NodiGutter";
const EM_GETFIRSTVISIBLELINE: u32 = 0x00CE;
const EM_LINEINDEX: u32 = 0x00BB;
const TIMER: usize = 0x4E47; // 'NG'
const TIMER_MS: u32 = 60;

thread_local! {
    /// `(editor, paragraph starts)` of the note the numbers belong to.
    static STARTS: RefCell<Option<(isize, Vec<i32>)>> = const { RefCell::new(None) };
    /// What the strip showed last: editor, first visible line, its y, length.
    static SHOWN: RefCell<(isize, i32, i32, i32)> = const { RefCell::new((0, -1, 0, 0)) };
}

/// Forget the cached paragraph starts (the note changed).
pub fn invalidate(editor: HWND) {
    STARTS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache
            .as_ref()
            .is_some_and(|(owner, _)| *owner == editor as isize)
        {
            *cache = None;
        }
    });
}

/// Start positions of every paragraph (line) of `editor`.
fn starts(editor: HWND) -> Vec<i32> {
    if let Some(starts) = STARTS.with(|cache| {
        cache
            .borrow()
            .as_ref()
            .filter(|(owner, _)| *owner == editor as isize)
            .map(|(_, starts)| starts.clone())
    }) {
        return starts;
    }
    let raw = editor::raw_text(editor);
    let mut starts = vec![0];
    for (index, unit) in raw.iter().enumerate() {
        if *unit == 0x000D {
            starts.push(index as i32 + 1);
        }
    }
    STARTS.with(|cache| *cache.borrow_mut() = Some((editor as isize, starts.clone())));
    starts
}

/// 1-based line (paragraph) number of character `pos`, and that line's start.
pub fn line_of(editor: HWND, pos: i32) -> (usize, i32) {
    let starts = starts(editor);
    let index = match starts.binary_search(&pos) {
        Ok(index) => index,
        Err(index) => index.saturating_sub(1),
    };
    (index + 1, starts.get(index).copied().unwrap_or(0))
}

/// Number of lines (for the strip's width).
fn line_count(editor: HWND) -> usize {
    starts(editor).len()
}

pub fn create(app: &App) -> HWND {
    let class = win::wide(CLASS);
    // SAFETY: registering and creating our own child window class.
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: GetModuleHandleW(ptr::null()),
            hIcon: ptr::null_mut(),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class.as_ptr(),
            hIconSm: ptr::null_mut(),
        };
        RegisterClassExW(&wc);
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            ptr::null(),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            0,
            0,
            app.hwnd,
            ptr::null_mut(),
            GetModuleHandleW(ptr::null()),
            ptr::null(),
        );
        if !window.is_null() {
            SetTimer(window, TIMER, TIMER_MS, None);
        }
        window
    }
}

/// Width the strip needs for the active note (0 when switched off).
pub fn width(app: &App) -> i32 {
    if !app.settings.line_numbers || app.gutter.is_null() {
        return 0;
    }
    let lines = app.active_editor().map_or(1, line_count);
    let digits = lines.max(1).to_string().len().max(2) as i32;
    app.scale(10 + digits * 8)
}

/// Repaints the strip (a note was opened, typed into or scrolled).
pub fn refresh(app: &App) {
    if !app.gutter.is_null() {
        // SAFETY: our own child window.
        unsafe {
            InvalidateRect(app.gutter, ptr::null(), 0);
        }
    }
}

/// Timer: repaint when the visible part of the note moved, and widen the
/// strip when the number of digits changed.
fn tick(app: &mut App) {
    let Some(window) = app.active_editor() else {
        return;
    };
    if !app.settings.line_numbers {
        return;
    }
    // SAFETY: plain queries of our own Rich Edit.
    let first = unsafe { SendMessageW(window, EM_GETFIRSTVISIBLELINE, 0, 0) } as i32;
    // SAFETY: as above.
    let first_char = unsafe { SendMessageW(window, EM_LINEINDEX, first as usize, 0) } as i32;
    let (_, top) = editor::char_point(window, first_char.max(0));
    let length = editor::text_length(window);
    let now = (window as isize, first, top, length);
    let changed = SHOWN.with(|shown| {
        let mut shown = shown.borrow_mut();
        let changed = *shown != now;
        *shown = now;
        changed
    });
    if changed {
        let mut rect = RECT::default();
        // SAFETY: our own child window.
        unsafe {
            GetClientRect(app.gutter, &mut rect);
        }
        if rect.right != width(app) {
            crate::wnd::layout(app);
        }
        refresh(app);
    }
}

unsafe fn paint(app: &App, hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let dc = BeginPaint(hwnd, &mut ps);
    let mut rect = RECT::default();
    GetClientRect(hwnd, &mut rect);
    let palette = &app.palette;
    let brush = CreateSolidBrush(palette.gutter_bg);
    FillRect(dc, &rect, brush);
    DeleteObject(brush as HGDIOBJ);
    if let Some(window) = app.active_editor() {
        let old = SelectObject(dc, app.ui_font as HGDIOBJ);
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, palette.gutter_text);
        let starts = starts(window);
        let first = SendMessageW(window, EM_GETFIRSTVISIBLELINE, 0, 0) as i32;
        let mut line = first.max(0);
        let mut guard = 0;
        loop {
            let char_index = SendMessageW(window, EM_LINEINDEX, line as usize, 0) as i32;
            if char_index < 0 || guard > 400 {
                break;
            }
            let (_, y) = editor::char_point(window, char_index);
            if y > rect.bottom {
                break;
            }
            if let Ok(index) = starts.binary_search(&char_index) {
                let text = win::wide(&(index + 1).to_string());
                let mut cell = RECT {
                    left: 0,
                    top: y + app.scale(1),
                    right: rect.right - app.scale(6),
                    bottom: y + app.scale(24),
                };
                DrawTextW(
                    dc,
                    text.as_ptr(),
                    -1,
                    &mut cell,
                    DT_RIGHT | DT_TOP | DT_SINGLELINE,
                );
            }
            line += 1;
            guard += 1;
        }
        if !old.is_null() {
            SelectObject(dc, old);
        }
    }
    EndPaint(hwnd, &ps);
}

/// A click on a number selects that whole line.
fn click(app: &mut App, y: i32) {
    let Some(window) = app.active_editor() else {
        return;
    };
    let Some(pos) = editor::char_at_point(window, 1, y) else {
        return;
    };
    let (line, start) = line_of(window, pos);
    let starts = starts(window);
    let end = starts
        .get(line)
        .copied()
        .unwrap_or(editor::text_length(window));
    editor::set_selection(window, start, end);
    // SAFETY: focusing our own editor.
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(window);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let frame = windows_sys::Win32::UI::WindowsAndMessaging::GetParent(hwnd);
    match msg {
        WM_PAINT => {
            match crate::wnd::app_mut(frame) {
                Some(app) => paint(app, hwnd),
                None => {
                    let mut ps: PAINTSTRUCT = std::mem::zeroed();
                    BeginPaint(hwnd, &mut ps);
                    EndPaint(hwnd, &ps);
                }
            }
            0
        }
        WM_TIMER if wparam == TIMER => {
            if let Some(app) = crate::wnd::app_mut(frame) {
                tick(app);
            }
            0
        }
        WM_LBUTTONDOWN => {
            if let Some(app) = crate::wnd::app_mut(frame) {
                click(app, win::y_from_lparam(lparam));
            }
            0
        }
        windows_sys::Win32::UI::WindowsAndMessaging::WM_DESTROY => {
            KillTimer(hwnd, TIMER);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
