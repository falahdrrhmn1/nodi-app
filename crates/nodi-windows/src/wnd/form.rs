//! A small, DPI-aware modal form: labelled edit fields plus OK/Cancel and
//! optional extra buttons. Used by the Image Size, Insert Table, Link,
//! Formula and Go To Line dialogs.

use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateFontIndirectW, DeleteObject, HFONT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetActiveWindow, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsDialogMessageW,
    IsWindow, LoadCursorW, RegisterClassExW, SendMessageW, SetWindowLongPtrW, SetWindowTextW,
    SystemParametersInfoW, TranslateMessage, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CREATESTRUCTW,
    ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_WANTRETURN, GWLP_USERDATA, IDCANCEL,
    IDC_ARROW, IDOK, MSG, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, WM_CLOSE, WM_COMMAND,
    WM_CREATE, WM_NCCREATE, WM_NCDESTROY, WM_SETFONT, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD,
    WS_EX_CONTROLPARENT, WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
    WS_VSCROLL,
};

use crate::win;

const CLASS: &str = "NodiForm";
const FIRST_FIELD_ID: i32 = 1000;
const FIRST_EXTRA_ID: i32 = 2000;
const FIRST_CHECK_ID: i32 = 3000;
const EM_SETSEL: u32 = 0x00B1;
const BS_AUTOCHECKBOX: u32 = 0x0000_0003;
const BM_SETCHECK: u32 = 0x00F1;
const BM_GETCHECK: u32 = 0x00F0;
const BST_CHECKED: usize = 1;

/// One labelled input.
pub struct Field {
    pub label: String,
    pub value: String,
    /// Height in text lines (1 = single-line edit).
    pub lines: i32,
}

impl Field {
    pub fn new(label: &str, value: impl Into<String>) -> Self {
        Self {
            label: label.to_owned(),
            value: value.into(),
            lines: 1,
        }
    }

    pub fn multiline(label: &str, value: impl Into<String>, lines: i32) -> Self {
        Self {
            lines: lines.max(2),
            ..Self::new(label, value)
        }
    }
}

/// Extra button: gets the owner window and the current field values and may
/// rewrite them (e.g. "Browse..." fills in a path).
pub type ExtraAction = Box<dyn FnMut(HWND, &mut [String])>;

pub struct Form {
    pub title: String,
    /// Optional explanatory text above the fields.
    pub note: String,
    pub fields: Vec<Field>,
    /// Check boxes below the fields: (label, checked by default).
    pub checkboxes: Vec<(String, bool)>,
    pub extras: Vec<(String, ExtraAction)>,
}

impl Form {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            note: String::new(),
            fields: Vec::new(),
            checkboxes: Vec::new(),
            extras: Vec::new(),
        }
    }

    pub fn note(mut self, note: &str) -> Self {
        self.note = note.to_owned();
        self
    }

    pub fn field(mut self, field: Field) -> Self {
        self.fields.push(field);
        self
    }

    pub fn checkbox(mut self, label: &str, checked: bool) -> Self {
        self.checkboxes.push((label.to_owned(), checked));
        self
    }

    pub fn extra(mut self, label: &str, action: ExtraAction) -> Self {
        self.extras.push((label.to_owned(), action));
        self
    }

    /// Shows the form modally. `Some(values)` on OK, `None` on Cancel.
    pub fn show(self, owner: HWND) -> Option<Vec<String>> {
        // SAFETY: the window lives only inside this call; the state box is
        // reclaimed after the modal loop ends.
        unsafe { run(owner, self) }
    }
}

struct State {
    form: Form,
    edits: Vec<HWND>,
    checks: Vec<HWND>,
    font: HFONT,
    result: Option<Vec<String>>,
    dpi: i32,
}

unsafe fn register() {
    let name = win::wide(CLASS);
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: 0,
        lpfnWndProc: Some(proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: GetModuleHandleW(ptr::null()),
        hIcon: ptr::null_mut(),
        hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
        hbrBackground: 16 as _, // COLOR_BTNFACE + 1
        lpszMenuName: ptr::null(),
        lpszClassName: name.as_ptr(),
        hIconSm: ptr::null_mut(),
    };
    RegisterClassExW(&class);
}

fn scale(value: i32, dpi: i32) -> i32 {
    win::mul_div(value, dpi, 96)
}

/// Layout in 96-DPI units.
const MARGIN: i32 = 12;
const LABEL_W: i32 = 110;
const EDIT_W: i32 = 330;
const ROW_H: i32 = 24;
const LINE_H: i32 = 18;
const BUTTON_W: i32 = 88;
const BUTTON_H: i32 = 26;

