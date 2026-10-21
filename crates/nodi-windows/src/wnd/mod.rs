//! Frame window: creation, message loop, layout and splitter handling.

pub mod dialogs;
pub mod editor;
pub mod menu;
pub mod status;
pub mod tabs;
pub mod toolbar;
pub mod tree;

use std::mem::size_of;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, DeleteObject, EndPaint, FillRect, GetSysColor, GetSysColorBrush,
    InvalidateRect, LineTo, MoveToEx, ScreenToClient, SelectObject, UpdateWindow, COLOR_3DFACE,
    COLOR_3DSHADOW, HBRUSH, HDC, PAINTSTRUCT, PS_SOLID,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_BAR_CLASSES, ICC_STANDARD_CLASSES, ICC_TAB_CLASSES,
    ICC_TREEVIEW_CLASSES, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, NMHDR, TCN_SELCHANGE,
};
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::DestroyAcceleratorTable;
use windows_sys::Win32::UI::WindowsAndMessaging::HMENU;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos,
    GetMessageW, GetWindowLongPtrW, GetWindowRect, LoadIconW, MessageBoxW, PostQuitMessage,
    RegisterClassExW, SetCursor, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    SystemParametersInfoW, TranslateAcceleratorW, TranslateMessage, CREATESTRUCTW, CS_HREDRAW,
    CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HTCLIENT, IDI_APPLICATION, MB_ICONERROR, MB_OK,
    MINMAXINFO, MSG, SIZE_MINIMIZED, SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    SW_SHOW, WM_CAPTURECHANGED, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_DPICHANGED,
    WM_ERASEBKGND, WM_GETMINMAXINFO, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE,
    WM_NCDESTROY, WM_NOTIFY, WM_PAINT, WM_SETCURSOR, WM_SETFOCUS, WM_SIZE, WNDCLASSEXW,
    WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW,
};

use crate::app::{self, App};
use crate::win;

pub const CLASS_NAME: &str = "NodiMainWindow";
const SPLITTER_WIDTH: i32 = 6;

/// Entry point for the GUI: loads the Rich Edit library, registers the frame
/// window class, runs the message loop and returns the exit code.
pub fn run() -> i32 {
    // SAFETY: the whole function is the process' single-threaded GUI bootstrap;
    // every pointer passed out lives on this stack frame.
    unsafe { run_inner() }
}

unsafe fn run_inner() -> i32 {
    let msftedit = LoadLibraryW(win::wide("msftedit.dll").as_ptr());
    if msftedit.is_null() {
        show_fatal("Nodi requires msftedit.dll (Rich Edit 4.1), which is missing.");
        return 1;
    }
    if LoadLibraryW(win::wide("comctl32.dll").as_ptr()).is_null() {
        show_fatal("Nodi requires comctl32.dll, which is missing.");
        return 1;
    }

    let classes = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_WIN95_CLASSES
            | ICC_BAR_CLASSES
            | ICC_TAB_CLASSES
            | ICC_TREEVIEW_CLASSES
            | ICC_STANDARD_CLASSES,
    };
    InitCommonControlsEx(&classes);

    // The manifest already requests PerMonitorV2; this call is a fallback for
    // builds where the resource compiler was unavailable. It fails harmlessly
    // when the manifest already set the awareness.
    SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

    let hinstance = GetModuleHandleW(ptr::null());
    if hinstance.is_null() {
        return 1;
    }

    let icon = {
        let loaded = LoadIconW(hinstance, win::int_resource(1));
        if loaded.is_null() {
            LoadIconW(ptr::null_mut(), IDI_APPLICATION)
        } else {
            loaded
        }
    };

    let class_name = win::wide(CLASS_NAME);
    let menu = menu::create_menu();
    if menu.is_null() {
        show_fatal("Nodi could not create its menu.");
        return 1;
    }
    let accel = menu::create_accel();

    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: icon,
        hCursor: windows_sys::Win32::UI::WindowsAndMessaging::LoadCursorW(
            ptr::null_mut(),
            windows_sys::Win32::UI::WindowsAndMessaging::IDC_ARROW,
        ),
        hbrBackground: (COLOR_3DFACE as usize + 1) as HBRUSH,
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: icon,
    };
    if RegisterClassExW(&class) == 0 {
        show_fatal("Nodi could not register its window class.");
        if !accel.is_null() {
            DestroyAcceleratorTable(accel);
        }
        destroy_menu_compat(menu);
        return 1;
    }

    let mut state = Box::new(App::new());
    state.hinstance = hinstance;
    state.menu = menu;
    state.accel = accel;
    let state_ptr = Box::into_raw(state);

    let dpi = GetDpiForSystem();
    let width = win::mul_div(1100, dpi as i32, 96);
    let height = win::mul_div(720, dpi as i32, 96);
    let title = win::wide("Nodi");
    let hwnd = CreateWindowExW(
        0,
        class_name.as_ptr(),
        title.as_ptr(),
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        width,
        height,
        ptr::null_mut(),
        menu,
        hinstance,
        state_ptr as *const core::ffi::c_void,
    );

    if hwnd.is_null() {
        drop(Box::from_raw(state_ptr));
        if !accel.is_null() {
            DestroyAcceleratorTable(accel);
        }
        destroy_menu_compat(menu);
        return 1;
    }

    center_window(hwnd);
    ShowWindow(hwnd, SW_SHOW);
    UpdateWindow(hwnd);

    let mut message: MSG = std::mem::zeroed();
    loop {
        let result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
        if result <= 0 {
            break;
        }
        if TranslateAcceleratorW(hwnd, accel, &message) == 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    if !accel.is_null() {
        DestroyAcceleratorTable(accel);
    }
    destroy_menu_compat(menu);

    message.wParam as i32
}

