//! Frame window: creation, message loop, layout and splitter handling.

pub mod caret;
pub mod clipboard;
pub mod context;
pub mod dialogs;
pub mod editkeys;
pub mod editor;
pub mod editword;
pub mod find;
pub mod form;
pub mod format;
pub mod formula;
pub mod gutter;
pub mod image;
pub mod import;
pub mod link;
pub mod menu;
pub mod net;
pub mod outline;
pub mod pattern;
pub mod status;
pub mod table;
pub mod tabs;
pub mod theme;
pub mod toolbar;
pub mod tree;

use std::mem::size_of;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CreatePen, DeleteObject, EndPaint, FillRect, GetSysColorBrush, InvalidateRect,
    LineTo, MoveToEx, ScreenToClient, SelectObject, SetBkColor, SetTextColor, UpdateWindow,
    COLOR_3DFACE, HBRUSH, HDC, PAINTSTRUCT, PS_SOLID,
};
use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows_sys::Win32::System::Ole::OleInitialize;
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, DRAWITEMSTRUCT, ICC_BAR_CLASSES, ICC_STANDARD_CLASSES, ICC_TAB_CLASSES,
    ICC_TREEVIEW_CLASSES, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, NMHDR, TCN_SELCHANGE,
};
use windows_sys::Win32::UI::HiDpi::{
    GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, ReleaseCapture, SetCapture, SetFocus,
};
use windows_sys::Win32::UI::WindowsAndMessaging::DestroyAcceleratorTable;
use windows_sys::Win32::UI::WindowsAndMessaging::HMENU;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos,
    GetMessageW, GetWindowLongPtrW, GetWindowRect, LoadIconW, MessageBoxW, PostQuitMessage,
    RegisterClassExW, SetCursor, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    SystemParametersInfoW, TranslateAcceleratorW, TranslateMessage, CREATESTRUCTW, CS_HREDRAW,
    CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HTCLIENT, IDI_APPLICATION, MB_ICONERROR, MB_OK,
    MINMAXINFO, MSG, SIZE_MINIMIZED, SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
    SW_SHOW, WM_CAPTURECHANGED, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU, WM_COPYDATA, WM_CREATE,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_DRAWITEM, WM_ENDSESSION, WM_ERASEBKGND,
    WM_GETMINMAXINFO, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY,
    WM_NOTIFY, WM_PAINT, WM_QUERYENDSESSION, WM_SETCURSOR, WM_SETFOCUS, WM_SIZE, WM_TIMER,
    WNDCLASSEXW, WS_CLIPCHILDREN, WS_OVERLAPPEDWINDOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, IsDialogMessageW};

