//! Tab control wrapper. One Rich Edit lives per tab; the control only holds
//! the labels, activation is handled by `app::activate_tab`.

use std::ptr;

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::UI::Controls::{
    TCIF_TEXT, TCITEMW, TCM_ADJUSTRECT, TCM_DELETEITEM, TCM_GETCURSEL, TCM_INSERTITEMW,
    TCM_SETCURSEL, TCM_SETITEMW, TCS_FOCUSNEVER, TCS_HOTTRACK, TCS_TOOLTIPS, WC_TABCONTROLW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, SendMessageW, WS_CHILD, WS_TABSTOP, WS_VISIBLE,
};

use crate::app::App;
use crate::win;

pub fn create(app: &App) -> HWND {
    let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP | TCS_HOTTRACK | TCS_TOOLTIPS | TCS_FOCUSNEVER;

    // SAFETY: class name constant; parent handle belongs to `app`.
    unsafe {
        CreateWindowExW(
            0,
            WC_TABCONTROLW,
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

/// Height of the tab row only, derived from `TCM_ADJUSTRECT` (the difference
/// between the window rect and the display rect it needs).
pub fn measure_height(app: &App) -> i32 {
    if app.tabctrl.is_null() {
        return app.scale(26);
    }
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 100,
        bottom: 100,
    };
    // SAFETY: `rect` is a valid RECT; `TCM_ADJUSTRECT` with wParam TRUE expands
    // a display rect into the window rect required to show it.
    unsafe {
        SendMessageW(
            app.tabctrl,
            TCM_ADJUSTRECT,
            1,
            &mut rect as *mut RECT as isize,
        );
    }
    let header = (rect.bottom - rect.top) - 100;
    header.max(app.scale(24))
}

pub fn insert(app: &App, index: usize, title: &str) {
    let mut text = win::wide(title);
    let mut item = item(&mut text);
    // SAFETY: `item` and `text` live for the duration of the call.
    unsafe {
        SendMessageW(
            app.tabctrl,
            TCM_INSERTITEMW,
            index,
            &mut item as *mut TCITEMW as isize,
        );
    }
}

pub fn set_title(app: &App, index: usize, title: &str) {
    let mut text = win::wide(title);
    let mut item = item(&mut text);
    // SAFETY: `item` and `text` live for the duration of the call.
    unsafe {
        SendMessageW(
            app.tabctrl,
            TCM_SETITEMW,
            index,
            &mut item as *mut TCITEMW as isize,
        );
    }
}

fn item(text: &mut [u16]) -> TCITEMW {
    TCITEMW {
        mask: TCIF_TEXT,
        dwState: 0,
        dwStateMask: 0,
        pszText: text.as_mut_ptr(),
        cchTextMax: text.len() as i32,
        iImage: 0,
        lParam: 0,
    }
}

pub fn delete(app: &App, index: usize) {
    // SAFETY: index was validated by the caller against the tab list.
    unsafe {
        SendMessageW(app.tabctrl, TCM_DELETEITEM, index, 0);
    }
}

pub fn set_cur(app: &App, index: usize) {
    // SAFETY: programmatic selection does not send `TCN_SELCHANGE`.
    unsafe {
        SendMessageW(app.tabctrl, TCM_SETCURSEL, index, 0);
    }
}

pub fn cur_sel(app: &App) -> Option<usize> {
    // SAFETY: `app.tabctrl` is the live tab control.
    let selected = unsafe { SendMessageW(app.tabctrl, TCM_GETCURSEL, 0, 0) };
    if selected < 0 {
        None
    } else {
        Some(selected as usize)
    }
}
