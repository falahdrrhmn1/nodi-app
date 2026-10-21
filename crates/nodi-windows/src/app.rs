//! Application state, reachable from the window procedure through
//! `GWLP_USERDATA` (see [`crate::wnd`]).

use std::mem::size_of;
use std::ptr;

use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontIndirectW, DeleteObject, DEFAULT_CHARSET, FW_BOLD, FW_NORMAL, HFONT, LOGFONTW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, LoadCursorW, SendMessageW, SetWindowTextW, ShowWindow, SystemParametersInfoW,
    HACCEL, HCURSOR, HMENU, IDC_SIZEWE, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SW_HIDE,
    SW_SHOW, WM_SETFONT,
};

use crate::win;
use crate::wnd::{editor, status, tabs};

pub struct Tab {
    pub editor: HWND,
    /// Base title, without the dirty marker.
    pub title: String,
    pub dirty: bool,
}

pub struct App {
    pub hwnd: HWND,
    pub hinstance: HINSTANCE,
    pub menu: HMENU,
    pub accel: HACCEL,

    pub tabs: Vec<Tab>,
    pub active: Option<usize>,
    pub untitled_count: u32,

    pub dpi: u32,
    pub ui_font: HFONT,
    pub bold_font: HFONT,
    pub hcursor_we: HCURSOR,

    pub toolbar: HWND,
    pub sidebar_title: HWND,
    pub tree: HWND,
    pub tabctrl: HWND,
    pub status: HWND,

    pub toolbar_visible: bool,
    pub status_visible: bool,
    pub sidebar_width: i32,
    pub splitter_drag: bool,

    pub toolbar_height: i32,
    /// Set once `WM_CREATE` finished: from then on the window procedure owns
    /// the boxed `App` and frees it in `WM_NCDESTROY`.
    pub created: bool,
}

impl App {
    pub(crate) fn new() -> Self {
        Self {
            hwnd: ptr::null_mut(),
            hinstance: ptr::null_mut(),
            menu: ptr::null_mut(),
            accel: ptr::null_mut(),
            tabs: Vec::new(),
            active: None,
            untitled_count: 0,
            dpi: 96,
            ui_font: ptr::null_mut(),
            bold_font: ptr::null_mut(),
            // SAFETY: `IDC_SIZEWE` is a predefined system cursor; the call
            // takes no pointers we own and returns a shared handle.
            hcursor_we: unsafe { LoadCursorW(ptr::null_mut(), IDC_SIZEWE) },
            toolbar: ptr::null_mut(),
            sidebar_title: ptr::null_mut(),
            tree: ptr::null_mut(),
            tabctrl: ptr::null_mut(),
            status: ptr::null_mut(),
            toolbar_visible: true,
            status_visible: true,
            sidebar_width: 240,
            splitter_drag: false,
            toolbar_height: 0,
            created: false,
        }
    }

    pub fn active_editor(&self) -> Option<HWND> {
        self.active
            .and_then(|index| self.tabs.get(index))
            .map(|tab| tab.editor)
    }

    pub fn tab_index(&self, editor: HWND) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.editor == editor)
    }

    pub fn scale(&self, value: i32) -> i32 {
        win::mul_div(value, self.dpi as i32, 96)
    }
}

/// Creates (or recreates, after a DPI change) the UI fonts and applies them to
/// every control that currently exists.
pub fn recreate_fonts(app: &mut App) {
    // SAFETY: both calls only read system metrics and create a GDI font.
    let ui = unsafe { make_ui_font(app.dpi, false) };
    let bold = unsafe { make_ui_font(app.dpi, true) };
    let old_ui = std::mem::replace(&mut app.ui_font, ui);
    let old_bold = std::mem::replace(&mut app.bold_font, bold);

    apply_fonts(app);

    delete_font(old_ui);
    delete_font(old_bold);
}

pub fn destroy_fonts(app: &mut App) {
    let ui = std::mem::replace(&mut app.ui_font, ptr::null_mut());
    let bold = std::mem::replace(&mut app.bold_font, ptr::null_mut());
    delete_font(ui);
    delete_font(bold);
}

fn delete_font(font: HFONT) {
    if font.is_null() {
        return;
    }
    // SAFETY: `font` was created by `CreateFontIndirectW` and has already been
    // replaced in every control (or its window is gone).
    unsafe {
        DeleteObject(font);
    }
}

/// `WM_SETFONT` for every control that uses the chrome font; the sidebar
/// caption gets the bold variant.
pub fn apply_fonts(app: &App) {
    if app.ui_font.is_null() {
        return;
    }
    // SAFETY: all handles are null (skipped) or child windows created by this
    // process; `WM_SETFONT` only stores the handle.
    unsafe {
        for control in [app.tree, app.tabctrl, app.status, app.toolbar] {
            if !control.is_null() {
                SendMessageW(control, WM_SETFONT, app.ui_font as usize, 1);
            }
        }
        if !app.sidebar_title.is_null() && !app.bold_font.is_null() {
            SendMessageW(app.sidebar_title, WM_SETFONT, app.bold_font as usize, 1);
        }
    }
}