use nodi_core::autosave::SaveResult;

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
    // OLE powers Rich Edit's drag-and-drop (moving a picture or text inside a
    // note) and its clipboard data objects. Failure only disables those.
    let _ = OleInitialize(ptr::null());

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
        if editword::handle(&message) {
            continue;
        }
        // The modeless Find/Replace dialog handles its own keys (Tab, Enter).
        let find_dialog = find::dialog();
        if !find_dialog.is_null() && IsDialogMessageW(find_dialog, &message) != 0 {
            continue;
        }
        // While a tree label is being renamed, keys belong to that edit box
        // (Ctrl+Z, Ctrl+C, F2...), not to the note commands.
        let renaming = is_plain_edit(GetFocus());
        if renaming || TranslateAcceleratorW(hwnd, accel, &message) == 0 {
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

/// True for a standard "Edit" control (the tree's inline rename box).
fn is_plain_edit(window: HWND) -> bool {
    if window.is_null() {
        return false;
    }
    let mut name = [0u16; 16];
    // SAFETY: `name` is a valid buffer of the given length.
    let length = unsafe { GetClassNameW(window, name.as_mut_ptr(), name.len() as i32) };
    String::from_utf16_lossy(&name[..length.max(0) as usize]).eq_ignore_ascii_case("Edit")
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

pub(crate) unsafe fn app_mut<'a>(hwnd: HWND) -> Option<&'a mut App> {
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

    toolbar::create(app);
    app.sidebar_title = tree::create_header(app);
    app.tree_view = tree::create(app);
    app.tabctrl = tabs::create(app);
    app.status = status::create(app);
    app.gutter = gutter::create(app);

    if app.toolbar.is_null()
        || app.formatbar.is_null()
        || app.sidebar_title.is_null()
        || app.tree_view.is_null()
        || app.tabctrl.is_null()
        || app.status.is_null()
    {
        return false;
    }

    tree::create_images(app);
    tree::install_subclass(app);
    tabs::install_subclass(app);
    app::apply_fonts(app);
    toolbar::refresh_height(app);

    // Resolve the data dir, repair leftovers, load/repair the tree and restore
    // the previous session before the first real layout pass.
    if !app.startup() {
        return false;
    }

    // Settings are known now: theme the chrome before the first paint.
    theme::apply(app);

    layout(app);
    // Pane geometry only exists after the first layout pass, so set the static
    // labels afterwards; otherwise `SB_SETPARTS` drops texts written earlier.
    status::init(app);
    app.refresh_active_ui();
    true
}

/// Positions every child window for the current client size.
/// A `DeferWindowPos` batch; falls back to `SetWindowPos` when the batch
/// cannot be created. The moves happen together when it is dropped.
struct Batch(windows_sys::Win32::UI::WindowsAndMessaging::HDWP);

impl Batch {
    unsafe fn new(count: i32) -> Self {
        Self(windows_sys::Win32::UI::WindowsAndMessaging::BeginDeferWindowPos(count))
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn pos(
        &mut self,
        window: HWND,
        after: HWND,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) {
        if window.is_null() {
            return;
        }
        if !self.0.is_null() {
            self.0 = windows_sys::Win32::UI::WindowsAndMessaging::DeferWindowPos(
                self.0, window, after, x, y, width, height, flags,
            );
        }
        if self.0.is_null() {
            SetWindowPos(window, after, x, y, width, height, flags);
        }
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the handle came from BeginDeferWindowPos/DeferWindowPos.
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::EndDeferWindowPos(self.0);
            }
        }
    }
}

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

            let format_height = app.formatbar_height.max(app.scale(32));
            SetWindowPos(
                app.formatbar,
                ptr::null_mut(),
                0,
                toolbar_height,
                width,
                format_height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            ShowWindow(app.formatbar, SW_SHOW);
            toolbar::relayout(app);
            top = toolbar_height + format_height;
        } else {
            for bar in [app.toolbar, app.formatbar] {
                if !bar.is_null() {
                    ShowWindow(bar, windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE);
                }
            }
        }

        let content_height = (bottom - top).max(0);
        let min_sidebar = app.scale(120);
        let max_by_fraction = width * 60 / 100;
        let max_by_editor = width - SPLITTER_WIDTH - app.scale(160);
        let max_sidebar = max_by_fraction.min(max_by_editor).max(min_sidebar);
        let sidebar = app.sidebar_width.clamp(min_sidebar, max_sidebar);
        app.sidebar_width = sidebar;

        // The content area moves as one batch, so dragging the splitter does
        // not show the tree, tabs and editor at different widths in between.
        let mut batch = Batch::new(5);
        let header_height = app.scale(24).max(app.scale(18));
        let tree_height = (content_height - header_height).max(0);
        batch.pos(
            app.sidebar_title,
            ptr::null_mut(),
            0,
            top,
            sidebar,
            header_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        batch.pos(
            app.tree_view,
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
        batch.pos(
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
        let gutter_width = gutter::width(app).min(right_width);
        batch.pos(
            app.gutter,
            ptr::null_mut(),
            right_x,
            editor_top,
            gutter_width,
            editor_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        // Hidden editors are sized when they become active (see
        // `App::activate_tab_impl`), so many open tabs stay cheap.
        if let Some(editor) = app.active_editor() {
            batch.pos(
                editor,
                ptr::null_mut(),
                right_x + gutter_width,
                editor_top,
                right_width - gutter_width,
                editor_height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }

        drop(batch);

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
    let brush = if app.background_brush.is_null() {
        GetSysColorBrush(COLOR_3DFACE)
    } else {
        app.background_brush
    };
    FillRect(hdc, &client, brush);

    let x = app.sidebar_width + SPLITTER_WIDTH / 2;
    let pen = CreatePen(PS_SOLID, 1, app.palette.border);
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
                    // Files dragged from Explorer: import (or pictures into
                    // the note).
                    windows_sys::Win32::UI::Shell::DragAcceptFiles(hwnd, 1);
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
                if crate::testhook::enabled() {
                    crate::testhook::note_command(id, code);
                }
                if code == editor::EN_CHANGE_NOTIFY && app.tab_index(child).is_some() {
                    app::on_editor_change(app, child);
                } else if code == windows_sys::Win32::UI::WindowsAndMessaging::CBN_SELCHANGE as u16
                    && toolbar::is_format_combo(app, child)
                {
                    toolbar::on_combo(app, child);
                } else if (code == windows_sys::Win32::UI::WindowsAndMessaging::CBN_SELENDOK as u16
                    || code == toolbar::COMBO_COMMIT
                    || code == toolbar::COMBO_CANCEL)
                    && (child == app.font_combo || child == app.size_combo)
                {
                    toolbar::on_combo_commit(app, child, code);
                } else if code == 0 || (code == 1 && child.is_null()) {
                    // 0 = menu / toolbar button, 1 = accelerator (shortcut).
                    menu::on_command(app, id);
                }
            }
            0
        }

        WM_TIMER => {
            if let Some(app) = app_mut(hwnd) {
                app.on_timer(wparam);
            }
            0
        }

        windows_sys::Win32::UI::WindowsAndMessaging::WM_DROPFILES => {
            if let Some(app) = app_mut(hwnd) {
                import::on_drop(app, wparam as windows_sys::Win32::UI::Shell::HDROP);
            }
            0
        }

        import::WM_APP_IMPORT_DONE => {
            let raw = lparam as *mut import::Converted;
            if !raw.is_null() {
                // SAFETY: the import worker boxed this result for us.
                let converted = *Box::from_raw(raw);
                if let Some(app) = app_mut(hwnd) {
                    import::finish(app, converted);
                }
            }
            0
        }

        app::WM_APP_SAVE_RESULT => {
            let raw = lparam as *mut SaveResult;
            if !raw.is_null() {
                // SAFETY: the writer thread boxed this `SaveResult` for us.
                let result = *Box::from_raw(raw);
                if let Some(app) = app_mut(hwnd) {
                    app.on_save_result(result);
                }
            }
            0
        }

        WM_COPYDATA if crate::testhook::enabled() => {
            let data = lparam as *const COPYDATASTRUCT;
            match app_mut(hwnd) {
                Some(app) => LRESULT::from(crate::testhook::handle(app, data)),
                None => 0,
            }
        }

        WM_CONTEXTMENU => {
            let mut handled = false;
            if let Some(app) = app_mut(hwnd) {
                if wparam as HWND == app.tree_view {
                    let screen = if lparam == -1 {
                        None
                    } else {
                        Some(POINT {
                            x: win::x_from_lparam(lparam),
                            y: win::y_from_lparam(lparam),
                        })
                    };
                    tree::show_context_menu(app, screen);
                    handled = true;
                } else if wparam as HWND == app.tabctrl {
                    let screen = (lparam != -1).then(|| POINT {
                        x: win::x_from_lparam(lparam),
                        y: win::y_from_lparam(lparam),
                    });
                    let index = screen
                        .and_then(|point| tabs::index_at(app, point))
                        .or_else(|| tabs::cur_sel(app));
                    if let Some(index) = index {
                        tabs::show_context_menu(app, index, screen);
                    }
                    handled = true;
                } else if app.tab_index(wparam as HWND).is_some() {
                    let screen = (lparam != -1).then(|| POINT {
                        x: win::x_from_lparam(lparam),
                        y: win::y_from_lparam(lparam),
                    });
                    context::show_editor_menu(app, wparam as HWND, screen);
                    handled = true;
                }
            }
            if handled {
                0
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }

        WM_NOTIFY => {
            let header = lparam as *const NMHDR;
            if header.is_null() {
                return 0;
            }
            let header = &*header;
            if let Some(app) = app_mut(hwnd) {
                if let Some(result) = theme::on_notify(app, header, lparam) {
                    return result;
                }
                if let Some(result) = tree::notify_result(app, header, lparam) {
                    return result;
                }
                if tree::handle_notify(app, header) {
                    return 0;
                }
                if header.hwndFrom == app.tabctrl && header.code == TCN_SELCHANGE {
                    app::on_tab_selchange(app);
                } else if header.code == editor::EN_SELCHANGE_NOTIFY
                    && app.tab_index(header.hwndFrom).is_some()
                {
                    crate::wnd::format::update_heading_marker(header.hwndFrom);
                    status::update_caret(app);
                    status::schedule_counts(app);
                    toolbar::sync_format(app);
                } else if header.code == link::EN_LINK && app.tab_index(header.hwndFrom).is_some() {
                    // SAFETY: Rich Edit sends an ENLINK for EN_LINK.
                    let info = &*(lparam as *const link::EnLink);
                    return LRESULT::from(link::on_link(app, info));
                } else if header.code == windows_sys::Win32::UI::Controls::TBN_DROPDOWN {
                    // SAFETY: a `TBN_DROPDOWN` carries an `NMTOOLBARW`.
                    toolbar::on_dropdown(app, lparam);
                } else if header.code == toolbar::TTN_GETDISPINFOW {
                    toolbar::tooltip(lparam);
                }
            }
            0
        }

        msg if msg == find::message_id() && msg != 0 => {
            if let Some(app) = app_mut(hwnd) {
                find::on_message(app);
            }
            0
        }

        WM_DRAWITEM => {
            if let Some(app) = app_mut(hwnd) {
                let item = &*(lparam as *const DRAWITEMSTRUCT);
                if item.hwndItem == app.status && status::draw_item(app, item) {
                    return 0;
                }
                if item.hwndItem == app.tabctrl && tabs::draw_item(app, item) {
                    return 0;
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        WM_CTLCOLORSTATIC => {
            if let Some(app) = app_mut(hwnd) {
                // `wparam` is the control's HDC, `lparam` the control.
                if lparam as HWND == app.sidebar_title && !app.background_brush.is_null() {
                    SetTextColor(wparam as HDC, app.palette.window_text);
                    SetBkColor(wparam as HDC, app.palette.window_bg);
                    return app.background_brush as LRESULT;
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
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
                    let brush = match app_mut(hwnd) {
                        Some(app) if !app.background_brush.is_null() => app.background_brush,
                        _ => GetSysColorBrush(COLOR_3DFACE),
                    };
                    FillRect(hdc, &client, brush);
                }
            }
            1
        }

        // Alt+<letter> without a menu entry: close instead of the default
        // beep (Preferences > Sounds off).
        windows_sys::Win32::UI::WindowsAndMessaging::WM_MENUCHAR if !dialogs::sounds() => {
            (windows_sys::Win32::UI::WindowsAndMessaging::MNC_CLOSE as isize) << 16
        }

        theme::WM_UAHDRAWMENU | theme::WM_UAHDRAWMENUITEM => {
            if let Some(app) = app_mut(hwnd) {
                if let Some(result) = theme::on_uah(app, msg, lparam) {
                    return result;
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }

        windows_sys::Win32::UI::WindowsAndMessaging::WM_NCPAINT
        | windows_sys::Win32::UI::WindowsAndMessaging::WM_NCACTIVATE => {
            let result = DefWindowProcW(hwnd, msg, wparam, lparam);
            if let Some(app) = app_mut(hwnd) {
                theme::after_ncpaint(app);
            }
            result
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

        WM_QUERYENDSESSION => {
            if let Some(app) = app_mut(hwnd) {
                app.flush_for_session_end();
            }
            1
        }

        WM_ENDSESSION => {
            if wparam != 0 {
                if let Some(app) = app_mut(hwnd) {
                    app.flush_for_session_end();
                }
            }
            0
        }

        WM_CLOSE => {
            let allowed = app_mut(hwnd).map(|app| app.maybe_close()).unwrap_or(true);
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
                if !app.background_brush.is_null() {
                    // SAFETY: the brush is owned by the app and never reused.
                    DeleteObject(app.background_brush);
                    app.background_brush = std::ptr::null_mut();
                }
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

/// Re-splits the status bar parts for the frame's current width (the counts
/// part grew or shrank).
pub fn relayout_status(app: &App) {
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    // SAFETY: `rect` is a valid out parameter for our frame window.
    if unsafe { GetClientRect(app.hwnd, &mut rect) } != 0 {
        status::layout_parts(app, rect.right - rect.left);
    }
}
