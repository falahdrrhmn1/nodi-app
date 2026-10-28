//! Tab control wrapper. One Rich Edit lives per tab; the control only holds
//! the labels, activation is handled by `app::activate_tab`.

use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    DrawTextW, InvalidateRect, ScreenToClient, SelectObject, SetBkMode, SetTextColor, DT_CENTER,
    DT_END_ELLIPSIS, DT_SINGLELINE, DT_VCENTER, HGDIOBJ, TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::{
    DRAWITEMSTRUCT, TCHITTESTINFO, TCIF_TEXT, TCITEMW, TCM_ADJUSTRECT, TCM_DELETEITEM,
    TCM_GETCURSEL, TCM_GETITEMW, TCM_HITTEST, TCM_INSERTITEMW, TCM_SETCURSEL, TCM_SETITEMW,
    TCS_FOCUSNEVER, TCS_HOTTRACK, TCS_OWNERDRAWFIXED, TCS_TOOLTIPS, WC_TABCONTROLW,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, PostMessageW, SendMessageW, TrackPopupMenu, TPM_LEFTALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_COMMAND, WM_ERASEBKGND, WM_MBUTTONDOWN, WS_CHILD, WS_TABSTOP, WS_VISIBLE,
};

use crate::app::App;
use crate::win;
use crate::wnd::menu;
use crate::wnd::theme;

/// Subclass id for the tab strip's background painting.
const SUBCLASS_ID: usize = 0x5441_4253; // 'TABS'

pub fn create(app: &App) -> HWND {
    let style = WS_CHILD
        | WS_VISIBLE
        | WS_TABSTOP
        | TCS_HOTTRACK
        | TCS_TOOLTIPS
        | TCS_FOCUSNEVER
        | TCS_OWNERDRAWFIXED;

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

/// Draws one tab in the current palette (`TCS_OWNERDRAWFIXED`).
pub fn draw_item(app: &App, item: &DRAWITEMSTRUCT) -> bool {
    if item.hwndItem != app.tabctrl {
        return false;
    }
    let index = item.itemID as usize;
    let selected = cur_sel(app) == Some(index);
    if index >= app.tabs.len() {
        return false;
    }
    // SAFETY: owner-draw hands us a valid HDC and rect for this item.
    unsafe {
        theme::fill(
            item.hDC,
            &item.rcItem,
            if selected {
                app.palette.tab_active
            } else {
                app.palette.tab_bg
            },
        );

        let mut text = [0u16; 512];
        let mut entry = TCITEMW {
            mask: TCIF_TEXT,
            dwState: 0,
            dwStateMask: 0,
            pszText: text.as_mut_ptr(),
            cchTextMax: text.len() as i32,
            iImage: 0,
            lParam: 0,
        };
        SendMessageW(
            app.tabctrl,
            TCM_GETITEMW,
            index,
            &mut entry as *mut TCITEMW as isize,
        );

        SetBkMode(item.hDC, TRANSPARENT as i32);
        SetTextColor(item.hDC, app.palette.window_text);
        let previous: HGDIOBJ = SelectObject(item.hDC, app.ui_font);
        let mut rect = item.rcItem;
        rect.left += app.scale(6);
        rect.right -= app.scale(6);
        DrawTextW(
            item.hDC,
            text.as_ptr(),
            -1,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
        );
        SelectObject(item.hDC, previous);

        // A thin rule under unselected tabs keeps them readable on dark.
        if !selected {
            let mut line = item.rcItem;
            line.top = line.bottom - 1;
            theme::fill(item.hDC, &line, app.palette.border);
        }
    }
    true
}

pub fn invalidate(app: &App) {
    if !app.tabctrl.is_null() {
        // SAFETY: our tab control.
        unsafe {
            InvalidateRect(app.tabctrl, ptr::null(), 0);
        }
    }
}

/// Tab index under a screen point (`None` outside every tab).
pub fn index_at(app: &App, screen: POINT) -> Option<usize> {
    if app.tabctrl.is_null() {
        return None;
    }
    let mut point = screen;
    // SAFETY: plain coordinate conversion for our own control.
    unsafe {
        if ScreenToClient(app.tabctrl, &mut point) == 0 {
            return None;
        }
        let mut hit = TCHITTESTINFO {
            pt: point,
            flags: 0,
        };
        let index = SendMessageW(
            app.tabctrl,
            TCM_HITTEST,
            0,
            &mut hit as *mut TCHITTESTINFO as isize,
        );
        (index >= 0).then_some(index as usize)
    }
}

/// Right-click menu for a tab. `screen` is the mouse position (keyboard
/// invocation passes the active tab's strip position).
pub fn show_context_menu(app: &mut App, index: usize, screen: Option<POINT>) {
    if index >= app.tabs.len() {
        return;
    }
    let menu_handle = menu::popup(&[
        menu::Item::Cmd(menu::IDM_TAB_CLOSE, "&Close"),
        menu::Item::Cmd(menu::IDM_TAB_CLOSE_OTHERS, "Close &Others"),
        menu::Item::Cmd(menu::IDM_TAB_CLOSE_ALL, "Close &All"),
        menu::Item::Sep,
        menu::Item::Cmd(menu::IDM_TAB_COPY_LINK, "Copy Internal &Link"),
        menu::Item::Cmd(menu::IDM_TAB_REVEAL, "&Reveal in Tree"),
    ]);
    if menu_handle.is_null() {
        return;
    }
    let (x, y) = match screen {
        Some(point) => (point.x, point.y),
        None => {
            let mut point = POINT { x: 0, y: 0 };
            // SAFETY: `point` is a valid out-parameter.
            unsafe {
                let mut rect = RECT::default();
                if windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(
                    app.tabctrl,
                    &mut rect,
                ) != 0
                {
                    point = POINT {
                        x: rect.left + app.scale(30) * (index as i32 + 1),
                        y: rect.bottom - app.scale(4),
                    };
                }
            }
            (point.x, point.y)
        }
    };
    app.tab_context = Some(index);
    // SAFETY: `menu_handle` is a popup we created and destroy below.
    let command = unsafe {
        TrackPopupMenu(
            menu_handle,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN,
            x,
            y,
            0,
            app.hwnd,
            ptr::null(),
        )
    };
    // SAFETY: the menu is not attached to the bar.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::DestroyMenu(menu_handle);
    }
    if command == 0 {
        app.tab_context = None;
        return;
    }
    menu::on_command(app, command as u16);
}