unsafe fn make_ui_font(dpi: u32, bold: bool) -> HFONT {
    let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
    metrics.cbSize = size_of::<NONCLIENTMETRICSW>() as u32;
    let ok = SystemParametersInfoW(
        SPI_GETNONCLIENTMETRICS,
        metrics.cbSize,
        &mut metrics as *mut _ as *mut core::ffi::c_void,
        0,
    );

    let mut logfont: LOGFONTW = if ok != 0 {
        metrics.lfMessageFont
    } else {
        let mut fallback: LOGFONTW = std::mem::zeroed();
        fallback.lfHeight = -12;
        fallback.lfWeight = FW_NORMAL as i32;
        fallback.lfCharSet = DEFAULT_CHARSET;
        set_face(&mut fallback.lfFaceName, "Segoe UI");
        fallback
    };

    logfont.lfHeight = win::mul_div(logfont.lfHeight, dpi as i32, 96);
    if logfont.lfHeight == 0 {
        logfont.lfHeight = -win::mul_div(12, dpi as i32, 96);
    }
    if bold {
        logfont.lfWeight = FW_BOLD as i32;
    }

    CreateFontIndirectW(&logfont)
}

fn set_face(buffer: &mut [u16; 32], name: &str) {
    buffer.fill(0);
    for (slot, unit) in buffer.iter_mut().zip(name.encode_utf16()) {
        *slot = unit;
    }
}

/// Called on `EN_CHANGE`: marks the tab dirty (once) and updates the chrome.
pub fn mark_dirty(app: &mut App, editor: HWND) {
    let Some(index) = app.tab_index(editor) else {
        return;
    };
    if !app.tabs[index].dirty {
        app.tabs[index].dirty = true;
        let label = format!("{} *", app.tabs[index].title);
        tabs::set_title(app, index, &label);
        sync_window_title(app);
    }
    status::set_save_state(app, true);
    status::set_message(app, "Modified");
}

/// Keeps the frame caption in sync with the active tab, mirroring its dirty
/// marker. The caption always ends with the product name.
pub fn sync_window_title(app: &App) {
    if app.hwnd.is_null() {
        return;
    }

    let caption = match app.active.and_then(|index| app.tabs.get(index)) {
        Some(tab) if tab.dirty => format!("{} * - Nodi", tab.title),
        Some(tab) => format!("{} - Nodi", tab.title),
        None => "Nodi".to_string(),
    };
    let text = win::wide(&caption);

    // SAFETY: the buffer is NUL terminated and outlives the call; `hwnd` is our
    // own frame window.
    unsafe {
        SetWindowTextW(app.hwnd, text.as_ptr());
    }
}

pub fn on_editor_change(app: &mut App, editor: HWND) {
    mark_dirty(app, editor);
    status::update_caret(app);
}

pub fn on_tab_selchange(app: &mut App) {
    let Some(selected) = tabs::cur_sel(app) else {
        return;
    };
    if app.active != Some(selected) {
        activate_tab(app, selected);
    }
}

pub fn new_tab(app: &mut App) {
    let Some(editor) = editor::create(app) else {
        status::set_message(app, "Could not create the editor");
        return;
    };

    app.untitled_count += 1;
    let title = format!("Untitled {}", app.untitled_count);
    app.tabs.push(Tab {
        editor,
        title: title.clone(),
        dirty: false,
    });

    let index = app.tabs.len() - 1;
    tabs::insert(app, index, &title);
    crate::wnd::layout(app);
    activate_tab(app, index);
}

pub fn activate_tab(app: &mut App, index: usize) {
    if index >= app.tabs.len() {
        return;
    }

    // SAFETY: every handle came from `CreateWindowExW` and is still alive.
    unsafe {
        for (position, tab) in app.tabs.iter().enumerate() {
            ShowWindow(
                tab.editor,
                if position == index { SW_SHOW } else { SW_HIDE },
            );
        }
    }

    app.active = Some(index);
    tabs::set_cur(app, index);

    let dirty = app.tabs[index].dirty;
    status::set_save_state(app, dirty);
    status::update_caret(app);
    status::set_message(app, "Ready");
    sync_window_title(app);

    let editor = app.tabs[index].editor;
    // SAFETY: the editor window is our child and alive.
    unsafe {
        SetFocus(editor);
    }
}

pub fn close_active_tab(app: &mut App) {
    if let Some(index) = app.active {
        close_tab(app, index);
    }
}

pub fn close_tab(app: &mut App, index: usize) {
    if index >= app.tabs.len() {
        return;
    }

    let previous_active = app.active;
    let editor = app.tabs[index].editor;
    if !editor.is_null() {
        // SAFETY: the editor is a child window of this process.
        unsafe {
            DestroyWindow(editor);
        }
    }
    app.tabs.remove(index);
    tabs::delete(app, index);

    if app.tabs.is_empty() {
        app.active = None;
        status::set_save_state(app, false);
        status::set_message(app, "No note open");
        status::clear_caret(app);
        sync_window_title(app);
        // SAFETY: the main window is alive while handling this command.
        unsafe {
            SetFocus(app.hwnd);
        }
    } else {
        let target = match previous_active {
            Some(active) if active > index => active - 1,
            Some(active) if active < index => active,
            _ => index.min(app.tabs.len() - 1),
        };
        app.active = None;
        activate_tab(app, target);
    }

    crate::wnd::layout(app);
}

pub fn cycle_tab(app: &mut App, delta: i32) {
    let Some(active) = app.active else {
        return;
    };
    let count = app.tabs.len() as i32;
    if count <= 1 {
        return;
    }
    let next = ((active as i32 + delta) % count + count) % count;
    activate_tab(app, next as usize);
}

/// `WM_CLOSE` helper: the shell cannot save yet, so warn once if text exists.
pub fn close_requested(app: &App) -> bool {
    if app.tabs.iter().any(|tab| tab.dirty) {
        return crate::wnd::dialogs::confirm_discard(app.hwnd);
    }
    true
}