fn destroy_menu_compat(menu: HMENU) {
    if !menu.is_null() {
        // SAFETY: the handle is either still valid or already destroyed by the
        // window; `DestroyMenu` validates its argument and returns FALSE then.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::DestroyMenu(menu);
        }
    }
}

unsafe fn show_fatal(text: &str) {
    let text = win::wide(text);
    let caption = win::wide("Nodi");
    MessageBoxW(
        ptr::null_mut(),
        text.as_ptr(),
        caption.as_ptr(),
        MB_OK | MB_ICONERROR,
    );
}

unsafe fn center_window(hwnd: HWND) {
    let mut work: RECT = std::mem::zeroed();
    if SystemParametersInfoW(
        SPI_GETWORKAREA,
        0,
        &mut work as *mut _ as *mut core::ffi::c_void,
        0,
    ) == 0
    {
        return;
    }
    let mut window: RECT = std::mem::zeroed();
    if GetWindowRect(hwnd, &mut window) == 0 {
        return;
    }
    let width = win::rect_width(&window);
    let height = win::rect_height(&window);
    let x = work.left + (win::rect_width(&work) - width) / 2;
    let y = work.top + (win::rect_height(&work) - height) / 2;
    SetWindowPos(
        hwnd,
        ptr::null_mut(),
        x,
        y,
        0,
        0,
        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
}

unsafe fn app_mut<'a>(hwnd: HWND) -> Option<&'a mut App> {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if pointer.is_null() {
        None
    } else {
        Some(&mut *pointer)
    }
}

unsafe fn create_ui(app: &mut App) -> bool {
    app.dpi = GetDpiForWindow(app.hwnd);
    if app.dpi == 0 {
        app.dpi = 96;
    }
    app.sidebar_width = app.scale(240);
    app::recreate_fonts(app);

    app.toolbar = toolbar::create(app);
    app.sidebar_title = tree::create_header(app);
    app.tree = tree::create(app);
    app.tabctrl = tabs::create(app);
    app.status = status::create(app);

    if app.toolbar.is_null()
        || app.sidebar_title.is_null()
        || app.tree.is_null()
        || app.tabctrl.is_null()
        || app.status.is_null()
    {
        return false;
    }

    tree::populate(app.tree);
    app::apply_fonts(app);
    toolbar::refresh_height(app);
    app::new_tab(app);
    layout(app);
    // Pane geometry only exists after the first layout pass, so set the static
    // labels afterwards; otherwise `SB_SETPARTS` drops texts written earlier.
    status::init(app);
    true
}

/// Positions every child window for the current client size.
pub fn layout(app: &mut App) {
    if app.hwnd.is_null() {
        return;
    }

    // SAFETY: `app.hwnd` is our frame window; all child handles are ours.
    unsafe {
        let mut client: RECT = std::mem::zeroed();
        if GetClientRect(app.hwnd, &mut client) == 0 {
            return;
        }
        let width = win::rect_width(&client);
        let height = win::rect_height(&client);
        if width <= 0 || height <= 0 {
            return;
        }

        let mut top = 0;
        let mut bottom = height;

        if app.status_visible {
            let status_height = status::measure_height(app);
            SetWindowPos(
                app.status,
                ptr::null_mut(),
                0,
                height - status_height,
                width,
                status_height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            ShowWindow(app.status, SW_SHOW);
            bottom = height - status_height;
        } else {
            ShowWindow(
                app.status,
                windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE,
            );
        }

        if app.toolbar_visible {
            let toolbar_height = app.toolbar_height.max(app.scale(28));
            SetWindowPos(
                app.toolbar,
                ptr::null_mut(),
                0,
                0,
                width,
                toolbar_height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            ShowWindow(app.toolbar, SW_SHOW);
            top = toolbar_height;
        } else {
            ShowWindow(
                app.toolbar,
                windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE,
            );
        }

        let content_height = (bottom - top).max(0);
        let min_sidebar = app.scale(120);
        let max_by_fraction = width * 60 / 100;
        let max_by_editor = width - SPLITTER_WIDTH - app.scale(160);
        let max_sidebar = max_by_fraction.min(max_by_editor).max(min_sidebar);
        let sidebar = app.sidebar_width.clamp(min_sidebar, max_sidebar);
        app.sidebar_width = sidebar;

        let header_height = app.scale(24).max(app.scale(18));
        let tree_height = (content_height - header_height).max(0);
        SetWindowPos(
            app.sidebar_title,
            ptr::null_mut(),
            0,
            top,
            sidebar,
            header_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        SetWindowPos(
            app.tree,
            ptr::null_mut(),
            0,
            top + header_height,
            sidebar,
            tree_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

        let right_x = sidebar + SPLITTER_WIDTH;
        let right_width = (width - right_x).max(0);
        let tab_height = tabs::measure_height(app).min(content_height);
        SetWindowPos(
            app.tabctrl,
            ptr::null_mut(),
            right_x,
            top,
            right_width,
            tab_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );

        let editor_top = top + tab_height;
        let editor_height = (content_height - tab_height).max(0);
        for tab in &app.tabs {
            SetWindowPos(
                tab.editor,
                ptr::null_mut(),
                right_x,
                editor_top,
                right_width,
                editor_height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }

        status::layout_parts(app, width);

        let splitter = RECT {
            left: sidebar,
            top,
            right: right_x,
            bottom,
        };
        InvalidateRect(app.hwnd, &splitter, 0);
    }
}

fn cursor_on_splitter(app: &App) -> bool {
    // SAFETY: both calls only write into the stack `POINT`.
    unsafe {
        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point) == 0 {
            return false;
        }
        if ScreenToClient(app.hwnd, &mut point) == 0 {
            return false;
        }
        let tolerance = app.scale(2);
        point.x >= app.sidebar_width - tolerance
            && point.x <= app.sidebar_width + SPLITTER_WIDTH + tolerance
    }
}

/// Fills the frame background and draws the splitter line.
unsafe fn paint_splitter(app: &App) {
    let mut paint: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(app.hwnd, &mut paint);
    if hdc.is_null() {
        return;
    }

    let mut client: RECT = std::mem::zeroed();
    GetClientRect(app.hwnd, &mut client);
    FillRect(hdc, &client, GetSysColorBrush(COLOR_3DFACE));

    let x = app.sidebar_width + SPLITTER_WIDTH / 2;
    let pen = CreatePen(PS_SOLID, 1, GetSysColor(COLOR_3DSHADOW));
    if !pen.is_null() {
        let previous = SelectObject(hdc, pen);
        MoveToEx(hdc, x, 0, ptr::null_mut());
        LineTo(hdc, x, client.bottom);
        SelectObject(hdc, previous);
        DeleteObject(pen);
    }

    EndPaint(app.hwnd, &paint);
}

pub unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = lparam as *const CREATESTRUCTW;
            if create.is_null() {
                return 0;
            }
            let state = (*create).lpCreateParams as *mut App;
            if state.is_null() {
                return 0;
            }
            (*state).hwnd = hwnd;
            (*state).hinstance = (*create).hInstance;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
            // Let the default handler finish NCCREATE: it stores the creation
            // title (`CREATESTRUCTW::lpszName`) and builds the window's text
            // buffer. Returning TRUE without it leaves the caption empty.
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_CREATE => match app_mut(hwnd) {
            Some(app) => {
                if create_ui(app) {
                    app.created = true;
                    0
                } else {
                    -1
                }
            }
            None => -1,
        },

        WM_SIZE => {
            if wparam != SIZE_MINIMIZED as usize {
                if let Some(app) = app_mut(hwnd) {
                    layout(app);
                }
            }
            0
        }

        WM_DPICHANGED => {
            let new_dpi = (wparam & 0xFFFF) as u32;
            let suggested = lparam as *const RECT;
            if !suggested.is_null() {
                let rect = *suggested;
                SetWindowPos(
                    hwnd,
                    ptr::null_mut(),
                    rect.left,
                    rect.top,
                    win::rect_width(&rect),
                    win::rect_height(&rect),
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
            if let Some(app) = app_mut(hwnd) {
                let old_dpi = app.dpi;
                if new_dpi != 0 {
                    app.dpi = new_dpi;
                }
                if old_dpi != 0 && old_dpi != app.dpi {
                    app.sidebar_width =
                        win::mul_div(app.sidebar_width, app.dpi as i32, old_dpi as i32);
                }
                app::recreate_fonts(app);
                toolbar::refresh_height(app);
                layout(app);
            }
            0
        }

        WM_GETMINMAXINFO => {
            let result = DefWindowProcW(hwnd, msg, wparam, lparam);
            if let Some(app) = app_mut(hwnd) {
                let info = lparam as *mut MINMAXINFO;
                if !info.is_null() {
                    (*info).ptMinTrackSize.x = app.scale(760);
                    (*info).ptMinTrackSize.y = app.scale(480);
                }
            }
            result
        }

        WM_COMMAND => {
            let id = win::loword(wparam);
            let code = win::hiword(wparam);
            let child = lparam as HWND;
            if let Some(app) = app_mut(hwnd) {
                if code == editor::EN_CHANGE_NOTIFY && app.tab_index(child).is_some() {
                    app::on_editor_change(app, child);
                } else if code == 0 {
                    menu::on_command(app, id);
                }
            }
            0
        }

        WM_NOTIFY => {
            let header = lparam as *const NMHDR;
            if header.is_null() {
                return 0;
            }
            let header = &*header;
            if let Some(app) = app_mut(hwnd) {
                if header.hwndFrom == app.tabctrl && header.code == TCN_SELCHANGE {
                    app::on_tab_selchange(app);
                } else if header.code == editor::EN_SELCHANGE_NOTIFY
                    && app.tab_index(header.hwndFrom).is_some()
                {
                    status::update_caret(app);
                }
            }
            0
        }

        WM_PAINT => {
            if let Some(app) = app_mut(hwnd) {
                paint_splitter(app);
            } else {
                let mut paint: PAINTSTRUCT = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut paint);
                EndPaint(hwnd, &paint);
                let _ = hdc;
            }
            0
        }

        WM_ERASEBKGND => {
            // The toolbar is created without `CCS_*` background painting and the
            // splitter is drawn by us, so the frame must actually fill itself.
            // Returning 1 without painting would leave stale pixels behind the
            // transparent toolbar strip.
            let hdc = wparam as HDC;
            if !hdc.is_null() {
                let mut client: RECT = std::mem::zeroed();
                if GetClientRect(hwnd, &mut client) != 0 {
                    FillRect(hdc, &client, GetSysColorBrush(COLOR_3DFACE));
                }
            }
            1
        }

        WM_SETCURSOR => {
            if win::loword(lparam as usize) as u32 == HTCLIENT {
                if let Some(app) = app_mut(hwnd) {
                    if cursor_on_splitter(app) {
                        SetCursor(app.hcursor_we);
                        return 1;
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_LBUTTONDOWN => {
            if let Some(app) = app_mut(hwnd) {
                if cursor_on_splitter(app) {
                    app.splitter_drag = true;
                    SetCapture(hwnd);
                    SetCursor(app.hcursor_we);
                    return 0;
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_MOUSEMOVE => {
            if let Some(app) = app_mut(hwnd) {
                if app.splitter_drag {
                    let mut client: RECT = std::mem::zeroed();
                    if GetClientRect(hwnd, &mut client) != 0 {
                        let width = win::rect_width(&client);
                        let min_sidebar = app.scale(120);
                        let max_sidebar = (width * 60 / 100)
                            .min(width - SPLITTER_WIDTH - app.scale(160))
                            .max(min_sidebar);
                        let requested = win::x_from_lparam(lparam);
                        let sidebar = requested.clamp(min_sidebar, max_sidebar);
                        if sidebar != app.sidebar_width {
                            app.sidebar_width = sidebar;
                            layout(app);
                        }
                    }
                    return 0;
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_LBUTTONUP => {
            if let Some(app) = app_mut(hwnd) {
                if app.splitter_drag {
                    app.splitter_drag = false;
                    ReleaseCapture();
                    return 0;
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_CAPTURECHANGED => {
            if let Some(app) = app_mut(hwnd) {
                app.splitter_drag = false;
            }
            0
        }

        WM_SETFOCUS => {
            if let Some(app) = app_mut(hwnd) {
                if let Some(active) = app.active_editor() {
                    SetFocus(active);
                }
            }
            0
        }

        WM_CLOSE => {
            let allowed = app_mut(hwnd)
                .map(|app| app::close_requested(app))
                .unwrap_or(true);
            if allowed {
                DestroyWindow(hwnd);
            }
            0
        }

        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }

        WM_NCDESTROY => {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            if !pointer.is_null() {
                let app = &mut *pointer;
                app::destroy_fonts(app);
                if app.created {
                    drop(Box::from_raw(pointer));
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
