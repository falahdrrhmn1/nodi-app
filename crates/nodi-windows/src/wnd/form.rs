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
    SystemParametersInfoW, TranslateMessage, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CBS_DROPDOWNLIST,
    CBS_HASSTRINGS, CBS_NOINTEGRALHEIGHT, CB_ADDSTRING, CB_GETCURSEL, CB_GETLBTEXT,
    CB_GETLBTEXTLEN, CB_SETCURSEL, CREATESTRUCTW, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE,
    ES_WANTRETURN, GWLP_USERDATA, IDCANCEL, IDC_ARROW, IDOK, LBS_NOINTEGRALHEIGHT, LBS_NOTIFY,
    LB_ADDSTRING, LB_GETCURSEL, MSG, NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, WM_CLOSE,
    WM_COMMAND, WM_CREATE, WM_NCCREATE, WM_NCDESTROY, WM_SETFONT, WNDCLASSEXW, WS_BORDER,
    WS_CAPTION, WS_CHILD, WS_EX_CLIENTEDGE, WS_EX_CONTROLPARENT, WS_EX_DLGMODALFRAME, WS_POPUP,
    WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
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

thread_local! {
    /// Index of the field that last had the keyboard focus.
    static LAST_FIELD: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// The field that had the focus before an extra button was pressed (e.g.
/// which colour Pick Colour... changes).
pub fn last_focused_field() -> Option<usize> {
    LAST_FIELD.with(|last| last.get())
}

/// How a field takes its input.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Input {
    /// Single-line or multiline edit box.
    Edit,
    /// Drop-down list; the value is the selected entry.
    Dropdown,
    /// List box; the value is the selected index, or empty when nothing is
    /// selected.
    List,
}

/// One labelled input.
pub struct Field {
    pub label: String,
    pub value: String,
    /// Height in text lines (1 = single-line edit).
    pub lines: i32,
    pub input: Input,
    /// Entries of a drop-down list or list box.
    pub items: Vec<String>,
}

impl Field {
    pub fn new(label: &str, value: impl Into<String>) -> Self {
        Self {
            label: label.to_owned(),
            value: value.into(),
            lines: 1,
            input: Input::Edit,
            items: Vec::new(),
        }
    }

    pub fn multiline(label: &str, value: impl Into<String>, lines: i32) -> Self {
        Self {
            lines: lines.max(2),
            ..Self::new(label, value)
        }
    }

    /// A drop-down list; `selected` is the entry to show (the first when it
    /// does not match any of them).
    pub fn dropdown(label: &str, items: &[String], selected: &str) -> Self {
        let value = items
            .iter()
            .find(|item| item.as_str() == selected)
            .or_else(|| items.first())
            .cloned()
            .unwrap_or_default();
        Self {
            input: Input::Dropdown,
            items: items.to_vec(),
            ..Self::new(label, value)
        }
    }

    /// A list box of `lines` visible entries; the value is the selected index.
    pub fn list(label: &str, items: &[String], lines: i32) -> Self {
        Self {
            input: Input::List,
            lines: lines.max(2),
            items: items.to_vec(),
            ..Self::new(label, "")
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
    /// Own buttons `(id, label)` instead of OK/Cancel (message boxes: Yes /
    /// No / OK). The first is the default button.
    pub buttons: Vec<(i32, String)>,
    /// Width of the text area in 96-DPI pixels (0 = the usual width).
    pub width: i32,
}

impl Form {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            note: String::new(),
            fields: Vec::new(),
            checkboxes: Vec::new(),
            extras: Vec::new(),
            buttons: Vec::new(),
            width: 0,
        }
    }

    pub fn button(mut self, id: i32, label: &str) -> Self {
        self.buttons.push((id, label.to_owned()));
        self
    }

    pub fn wide(mut self, width: i32) -> Self {
        self.width = width;
        self
    }

    /// Shows the form modally and returns the id of the button pressed
    /// (`None` for Esc / the close box).
    pub fn show_choice(self, owner: HWND) -> Option<i32> {
        // SAFETY: as in `show`.
        unsafe { run(owner, self).1 }
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
        unsafe { run(owner, self).0 }
    }

    /// Like [`Self::show`] but also reports which custom button was pressed
    /// (own buttons set both the values and the chosen id).
    pub fn show_buttons(self, owner: HWND) -> (Option<Vec<String>>, Option<i32>) {
        // SAFETY: as in `show`.
        unsafe { run(owner, self) }
    }
}