fn note_lines(note: &str) -> i32 {
    if note.is_empty() {
        0
    } else {
        note.lines()
            .map(|line| 1 + line.len() as i32 / 70)
            .sum::<i32>()
    }
}

fn content_height(form: &Form) -> i32 {
    let note = note_lines(&form.note);
    let mut height = MARGIN;
    if note > 0 {
        height += note * LINE_H + 8;
    }
    for field in &form.fields {
        height += field_height(field) + 8;
    }
    height += form.checkboxes.len() as i32 * ROW_H;
    height + BUTTON_H + MARGIN
}

fn field_height(field: &Field) -> i32 {
    if field.lines <= 1 {
        ROW_H
    } else {
        field.lines * LINE_H + 8
    }
}

unsafe fn run(owner: HWND, form: Form) -> Option<Vec<String>> {
    register();
    let dpi = match GetDpiForWindow(owner) {
        0 => 96,
        value => value as i32,
    };
    let width = scale(MARGIN * 2 + LABEL_W + EDIT_W, dpi) + scale(16, dpi);
    let height = scale(content_height(&form), dpi) + scale(40, dpi);
    let mut owner_rect: RECT = std::mem::zeroed();
    GetWindowRect(owner, &mut owner_rect);
    let x = owner_rect.left + (owner_rect.right - owner_rect.left - width) / 2;
    let y = owner_rect.top + (owner_rect.bottom - owner_rect.top - height) / 3;

    let title = win::wide(&form.title);
    let state = Box::into_raw(Box::new(State {
        form,
        edits: Vec::new(),
        checks: Vec::new(),
        font: ptr::null_mut(),
        result: None,
        dpi,
    }));
    let class = win::wide(CLASS);
    let dialog = CreateWindowExW(
        WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
        class.as_ptr(),
        title.as_ptr(),
        WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
        x,
        y,
        width,
        height,
        owner,
        ptr::null_mut(),
        GetModuleHandleW(ptr::null()),
        state as *const core::ffi::c_void,
    );
    if dialog.is_null() {
        drop(Box::from_raw(state));
        return None;
    }

    EnableWindow(owner, 0);
    let mut message: MSG = std::mem::zeroed();
    while IsWindow(dialog) != 0 {
        if GetMessageW(&mut message, ptr::null_mut(), 0, 0) <= 0 {
            break;
        }
        if IsDialogMessageW(dialog, &message) == 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    EnableWindow(owner, 1);
    SetActiveWindow(owner);

    let state = Box::from_raw(state);
    if !state.font.is_null() {
        DeleteObject(state.font);
    }
    state.result
}

unsafe fn make_font(dpi: i32) -> HFONT {
    let mut metrics: NONCLIENTMETRICSW = std::mem::zeroed();
    metrics.cbSize = std::mem::size_of::<NONCLIENTMETRICSW>() as u32;
    SystemParametersInfoW(
        SPI_GETNONCLIENTMETRICS,
        metrics.cbSize,
        &mut metrics as *mut _ as *mut core::ffi::c_void,
        0,
    );
    let mut font = metrics.lfMessageFont;
    font.lfHeight = scale(font.lfHeight, dpi);
    CreateFontIndirectW(&font)
}

#[allow(clippy::too_many_arguments)]
unsafe fn control(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    id: i32,
    rect: (i32, i32, i32, i32),
    dpi: i32,
    font: HFONT,
) -> HWND {
    let class = win::wide(class);
    let text = win::wide(text);
    let ex_style = if style & WS_BORDER != 0 { 0x200 } else { 0 }; // WS_EX_CLIENTEDGE
    let handle = CreateWindowExW(
        ex_style,
        class.as_ptr(),
        text.as_ptr(),
        (style & !WS_BORDER) | WS_CHILD | WS_VISIBLE,
        scale(rect.0, dpi),
        scale(rect.1, dpi),
        scale(rect.2, dpi),
        scale(rect.3, dpi),
        parent,
        id as isize as _,
        GetModuleHandleW(ptr::null()),
        ptr::null(),
    );
    if !handle.is_null() && !font.is_null() {
        SendMessageW(handle, WM_SETFONT, font as usize, 1);
    }
    handle
}

unsafe fn state<'a>(hwnd: HWND) -> Option<&'a mut State> {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    pointer.as_mut()
}

unsafe fn read(edit: HWND) -> String {
    let length = GetWindowTextLengthW(edit);
    let mut buffer = vec![0u16; length as usize + 1];
    let copied = GetWindowTextW(edit, buffer.as_mut_ptr(), buffer.len() as i32);
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

unsafe fn values(state: &State) -> Vec<String> {
    let mut values: Vec<String> = state.edits.iter().map(|edit| read(*edit)).collect();
    for check in &state.checks {
        // SAFETY: BM_GETCHECK only reads the button's state.
        let checked = SendMessageW(*check, BM_GETCHECK, 0, 0) as usize == BST_CHECKED;
        values.push(if checked { "1" } else { "0" }.to_owned());
    }
    values
}

unsafe fn build(hwnd: HWND, state: &mut State) {
    let dpi = state.dpi;
    state.font = make_font(dpi);
    let font = state.font;
    let mut y = MARGIN;
    let inner_w = LABEL_W + EDIT_W;
    if !state.form.note.is_empty() {
        let lines = note_lines(&state.form.note);
        control(
            hwnd,
            "Static",
            &state.form.note,
            0,
            -1,
            (MARGIN, y, inner_w, lines * LINE_H),
            dpi,
            font,
        );
        y += lines * LINE_H + 8;
    }
    for (index, field) in state.form.fields.iter().enumerate() {
        let height = field_height(field);
        control(
            hwnd,
            "Static",
            &field.label,
            0,
            -1,
            (MARGIN, y + 4, LABEL_W - 8, LINE_H),
            dpi,
            font,
        );
        let mut style = WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL as u32;
        if field.lines > 1 {
            style |= (ES_MULTILINE | ES_WANTRETURN | ES_AUTOVSCROLL) as u32 | WS_VSCROLL;
        }
        let value = field.value.replace("\r\n", "\n").replace('\n', "\r\n");
        let edit = control(
            hwnd,
            "Edit",
            &value,
            style,
            FIRST_FIELD_ID + index as i32,
            (MARGIN + LABEL_W, y, EDIT_W, height),
            dpi,
            font,
        );
        state.edits.push(edit);
        y += height + 8;
    }

    for (index, (label, checked)) in state.form.checkboxes.iter().enumerate() {
        let check = control(
            hwnd,
            "Button",
            label,
            BS_AUTOCHECKBOX | WS_TABSTOP,
            FIRST_CHECK_ID + index as i32,
            (MARGIN + LABEL_W, y + 2, EDIT_W, ROW_H - 4),
            dpi,
            font,
        );
        if *checked && !check.is_null() {
            // SAFETY: BM_SETCHECK only touches the button's state.
            unsafe { SendMessageW(check, BM_SETCHECK, BST_CHECKED, 0) };
        }
        state.checks.push(check);
        y += ROW_H;
    }

    // Buttons: extras on the left, OK/Cancel on the right.
    let mut x = MARGIN;
    for (index, (label, _)) in state.form.extras.iter().enumerate() {
        let width = BUTTON_W.max(label.chars().count() as i32 * 7 + 16);
        control(
            hwnd,
            "Button",
            label,
            BS_PUSHBUTTON as u32 | WS_TABSTOP,
            FIRST_EXTRA_ID + index as i32,
            (x, y, width, BUTTON_H),
            dpi,
            font,
        );
        x += width + 6;
    }
    let right = MARGIN + inner_w;
    control(
        hwnd,
        "Button",
        "OK",
        BS_DEFPUSHBUTTON as u32 | WS_TABSTOP,
        IDOK,
        (right - BUTTON_W * 2 - 6, y, BUTTON_W, BUTTON_H),
        dpi,
        font,
    );
    control(
        hwnd,
        "Button",
        "Cancel",
        BS_PUSHBUTTON as u32 | WS_TABSTOP,
        IDCANCEL,
        (right - BUTTON_W, y, BUTTON_W, BUTTON_H),
        dpi,
        font,
    );

    if let Some(first) = state.edits.first() {
        SetFocus(*first);
        SendMessageW(*first, EM_SETSEL, 0, -1);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = lparam as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CREATE => {
            if let Some(state) = state(hwnd) {
                build(hwnd, state);
            }
            0
        }
        WM_COMMAND => {
            let id = win::loword(wparam) as i32;
            if id == IDOK {
                if let Some(state) = state(hwnd) {
                    state.result = Some(values(state));
                }
                DestroyWindow(hwnd);
            } else if id == IDCANCEL {
                DestroyWindow(hwnd);
            } else if id >= FIRST_CHECK_ID {
                // Check-box notification: the state is read on OK.
            } else if id >= FIRST_EXTRA_ID {
                if let Some(state) = state(hwnd) {
                    let index = (id - FIRST_EXTRA_ID) as usize;
                    let mut current = values(state);
                    if let Some((_, action)) = state.form.extras.get_mut(index) {
                        action(hwnd, &mut current);
                        for (edit, value) in state.edits.iter().zip(&current) {
                            let text =
                                win::wide(&value.replace("\r\n", "\n").replace('\n', "\r\n"));
                            SetWindowTextW(*edit, text.as_ptr());
                        }
                    }
                }
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