/// Where a tab's label starts, for the test hook and the keyboard menu.
pub fn screen_anchor(app: &App, index: usize) -> POINT {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: our tab control and a valid out-parameter.
    unsafe {
        let mut rect = RECT::default();
        if windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(app.tabctrl, &mut rect) != 0 {
            point = POINT {
                x: rect.left + app.scale(30) * (index as i32 + 1),
                y: rect.top + app.scale(8),
            };
        }
    }
    point
}

/// Fills the tab strip background so the themed area reaches the control edge.
pub fn install_subclass(app: &App) {
    if app.tabctrl.is_null() {
        return;
    }
    // SAFETY: our tab control; the frame handle travels as reference data.
    unsafe {
        SetWindowSubclass(
            app.tabctrl,
            Some(tab_subclass),
            SUBCLASS_ID,
            app.hwnd as usize,
        );
    }
}

unsafe extern "system" fn tab_subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    refdata: usize,
) -> LRESULT {
    if msg == WM_ERASEBKGND {
        if let Some(app) = crate::wnd::app_mut(refdata as HWND) {
            let hdc = wparam as windows_sys::Win32::Graphics::Gdi::HDC;
            let mut rect: RECT = std::mem::zeroed();
            if windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect) != 0 {
                theme::fill(hdc, &rect, app.palette.tab_bg);
                return 1;
            }
        }
    }
    if msg == WM_MBUTTONDOWN {
        // Middle click closes the tab under the cursor. The close is posted so
        // the tab control finishes handling the mouse message first.
        let mut hit = TCHITTESTINFO {
            pt: POINT {
                x: win::x_from_lparam(lparam),
                y: win::y_from_lparam(lparam),
            },
            flags: 0,
        };
        let index = SendMessageW(
            hwnd,
            TCM_HITTEST,
            0,
            &mut hit as *mut TCHITTESTINFO as isize,
        );
        if index >= 0 {
            if let Some(app) = crate::wnd::app_mut(refdata as HWND) {
                app.tab_context = Some(index as usize);
            }
            PostMessageW(refdata as HWND, WM_COMMAND, menu::IDM_TAB_CLOSE as usize, 0);
            return 0;
        }
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}