struct State {
    form: Form,
    edits: Vec<HWND>,
    combos: Vec<HWND>,
    lists: Vec<HWND>,
    checks: Vec<HWND>,
    font: HFONT,
    result: Option<Vec<String>>,
    answer: Option<i32>,
    dpi: i32,
    /// Control that had the keyboard focus when the form was deactivated.
    focus: HWND,
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

/// Width of an extra button, wide enough for its label.
fn extra_width(label: &str) -> i32 {
    BUTTON_W.max(label.chars().count() as i32 * 7 + 22)
}

/// True when the extra buttons and OK/Cancel do not fit on one row: OK and
/// Cancel then get a row of their own below (they used to overlap).
fn buttons_wrap(form: &Form) -> bool {
    let extras: i32 = form
        .extras
        .iter()
        .map(|(label, _)| extra_width(label) + 6)
        .sum();
    extras > 0 && extras + BUTTON_W * 2 + 12 > LABEL_W + EDIT_W
}

/// Text lines of `note` when about `per_line` characters fit on a line.
fn note_lines_for(note: &str, per_line: i32) -> i32 {
    if note.is_empty() {
        0
    } else {
        note.lines()
            .map(|line| 1 + line.chars().count() as i32 / per_line.max(10))
            .sum::<i32>()
    }
}

fn form_note_lines(form: &Form) -> i32 {
    note_lines_for(&form.note, inner_width(form) * 70 / (LABEL_W + EDIT_W))
}

fn content_height(form: &Form) -> i32 {
    let note = form_note_lines(form);
    let inner = inner_width(form);
    let mut height = MARGIN;
    if note > 0 {
        height += note * LINE_H + 8;
    }
    for field in &form.fields {
        height += field_row_height(field, inner) + 8;
    }
    height += form.checkboxes.len() as i32 * ROW_H;
    if buttons_wrap(form) {
        height += BUTTON_H + 8;
    }
    height + BUTTON_H + MARGIN
}

/// Lines `text` wraps to inside a column `width` 96-DPI units wide. The form
/// uses the 9 pt system font, of which about 70 characters fit in 440 units.
fn wrap_lines(text: &str, width: i32) -> i32 {
    let per_line = (width * 70 / (LABEL_W + EDIT_W)).max(8);
    text.lines()
        .map(|line| {
            let count = line.chars().count() as i32;
            // Round up: a line of exactly `per_line` characters still fits.
            if count == 0 {
                1
            } else {
                (count + per_line - 1) / per_line
            }
        })
        .sum()
}

/// Room a field's label needs, in 96-DPI units: a label longer than the label
/// column wraps, and the control used to be one line tall, so at 150% (a
/// larger font in the same box) the second line — and the bottom of the first —
/// were cut off.
fn label_height(label: &str) -> i32 {
    wrap_lines(label, LABEL_W - 8) * LINE_H + 4
}

/// Height of the input control itself.
fn input_height(field: &Field) -> i32 {
    if field.lines <= 1 {
        ROW_H
    } else {
        field.lines * LINE_H + 8
    }
}

/// Total vertical space a field takes. The label and the input share a row
/// ("Trigger: [edit]") — except for a list box, whose label goes above it, so
/// the long entries get the form's whole width.
fn field_row_height(field: &Field, inner_w: i32) -> i32 {
    if field.input == Input::List {
        let label = wrap_lines(&field.label, inner_w) * LINE_H + 4;
        label + 4 + input_height(field)
    } else {
        input_height(field).max(label_height(&field.label))
    }
}

unsafe fn run(owner: HWND, form: Form) -> (Option<Vec<String>>, Option<i32>) {
    register();
    let dpi = match GetDpiForWindow(owner) {
        0 => 96,
        value => value as i32,
    };
    let width = scale(MARGIN * 2 + inner_width(&form), dpi) + scale(16, dpi);
    let height = scale(content_height(&form), dpi) + scale(40, dpi);
    let mut owner_rect: RECT = std::mem::zeroed();
    GetWindowRect(owner, &mut owner_rect);
    let x = owner_rect.left + (owner_rect.right - owner_rect.left - width) / 2;
    let y = owner_rect.top + (owner_rect.bottom - owner_rect.top - height) / 3;

    let title = win::wide(&form.title);
    let state = Box::into_raw(Box::new(State {
        form,
        edits: Vec::new(),
        combos: Vec::new(),
        lists: Vec::new(),
        checks: Vec::new(),
        font: ptr::null_mut(),
        result: None,
        answer: None,
        dpi,
        focus: ptr::null_mut(),
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
        return (None, None);
    }

    EnableWindow(owner, 0);
    let mut message: MSG = std::mem::zeroed();
    while IsWindow(dialog) != 0 {
        if GetMessageW(&mut message, ptr::null_mut(), 0, 0) <= 0 {
            break;
        }
        if crate::wnd::editword::handle(&message) {
            continue;
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
    (state.result, state.answer)
}

/// Width of the label + field area (or of the note for a message box).
fn inner_width(form: &Form) -> i32 {
    if form.width > 0 {
        form.width
    } else {
        LABEL_W + EDIT_W
    }
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
    font.lfHeight = scale_dpi(font.lfHeight, dpi);
    CreateFontIndirectW(&font)
}

/// Scales a font height taken from `SPI_GETNONCLIENTMETRICS` (which is already
/// in the system DPI) to the DPI of the window the form is opening on.
fn scale_dpi(height: i32, dpi: i32) -> i32 {
    crate::win::mul_div(height, dpi, crate::win::system_dpi())
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
    let mut values = Vec::with_capacity(state.form.fields.len() + state.checks.len());
    let mut edits = state.edits.iter();
    let mut combos = state.combos.iter();
    let mut lists = state.lists.iter();
    for field in &state.form.fields {
        values.push(match field.input {
            Input::Edit => edits.next().map_or_else(String::new, |edit| read(*edit)),
            Input::Dropdown => combos
                .next()
                .map_or_else(String::new, |combo| combo_value(*combo)),
            Input::List => lists
                .next()
                .map_or_else(String::new, |list| list_value(*list)),
        });
    }
    for check in &state.checks {
        // SAFETY: BM_GETCHECK only reads the button's state.
        let checked = SendMessageW(*check, BM_GETCHECK, 0, 0) as usize == BST_CHECKED;
        values.push(if checked { "1" } else { "0" }.to_owned());
    }
    values
}

/// Text of the entry selected in a drop-down list (empty when none is).
unsafe fn combo_value(combo: HWND) -> String {
    let selected = SendMessageW(combo, CB_GETCURSEL, 0, 0);
    if selected < 0 {
        return String::new();
    }
    let length = SendMessageW(combo, CB_GETLBTEXTLEN, selected as usize, 0).max(0) as usize;
    let mut buffer = vec![0u16; length + 1];
    let copied = SendMessageW(
        combo,
        CB_GETLBTEXT,
        selected as usize,
        buffer.as_mut_ptr() as isize,
    );
    String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
}

/// Index selected in a list box, as a string (empty when nothing is selected).
unsafe fn list_value(list: HWND) -> String {
    let selected = SendMessageW(list, LB_GETCURSEL, 0, 0);
    if selected < 0 {
        String::new()
    } else {
        selected.to_string()
    }
}

/// A drop-down list, created the same way as the format bar's combos: the
/// height asked for at creation is the height of the *dropped* list, so
/// `CBS_NOINTEGRALHEIGHT` is needed to keep it at the small size we want (a
/// plain combo always shows at least `CB_SETMINVISIBLE`, thirty entries).
unsafe fn combo_box(
    parent: HWND,
    id: i32,
    items: &[String],
    selected: &str,
    rect: (i32, i32, i32, i32),
    dpi: i32,
    font: HFONT,
) -> HWND {
    let list_height = items.len().clamp(1, 8) as i32 * (LINE_H + 4) + 6;
    let class = win::wide("ComboBox");
    let handle = CreateWindowExW(
        0,
        class.as_ptr(),
        ptr::null(),
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_VSCROLL
            | CBS_DROPDOWNLIST as u32
            | CBS_HASSTRINGS as u32
            | CBS_NOINTEGRALHEIGHT as u32,
        scale(rect.0, dpi),
        scale(rect.1, dpi),
        scale(rect.2, dpi),
        scale(list_height, dpi),
        parent,
        id as isize as _,
        GetModuleHandleW(ptr::null()),
        ptr::null(),
    );
    if handle.is_null() {
        return handle;
    }
    if !font.is_null() {
        SendMessageW(handle, WM_SETFONT, font as usize, 1);
    }
    for item in items {
        let text = win::wide(item);
        SendMessageW(handle, CB_ADDSTRING, 0, text.as_ptr() as isize);
    }
    let index = items.iter().position(|item| item == selected).unwrap_or(0);
    SendMessageW(handle, CB_SETCURSEL, index, 0);
    handle
}

/// A list box with a border, one entry per line.
unsafe fn list_box(
    parent: HWND,
    id: i32,
    items: &[String],
    rect: (i32, i32, i32, i32),
    dpi: i32,
    font: HFONT,
) -> HWND {
    let class = win::wide("ListBox");
    let handle = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        class.as_ptr(),
        ptr::null(),
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_VSCROLL
            | LBS_NOTIFY as u32
            | LBS_NOINTEGRALHEIGHT as u32,
        scale(rect.0, dpi),
        scale(rect.1, dpi),
        scale(rect.2, dpi),
        scale(rect.3, dpi),
        parent,
        id as isize as _,
        GetModuleHandleW(ptr::null()),
        ptr::null(),
    );
    if handle.is_null() {
        return handle;
    }
    if !font.is_null() {
        SendMessageW(handle, WM_SETFONT, font as usize, 1);
    }
    for item in items {
        let text = win::wide(item);
        SendMessageW(handle, LB_ADDSTRING, 0, text.as_ptr() as isize);
    }
    handle
}

unsafe fn build(hwnd: HWND, state: &mut State) {
    let dpi = state.dpi;
    state.font = make_font(dpi);
    let font = state.font;
    let mut y = MARGIN;
    let inner_w = inner_width(&state.form);
    if !state.form.note.is_empty() {
        let lines = form_note_lines(&state.form);
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
        let height = input_height(field);
        // A wide form uses its whole width for the input instead of the
        // default column, which left a band of empty space on the right.
        let input_w = (inner_w - LABEL_W).max(80);
        let id = FIRST_FIELD_ID + index as i32;
        match field.input {
            Input::List => {
                // The label goes above the list, which then spans the form.
                let label = wrap_lines(&field.label, inner_w) * LINE_H + 4;
                control(
                    hwnd,
                    "Static",
                    &field.label,
                    0,
                    -1,
                    (MARGIN, y, inner_w, label),
                    dpi,
                    font,
                );
                let list = list_box(
                    hwnd,
                    id,
                    &field.items,
                    (MARGIN, y + label + 4, inner_w, height),
                    dpi,
                    font,
                );
                state.lists.push(list);
                y += field_row_height(field, inner_w) + 8;
                continue;
            }
            Input::Dropdown => {
                let combo = combo_box(
                    hwnd,
                    id,
                    &field.items,
                    &field.value,
                    (MARGIN + LABEL_W, y, input_w, height),
                    dpi,
                    font,
                );
                state.combos.push(combo);
            }
            Input::Edit => {
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
                    id,
                    (MARGIN + LABEL_W, y, input_w, height),
                    dpi,
                    font,
                );
                state.edits.push(edit);
            }
        }
        control(
            hwnd,
            "Static",
            &field.label,
            0,
            -1,
            (MARGIN, y + 4, LABEL_W - 8, label_height(&field.label)),
            dpi,
            font,
        );
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
        let width = extra_width(label);
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
    if buttons_wrap(&state.form) {
        y += BUTTON_H + 8;
    }
    if !state.form.buttons.is_empty() {
        // Message-box buttons, right-aligned, the first one the default.
        let count = state.form.buttons.len() as i32;
        let mut x = right - count * BUTTON_W - (count - 1) * 6;
        for (index, (id, label)) in state.form.buttons.iter().enumerate() {
            let style = if index == 0 {
                BS_DEFPUSHBUTTON
            } else {
                BS_PUSHBUTTON
            };
            let button = control(
                hwnd,
                "Button",
                label,
                style as u32 | WS_TABSTOP,
                *id,
                (x, y, BUTTON_W, BUTTON_H),
                dpi,
                font,
            );
            if index == 0 && state.edits.is_empty() {
                SetFocus(button);
                state.focus = button;
            }
            x += BUTTON_W + 6;
        }
        if let Some(first) = state.edits.first() {
            SetFocus(*first);
        }
        return;
    }
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
            const EN_SETFOCUS: u16 = 0x0100;
            if win::hiword(wparam) == EN_SETFOCUS && (FIRST_FIELD_ID..FIRST_EXTRA_ID).contains(&id)
            {
                LAST_FIELD.with(|last| last.set(Some((id - FIRST_FIELD_ID) as usize)));
            }
            let custom = state(hwnd)
                .is_some_and(|state| state.form.buttons.iter().any(|(button, _)| *button == id));
            if custom {
                if let Some(state) = state(hwnd) {
                    state.answer = Some(id);
                    state.result = Some(values(state));
                }
                DestroyWindow(hwnd);
            } else if id == IDOK {
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
        // A plain window gives itself the keyboard focus when activated, so
        // typing went nowhere until a field was clicked: hand the focus to
        // the field (or button) that had it, the first field at the start.
        windows_sys::Win32::UI::WindowsAndMessaging::WM_ACTIVATE => {
            const WA_INACTIVE: usize = 0;
            if let Some(state) = state(hwnd) {
                if wparam & 0xFFFF == WA_INACTIVE {
                    let focus = windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus();
                    if !focus.is_null()
                        && windows_sys::Win32::UI::WindowsAndMessaging::IsChild(hwnd, focus) != 0
                    {
                        state.focus = focus;
                    }
                } else {
                    let target = if !state.focus.is_null() {
                        state.focus
                    } else {
                        state.edits.first().copied().unwrap_or(ptr::null_mut())
                    };
                    if !target.is_null() {
                        SetFocus(target);
                        return 0;
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
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
