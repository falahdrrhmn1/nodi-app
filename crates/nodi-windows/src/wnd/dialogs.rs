//! Message boxes for the simple prompts and three small modal popup windows:
//! the icon picker, the Trash list and "Find Note by Title".
//!
//! The popups run their own local message loop (the frame is disabled while
//! they are up) and return their result through a boxed state that the caller
//! reclaims after the loop.

use std::ptr;

use nodi_core::NoteId;
use windows_sys::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    GetStockObject, InvalidateRect, UpdateWindow, DEFAULT_GUI_FONT, HBRUSH,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemServices::SS_OWNERDRAW;
use windows_sys::Win32::UI::Controls::{EM_SETLIMITTEXT, EM_SETSEL};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetActiveWindow, SetFocus};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetDlgItem, GetMessageW,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsDialogMessageW,
    IsWindow, LoadCursorW, MessageBoxW, RegisterClassExW, SendMessageW, SetWindowLongPtrW,
    ShowWindow, TranslateMessage, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CREATESTRUCTW, CS_HREDRAW,
    CS_VREDRAW, ES_AUTOHSCROLL, GWLP_USERDATA, IDCANCEL, IDC_ARROW, IDOK, IDYES, LBN_DBLCLK,
    LBS_NOTIFY, LB_ADDSTRING, LB_GETCURSEL, LB_RESETCONTENT, LB_SETCURSEL, MB_ICONERROR,
    MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_YESNO, MSG, SW_SHOW, WM_CLOSE, WM_COMMAND,
    WM_CREATE, WM_DRAWITEM, WM_NCCREATE, WM_NCDESTROY, WM_SETFONT, WM_USER, WNDCLASSEXW, WS_BORDER,
    WS_CAPTION, WS_CHILD, WS_EX_CONTROLPARENT, WS_EX_DLGMODALFRAME, WS_POPUP, WS_SYSMENU,
    WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::win;

const COLOR_BTNFACE_BRUSH: HBRUSH = 16 as HBRUSH; // COLOR_BTNFACE + 1

// ------------------------------------------------------------- message boxes

pub fn error(owner: HWND, text: &str) {
    message(owner, "Nodi", text, MB_OK | MB_ICONERROR);
}

pub fn info(owner: HWND, text: &str) {
    message(owner, "Nodi", text, MB_OK | MB_ICONINFORMATION);
}

pub fn about(owner: HWND) {
    let text = format!(
        "Nodi Mark III — {}\n\nNative Win32 notes app (Rust + Win32 + Rich Edit).\n\n\
         Phase 2+3: notes tree, tabs, autosave, trash and backups.",
        env!("CARGO_PKG_VERSION")
    );
    info(owner, &text);
}

/// Help > Keyboard Shortcuts (F1).
pub fn shortcuts(owner: HWND) {
    let width = crate::wnd::menu::SHORTCUTS
        .iter()
        .map(|(keys, _)| keys.chars().count())
        .max()
        .unwrap_or(10);
    let text = crate::wnd::menu::SHORTCUTS
        .iter()
        .map(|(keys, action)| format!("{keys:<width$}\t{action}"))
        .collect::<Vec<_>>()
        .join("\n");
    message(
        owner,
        "Nodi — Keyboard Shortcuts",
        &text,
        MB_OK | MB_ICONINFORMATION,
    );
}

/// Yes → true.
pub fn ask_yes_no(owner: HWND, text: &str) -> bool {
    message(owner, "Nodi", text, MB_YESNO | MB_ICONWARNING) == IDYES
}

pub fn recovered(owner: HWND, paths: &[std::path::PathBuf]) {
    let list = paths
        .iter()
        .map(|path| format!("  {}", path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!(
        "Nodi moved {} leftover file(s) into recovery/ (nothing was deleted):\n\n{list}",
        paths.len()
    );
    info(owner, &text);
}

pub fn save_failed(owner: HWND, title: &str, message: &str) {
    let text = format!(
        "\"{title}\" could not be saved:\n{message}\n\n\
         Your text is still in the editor. Nodi keeps retrying every 5 seconds."
    );
    error(owner, &text);
}

/// Yes → close anyway.
pub fn unsaved_close(owner: HWND, titles: &[String]) -> bool {
    let list = titles
        .iter()
        .map(|title| format!("  {title}"))
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!("Some notes could not be saved:\n\n{list}\n\nClose anyway?");
    message(owner, "Nodi", &text, MB_YESNO | MB_ICONWARNING) == IDYES
}

fn message(owner: HWND, caption: &str, text: &str, style: u32) -> i32 {
    let text = win::wide(text);
    let caption = win::wide(caption);
    // SAFETY: both buffers are NUL terminated and outlive the call.
    unsafe { MessageBoxW(owner, text.as_ptr(), caption.as_ptr(), style) }
}

// ------------------------------------------------------------ modal plumbing

unsafe fn dialog_state<'a, T>(hwnd: HWND) -> Option<&'a mut T> {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut T;
    if pointer.is_null() {
        None
    } else {
        Some(&mut *pointer)
    }
}

unsafe fn ensure_class(name: &str, proc: windows_sys::Win32::UI::WindowsAndMessaging::WNDPROC) {
    let class_name = win::wide(name);
    let instance = GetModuleHandleW(ptr::null());
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: proc,
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: ptr::null_mut(),
        hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
        hbrBackground: COLOR_BTNFACE_BRUSH,
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: ptr::null_mut(),
    };
    // Registration fails harmlessly when the class already exists.
    RegisterClassExW(&class);
}

unsafe fn create_dialog(
    owner: HWND,
    class: &str,
    title: &str,
    width: i32,
    height: i32,
    state: *mut core::ffi::c_void,
) -> HWND {
    let class_name = win::wide(class);
    let title_text = win::wide(title);
    let mut owner_rect: RECT = std::mem::zeroed();
    GetWindowRect(owner, &mut owner_rect);
    let x = owner_rect.left + ((owner_rect.right - owner_rect.left) - width) / 2;
    let y = owner_rect.top + ((owner_rect.bottom - owner_rect.top) - height) / 2;
    CreateWindowExW(
        WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
        class_name.as_ptr(),
        title_text.as_ptr(),
        WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
        x,
        y,
        width,
        height,
        owner,
        ptr::null_mut(),
        GetModuleHandleW(ptr::null()),
        state,
    )
}

unsafe fn run_modal(owner: HWND, dialog: HWND) {
    EnableWindow(owner, 0);
    ShowWindow(dialog, SW_SHOW);
    UpdateWindow(dialog);
    let mut message: MSG = std::mem::zeroed();
    while IsWindow(dialog) != 0 {
        let result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
        if result <= 0 {
            break;
        }
        if IsDialogMessageW(dialog, &message) == 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    EnableWindow(owner, 1);
    SetActiveWindow(owner);
}

/// Creates one child control with the dialog font.
///
/// Win32 needs all nine parameters separately, so grouping them would only add
/// noise.
#[allow(clippy::too_many_arguments)]
unsafe fn child(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    id: i32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> HWND {
    let class_name = win::wide(class);
    let text = win::wide(text);
    let handle = CreateWindowExW(
        0,
        class_name.as_ptr(),
        text.as_ptr(),
        style | WS_CHILD | WS_VISIBLE,
        x,
        y,
        width,
        height,
        parent,
        id as isize as windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
        GetModuleHandleW(ptr::null()),
        ptr::null(),
    );
    if !handle.is_null() {
        let font = GetStockObject(DEFAULT_GUI_FONT);
        SendMessageW(handle, WM_SETFONT, font as usize, 1);
    }
    handle
}

fn listbox_style() -> u32 {
    WS_VSCROLL | WS_BORDER | WS_TABSTOP | LBS_NOTIFY as u32
}

fn add_string(list: HWND, text: &str) {
    let wide = win::wide(text);
    // SAFETY: `wide` is NUL terminated; `list` is our list box.
    unsafe {
        SendMessageW(list, LB_ADDSTRING, 0, wide.as_ptr() as isize);
    }
}

fn selected_index(list: HWND) -> Option<usize> {
    // SAFETY: `list` is our list box.
    let index = unsafe { SendMessageW(list, LB_GETCURSEL, 0, 0) };
    if index < 0 {
        None
    } else {
        Some(index as usize)
    }
}

unsafe fn read_text(hwnd: HWND) -> String {
    let length = GetWindowTextLengthW(hwnd);
    let mut buffer = vec![0u16; (length + 1).max(1) as usize];
    GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

// -------------------------------------------------------------- icon picker

const IDC_ICON_LIST: i32 = 120;
const IDC_ICON_FILE: i32 = 121;
const IDC_ICON_REMOVE: i32 = 122;

/// What the user picked in Change Icon.
pub enum IconChoice {
    Builtin(&'static str),
    File(std::path::PathBuf),
    Remove,
}

struct PickIconState {
    entries: Vec<(&'static str, String)>,
    current: String,
    list: HWND,
    result: Option<IconChoice>,
}

/// Shows the built-in icon set plus "From file..." / "Remove icon".
pub fn pick_icon(owner: HWND, current: &str) -> Option<IconChoice> {
    // SAFETY: the whole dialog lifetime is managed here; every pointer passed
    // out lives in the boxed state or on this stack frame.
    unsafe {
        ensure_class("NodiPickIcon", Some(pick_icon_proc));
        let state = Box::new(PickIconState {
            entries: crate::wnd::tree::catalog(),
            current: current.to_owned(),
            list: ptr::null_mut(),
            result: None,
        });
        let state_ptr = Box::into_raw(state);
        let dialog = create_dialog(
            owner,
            "NodiPickIcon",
            "Change Icon",
            320,
            340,
            state_ptr as *mut core::ffi::c_void,
        );
        if dialog.is_null() {
            drop(Box::from_raw(state_ptr));
            return None;
        }
        run_modal(owner, dialog);
        let state = Box::from_raw(state_ptr);
        state.result
    }
}

unsafe extern "system" fn pick_icon_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = lparam as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CREATE => {
            if let Some(state) = dialog_state::<PickIconState>(hwnd) {
                child(hwnd, "Static", "Icon:", 0, -1, 12, 12, 280, 18);
                state.list = child(
                    hwnd,
                    "LISTBOX",
                    "",
                    listbox_style(),
                    IDC_ICON_LIST,
                    12,
                    34,
                    280,
                    200,
                );
                child(
                    hwnd,
                    "BUTTON",
                    "From file...",
                    BS_PUSHBUTTON as u32,
                    IDC_ICON_FILE,
                    12,
                    244,
                    100,
                    26,
                );
                child(
                    hwnd,
                    "BUTTON",
                    "Remove icon",
                    BS_PUSHBUTTON as u32,
                    IDC_ICON_REMOVE,
                    122,
                    244,
                    110,
                    26,
                );
                child(
                    hwnd,
                    "BUTTON",
                    "OK",
                    BS_DEFPUSHBUTTON as u32,
                    IDOK,
                    132,
                    284,
                    75,
                    26,
                );
                child(
                    hwnd,
                    "BUTTON",
                    "Cancel",
                    BS_PUSHBUTTON as u32,
                    IDCANCEL,
                    217,
                    284,
                    75,
                    26,
                );
                for (index, (_, label)) in state.entries.iter().enumerate() {
                    add_string(state.list, label);
                    if *state_entries_key(&state.entries, index) == state.current {
                        SendMessageW(state.list, LB_SETCURSEL, index, 0);
                    }
                }
                SetFocus(state.list);
            }
            0
        }
        WM_COMMAND => {
            let id = win::loword(wparam) as i32;
            let code = win::hiword(wparam) as u32;
            let double_click = id == IDC_ICON_LIST && code == LBN_DBLCLK;
            if id == IDOK || double_click {
                if let Some(state) = dialog_state::<PickIconState>(hwnd) {
                    if let Some(index) = selected_index(state.list) {
                        state.result = state
                            .entries
                            .get(index)
                            .map(|(key, _)| IconChoice::Builtin(key));
                    }
                }
                DestroyWindow(hwnd);
            } else if id == IDC_ICON_FILE {
                let path = open_image_file(hwnd);
                if let Some(path) = path {
                    if let Some(state) = dialog_state::<PickIconState>(hwnd) {
                        state.result = Some(IconChoice::File(path.into()));
                    }
                    DestroyWindow(hwnd);
                }
            } else if id == IDC_ICON_REMOVE {
                if let Some(state) = dialog_state::<PickIconState>(hwnd) {
                    state.result = Some(IconChoice::Remove);
                }
                DestroyWindow(hwnd);
            } else if id == IDCANCEL {
                DestroyWindow(hwnd);
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

fn state_entries_key<'a>(entries: &'a [(&'static str, String)], index: usize) -> &'a &'static str {
    &entries[index].0
}

// ------------------------------------------------------------------- trash

const IDC_TRASH_LIST: i32 = 100;
const IDC_TRASH_RESTORE: i32 = 101;
const IDC_TRASH_DELETE: i32 = 102;
const IDC_TRASH_CLOSE: i32 = 103;

/// What the user chose in the Trash window.
pub enum TrashChoice {
    Restore(NoteId),
    Purge(NoteId),
    Close,
}

struct TrashState {
    entries: Vec<(NoteId, String)>,
    list: HWND,
    choice: Option<(bool, NoteId)>,
}

pub fn trash(owner: HWND, entries: &[(NoteId, String)]) -> TrashChoice {
    // SAFETY: as in `pick_icon`.
    unsafe {
        ensure_class("NodiTrash", Some(trash_proc));
        let state = Box::new(TrashState {
            entries: entries.to_vec(),
            list: ptr::null_mut(),
            choice: None,
        });
        let state_ptr = Box::into_raw(state);
        let dialog = create_dialog(
            owner,
            "NodiTrash",
            "Trash",
            380,
            340,
            state_ptr as *mut core::ffi::c_void,
        );
        if dialog.is_null() {
            drop(Box::from_raw(state_ptr));
            return TrashChoice::Close;
        }
        run_modal(owner, dialog);
        let state = Box::from_raw(state_ptr);
        match state.choice {
            Some((purge, id)) if purge => TrashChoice::Purge(id),
            Some((_, id)) => TrashChoice::Restore(id),
            None => TrashChoice::Close,
        }
    }
}

unsafe extern "system" fn trash_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = lparam as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CREATE => {
            if let Some(state) = dialog_state::<TrashState>(hwnd) {
                child(hwnd, "Static", "Trashed notes:", 0, -1, 12, 12, 340, 18);
                state.list = child(
                    hwnd,
                    "LISTBOX",
                    "",
                    listbox_style(),
                    IDC_TRASH_LIST,
                    12,
                    34,
                    340,
                    230,
                );
                child(
                    hwnd,
                    "BUTTON",
                    "Restore",
                    BS_DEFPUSHBUTTON as u32,
                    IDC_TRASH_RESTORE,
                    12,
                    274,
                    100,
                    26,
                );
                child(
                    hwnd,
                    "BUTTON",
                    "Delete Permanently",
                    BS_PUSHBUTTON as u32,
                    IDC_TRASH_DELETE,
                    122,
                    274,
                    150,
                    26,
                );
                child(
                    hwnd,
                    "BUTTON",
                    "Close",
                    BS_PUSHBUTTON as u32,
                    IDC_TRASH_CLOSE,
                    282,
                    274,
                    75,
                    26,
                );
                for (_, title) in &state.entries {
                    add_string(state.list, title);
                }
                if !state.entries.is_empty() {
                    SendMessageW(state.list, LB_SETCURSEL, 0, 0);
                }
                SetFocus(state.list);
            }
            0
        }
        WM_COMMAND => {
            let id = win::loword(wparam) as i32;
            let code = win::hiword(wparam) as u32;
            let double_click = id == IDC_TRASH_LIST && code == LBN_DBLCLK;
            match id {
                IDOK | IDC_TRASH_RESTORE => finish_trash(hwnd, false),
                IDC_TRASH_DELETE => {
                    let choice = {
                        let state = dialog_state::<TrashState>(hwnd);
                        state.and_then(|state| selected_index(state.list))
                    };
                    if let Some(index) = choice {
                        let message = "Delete this item permanently?\n\nIts files are moved to \
                             data/recovery/ (nothing is deleted).";
                        let text = win::wide(message);
                        let caption = win::wide("Nodi");
                        let answer = MessageBoxW(
                            hwnd,
                            text.as_ptr(),
                            caption.as_ptr(),
                            MB_YESNO | MB_ICONWARNING,
                        );
                        if answer == IDYES {
                            if let Some(state) = dialog_state::<TrashState>(hwnd) {
                                if let Some((id, _)) = state.entries.get(index) {
                                    state.choice = Some((true, id.clone()));
                                }
                            }
                            DestroyWindow(hwnd);
                        }
                    }
                }
                IDCANCEL | IDC_TRASH_CLOSE => {
                    DestroyWindow(hwnd);
                }
                _ if double_click => finish_trash(hwnd, false),
                _ => {}
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

unsafe fn finish_trash(hwnd: HWND, purge: bool) {
    if let Some(state) = dialog_state::<TrashState>(hwnd) {
        if let Some(index) = selected_index(state.list) {
            if let Some((id, _)) = state.entries.get(index) {
                state.choice = Some((purge, id.clone()));
            }
        }
    }
    DestroyWindow(hwnd);
}

// -------------------------------------------------------------- find note

const IDC_FIND_EDIT: i32 = 110;
const IDC_FIND_LIST: i32 = 111;

struct FindState {
    entries: Vec<(Option<NoteId>, String)>,
    shown: Vec<usize>,
    edit: HWND,
    list: HWND,
    result: Option<Option<NoteId>>,
    label: &'static str,
    action: &'static str,
}

/// Live-filtering title search. Enter or double-click opens the note.
pub fn find_note(owner: HWND, entries: &[(NoteId, String)]) -> Option<NoteId> {
    let entries: Vec<(Option<NoteId>, String)> = entries
        .iter()
        .map(|(id, title)| (Some(id.clone()), title.clone()))
        .collect();
    pick(owner, "Go to Note", "Title contains:", &entries, "Open").flatten()
}

/// A filterable list of notes (`entries` are shown as given).
pub fn pick_note(
    owner: HWND,
    caption: &str,
    label: &'static str,
    entries: &[(NoteId, String)],
) -> Option<NoteId> {
    let entries: Vec<(Option<NoteId>, String)> = entries
        .iter()
        .map(|(id, title)| (Some(id.clone()), title.clone()))
        .collect();
    pick(owner, caption, label, &entries, "Open").flatten()
}

/// "Move to..." picker: the list holds an explicit "Top level" row
/// (`None`) followed by every possible parent. `None` means cancelled,
/// `Some(None)` the top level and `Some(Some(id))` that parent.
pub fn move_to(owner: HWND, entries: &[(Option<NoteId>, String)]) -> Option<Option<NoteId>> {
    pick(owner, "Move to", "Title contains:", entries, "Move")
}

fn pick(
    owner: HWND,
    caption: &str,
    label: &'static str,
    entries: &[(Option<NoteId>, String)],
    action: &'static str,
) -> Option<Option<NoteId>> {
    // SAFETY: as in `pick_icon`.
    unsafe {
        ensure_class("NodiFindNote", Some(find_note_proc));
        let state = Box::new(FindState {
            entries: entries.to_vec(),
            shown: Vec::new(),
            edit: ptr::null_mut(),
            list: ptr::null_mut(),
            result: None,
            label,
            action,
        });
        let state_ptr = Box::into_raw(state);
        let dialog = create_dialog(
            owner,
            "NodiFindNote",
            caption,
            420,
            400,
            state_ptr as *mut core::ffi::c_void,
        );
        if dialog.is_null() {
            drop(Box::from_raw(state_ptr));
            return None;
        }
        run_modal(owner, dialog);
        let state = Box::from_raw(state_ptr);
        state.result
    }
}

unsafe extern "system" fn find_note_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = lparam as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CREATE => {
            if let Some(state) = dialog_state::<FindState>(hwnd) {
                child(hwnd, "Static", state.label, 0, -1, 12, 12, 380, 18);
                state.edit = child(
                    hwnd,
                    "EDIT",
                    "",
                    WS_TABSTOP | ES_AUTOHSCROLL as u32 | WS_BORDER,
                    IDC_FIND_EDIT,
                    12,
                    32,
                    396,
                    24,
                );
                SendMessageW(state.edit, EM_SETLIMITTEXT, 200, 0);
                state.list = child(
                    hwnd,
                    "LISTBOX",
                    "",
                    listbox_style(),
                    IDC_FIND_LIST,
                    12,
                    64,
                    396,
                    270,
                );
                child(
                    hwnd,
                    "BUTTON",
                    state.action,
                    BS_DEFPUSHBUTTON as u32,
                    IDOK,
                    252,
                    344,
                    75,
                    26,
                );
                child(
                    hwnd,
                    "BUTTON",
                    "Cancel",
                    BS_PUSHBUTTON as u32,
                    IDCANCEL,
                    333,
                    344,
                    75,
                    26,
                );
                find_refill(state);
                SetFocus(state.edit);
                SendMessageW(state.edit, EM_SETSEL, 0, -1);
            }
            0
        }
        WM_COMMAND => {
            let id = win::loword(wparam) as i32;
            let code = win::hiword(wparam) as u32;
            if id == IDC_FIND_EDIT && code == 0x0300 {
                if let Some(state) = dialog_state::<FindState>(hwnd) {
                    find_refill(state);
                }
            } else if id == IDOK {
                finish_find(hwnd);
            } else if id == IDCANCEL {
                DestroyWindow(hwnd);
            } else if id == IDC_FIND_LIST && code == LBN_DBLCLK {
                finish_find(hwnd);
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

unsafe fn find_refill(state: &mut FindState) {
    let query = read_text(state.edit).to_lowercase();
    state.shown.clear();
    SendMessageW(state.list, LB_RESETCONTENT, 0, 0);
    for (index, (_, title)) in state.entries.iter().enumerate() {
        if title.to_lowercase().contains(&query) {
            state.shown.push(index);
            add_string(state.list, title);
        }
    }
    if !state.shown.is_empty() {
        SendMessageW(state.list, LB_SETCURSEL, 0, 0);
    }
}

unsafe fn finish_find(hwnd: HWND) {
    if let Some(state) = dialog_state::<FindState>(hwnd) {
        if let Some(position) = selected_index(state.list) {
            if let Some(index) = state.shown.get(position) {
                state.result = state.entries.get(*index).map(|(id, _)| id.clone());
            }
        }
    }
    DestroyWindow(hwnd);
}

/// Standard "Save As" dialog. `filter_label` like "Markdown (*.md)".
pub fn save_file(
    owner: HWND,
    default_name: &str,
    filter_label: &str,
    extension: &str,
) -> Option<String> {
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetSaveFileNameW, OFN_EXPLORER, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST,
        OPENFILENAMEW,
    };
    let filter: Vec<u16> = format!("{filter_label}\0*.{extension}\0All files (*.*)\0*.*\0\0")
        .encode_utf16()
        .collect();
    let mut buffer = vec![0u16; 4096];
    for (slot, unit) in buffer
        .iter_mut()
        .zip(default_name.encode_utf16().take(4000))
    {
        *slot = unit;
    }
    let default_extension = win::wide(extension);
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = buffer.as_mut_ptr();
    dialog.nMaxFile = buffer.len() as u32;
    dialog.lpstrDefExt = default_extension.as_ptr();
    dialog.Flags = OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    // SAFETY: the pointers refer to buffers that outlive the call.
    if unsafe { GetSaveFileNameW(&mut dialog) } == 0 {
        return None;
    }
    let length = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buffer[..length]))
}

/// Standard "Open" dialog for any file; returns its full path.
pub fn open_any_file(owner: HWND) -> Option<String> {
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OPENFILENAMEW,
    };
    let filter: Vec<u16> = "All files (*.*)\0*.*\0\0".encode_utf16().collect();
    let mut buffer = vec![0u16; 4096];
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = buffer.as_mut_ptr();
    dialog.nMaxFile = buffer.len() as u32;
    dialog.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_NOCHANGEDIR;
    // SAFETY: the pointers refer to buffers that outlive the call.
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return None;
    }
    let length = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buffer[..length]))
}

/// Folder picker for the Link dialog; returns the full folder path.
pub fn open_folder(owner: HWND) -> Option<String> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        SHBrowseForFolderW, SHGetPathFromIDListW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS,
        BROWSEINFOW,
    };
    let title = win::wide("Choose a folder");
    let mut browse: BROWSEINFOW = unsafe { std::mem::zeroed() };
    browse.hwndOwner = owner;
    browse.lpszTitle = title.as_ptr();
    browse.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;
    let mut buffer = vec![0u16; 4096];
    // SAFETY: the title outlives the modal call; the item list is freed below.
    unsafe {
        let item = SHBrowseForFolderW(&browse);
        if item.is_null() {
            return None;
        }
        let ok = SHGetPathFromIDListW(item, buffer.as_mut_ptr());
        CoTaskMemFree(item as *mut core::ffi::c_void);
        if ok == 0 {
            return None;
        }
    }
    let length = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
    (length > 0).then(|| String::from_utf16_lossy(&buffer[..length]))
}

/// Standard "Open" dialog restricted to icon files.
pub fn open_image_file(owner: HWND) -> Option<String> {
    use windows_sys::Win32::UI::Controls::Dialogs::{
        GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OPENFILENAMEW,
    };
    let filter: Vec<u16> = "Icons (*.ico;*.png)\0*.ico;*.png\0All files (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect();
    let mut buffer = vec![0u16; 4096];
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = buffer.as_mut_ptr();
    dialog.nMaxFile = buffer.len() as u32;
    dialog.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_NOCHANGEDIR;
    // SAFETY: the pointers refer to buffers that outlive the call.
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return None;
    }
    let length = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buffer[..length]))
}

// ----------------------------------------------------------- restore backup

const IDC_SNAPSHOT_LIST: i32 = 130;

struct SnapshotState {
    entries: Vec<(String, std::path::PathBuf)>,
    list: HWND,
    result: Option<std::path::PathBuf>,
}

/// Tools > Restore Backup: a list of snapshots (label already contains date
/// and size). Returns the chosen snapshot directory.
pub fn pick_snapshot(
    owner: HWND,
    entries: &[(String, std::path::PathBuf)],
) -> Option<std::path::PathBuf> {
    // SAFETY: as in `pick_icon`.
    unsafe {
        ensure_class("NodiSnapshots", Some(snapshot_proc));
        let state = Box::new(SnapshotState {
            entries: entries.to_vec(),
            list: ptr::null_mut(),
            result: None,
        });
        let state_ptr = Box::into_raw(state);
        let dialog = create_dialog(
            owner,
            "NodiSnapshots",
            "Restore Backup",
            460,
            340,
            state_ptr as *mut core::ffi::c_void,
        );
        if dialog.is_null() {
            drop(Box::from_raw(state_ptr));
            return None;
        }
        run_modal(owner, dialog);
        let state = Box::from_raw(state_ptr);
        state.result
    }
}

unsafe extern "system" fn snapshot_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = lparam as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CREATE => {
            if let Some(state) = dialog_state::<SnapshotState>(hwnd) {
                child(
                    hwnd,
                    "Static",
                    "Backups (newest first):",
                    0,
                    -1,
                    12,
                    12,
                    420,
                    18,
                );
                state.list = child(
                    hwnd,
                    "LISTBOX",
                    "",
                    listbox_style(),
                    IDC_SNAPSHOT_LIST,
                    12,
                    34,
                    420,
                    230,
                );
                child(
                    hwnd,
                    "BUTTON",
                    "Restore",
                    BS_DEFPUSHBUTTON as u32,
                    IDOK,
                    272,
                    274,
                    75,
                    26,
                );
                child(
                    hwnd,
                    "BUTTON",
                    "Cancel",
                    BS_PUSHBUTTON as u32,
                    IDCANCEL,
                    357,
                    274,
                    75,
                    26,
                );
                for (label, _) in &state.entries {
                    add_string(state.list, label);
                }
                if !state.entries.is_empty() {
                    SendMessageW(state.list, LB_SETCURSEL, 0, 0);
                }
                SetFocus(state.list);
            }
            0
        }
        WM_COMMAND => {
            let id = win::loword(wparam) as i32;
            let code = win::hiword(wparam) as u32;
            if id == IDOK || (id == IDC_SNAPSHOT_LIST && code == LBN_DBLCLK) {
                if let Some(state) = dialog_state::<SnapshotState>(hwnd) {
                    if let Some(index) = selected_index(state.list) {
                        state.result = state.entries.get(index).map(|(_, path)| path.clone());
                    }
                }
                DestroyWindow(hwnd);
            } else if id == IDCANCEL {
                DestroyWindow(hwnd);
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

// ------------------------------------------------------------ custom theme

const IDC_THEME_BG_BUTTON: i32 = 150;
const IDC_THEME_TEXT_BUTTON: i32 = 151;
const IDC_THEME_ACCENT_BUTTON: i32 = 152;
const IDC_THEME_BG_SWATCH: i32 = 160;
const IDC_THEME_TEXT_SWATCH: i32 = 161;
const IDC_THEME_ACCENT_SWATCH: i32 = 162;
const IDC_THEME_RESET: i32 = 170;

type PreviewFn = extern "system" fn(HWND, u32, u32, u32);

struct ThemeState {
    /// `#rrggbb` values, in the same shape as `CustomTheme`.
    values: [String; 3],
    brushes: [HBRUSH; 3],
    preview: PreviewFn,
    frame: HWND,
    result: Option<nodi_core::storage::CustomTheme>,
}

fn colorref(text: &str) -> u32 {
    match nodi_core::storage::parse_hex_color(text) {
        Some((red, green, blue)) => red as u32 | (green as u32) << 8 | (blue as u32) << 16,
        None => 0,
    }
}

fn hex_of(colour: u32) -> String {
    nodi_core::storage::format_hex_color((
        (colour & 0xFF) as u8,
        ((colour >> 8) & 0xFF) as u8,
        ((colour >> 16) & 0xFF) as u8,
    ))
}

impl ThemeState {
    fn new(current: &nodi_core::storage::CustomTheme, preview: PreviewFn, frame: HWND) -> Self {
        Self {
            values: [
                current.background.clone(),
                current.text.clone(),
                current.accent.clone(),
            ],
            brushes: [ptr::null_mut(); 3],
            preview,
            frame,
            result: None,
        }
    }

    fn current(&self) -> nodi_core::storage::CustomTheme {
        nodi_core::storage::CustomTheme {
            background: self.values[0].clone(),
            text: self.values[1].clone(),
            accent: self.values[2].clone(),
        }
    }

    fn notify(&self) {
        (self.preview)(
            self.frame,
            colorref(&self.values[0]),
            colorref(&self.values[1]),
            colorref(&self.values[2]),
        );
    }
}

/// View > Theme > Custom...: three `ChooseColor` picks with live preview.
pub fn custom_theme(
    owner: HWND,
    current: nodi_core::storage::CustomTheme,
    frame: HWND,
    preview: PreviewFn,
) -> Option<nodi_core::storage::CustomTheme> {
    // SAFETY: as in `pick_icon`.
    unsafe {
        ensure_class("NodiCustomTheme", Some(theme_proc));
        let state = Box::new(ThemeState::new(&current, preview, frame));
        let state_ptr = Box::into_raw(state);
        let dialog = create_dialog(
            owner,
            "NodiCustomTheme",
            "Custom Theme",
            360,
            220,
            state_ptr as *mut core::ffi::c_void,
        );
        if dialog.is_null() {
            drop(Box::from_raw(state_ptr));
            return None;
        }
        run_modal(owner, dialog);
        let state = Box::from_raw(state_ptr);
        for brush in state.brushes {
            if !brush.is_null() {
                // SAFETY: every brush was created by this dialog.
                windows_sys::Win32::Graphics::Gdi::DeleteObject(brush);
            }
        }
        state.result
    }
}

unsafe extern "system" fn theme_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            let create = lparam as *const CREATESTRUCTW;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CREATE => {
            if dialog_state::<ThemeState>(hwnd).is_some() {
                for (row, (label, button, swatch)) in [
                    (
                        "Editor background:",
                        IDC_THEME_BG_BUTTON,
                        IDC_THEME_BG_SWATCH,
                    ),
                    ("Editor text:", IDC_THEME_TEXT_BUTTON, IDC_THEME_TEXT_SWATCH),
                    (
                        "Selection accent:",
                        IDC_THEME_ACCENT_BUTTON,
                        IDC_THEME_ACCENT_SWATCH,
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let y = 12 + row as i32 * 42;
                    child(hwnd, "Static", label, 0, -1, 12, y + 4, 120, 18);
                    child(
                        hwnd,
                        "Static",
                        "",
                        SS_OWNERDRAW | WS_BORDER,
                        swatch,
                        140,
                        y,
                        80,
                        26,
                    );
                    child(
                        hwnd,
                        "BUTTON",
                        "Choose...",
                        BS_PUSHBUTTON as u32,
                        button,
                        230,
                        y,
                        110,
                        26,
                    );
                }
                child(
                    hwnd,
                    "BUTTON",
                    "Reset",
                    BS_PUSHBUTTON as u32,
                    IDC_THEME_RESET,
                    12,
                    150,
                    75,
                    26,
                );
                child(
                    hwnd,
                    "BUTTON",
                    "OK",
                    BS_DEFPUSHBUTTON as u32,
                    IDOK,
                    180,
                    150,
                    75,
                    26,
                );
                child(
                    hwnd,
                    "BUTTON",
                    "Cancel",
                    BS_PUSHBUTTON as u32,
                    IDCANCEL,
                    265,
                    150,
                    75,
                    26,
                );
                SendMessageW(hwnd, WM_USER + 1, 0, 0);
            }
            0
        }
        msg if msg == WM_USER + 1 => {
            // Refresh the three swatches from `values`.
            if let Some(state) = dialog_state::<ThemeState>(hwnd) {
                for index in 0..3 {
                    if !state.brushes[index].is_null() {
                        windows_sys::Win32::Graphics::Gdi::DeleteObject(state.brushes[index]);
                    }
                    // SAFETY: creates the brush the dialog owns from now on.
                    state.brushes[index] = windows_sys::Win32::Graphics::Gdi::CreateSolidBrush(
                        colorref(&state.values[index]),
                    );
                    let swatch = [
                        IDC_THEME_BG_SWATCH,
                        IDC_THEME_TEXT_SWATCH,
                        IDC_THEME_ACCENT_SWATCH,
                    ][index];
                    let control = GetDlgItem(hwnd, swatch);
                    if !control.is_null() {
                        InvalidateRect(control, ptr::null(), 1);
                    }
                }
            }
            0
        }
        WM_DRAWITEM => {
            let item = lparam as *const windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT;
            if !item.is_null() {
                let item = &*item;
                let index = match item.CtlID as i32 {
                    IDC_THEME_BG_SWATCH => Some(0),
                    IDC_THEME_TEXT_SWATCH => Some(1),
                    IDC_THEME_ACCENT_SWATCH => Some(2),
                    _ => None,
                };
                if let Some(index) = index {
                    if let Some(state) = dialog_state::<ThemeState>(hwnd) {
                        windows_sys::Win32::Graphics::Gdi::FillRect(
                            item.hDC,
                            &item.rcItem,
                            state.brushes[index],
                        );
                        return 1;
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_COMMAND => {
            let id = win::loword(wparam) as i32;
            match id {
                IDC_THEME_BG_BUTTON | IDC_THEME_TEXT_BUTTON | IDC_THEME_ACCENT_BUTTON => {
                    let index = match id {
                        IDC_THEME_BG_BUTTON => 0,
                        IDC_THEME_TEXT_BUTTON => 1,
                        _ => 2,
                    };
                    if let Some(state) = dialog_state::<ThemeState>(hwnd) {
                        let initial = colorref(&state.values[index]);
                        if let Some(chosen) = choose_color(hwnd, initial) {
                            state.values[index] = hex_of(chosen);
                            SendMessageW(hwnd, WM_USER + 1, 0, 0);
                            state.notify();
                        }
                    }
                }
                IDC_THEME_RESET => {
                    if let Some(state) = dialog_state::<ThemeState>(hwnd) {
                        let defaults = nodi_core::storage::CustomTheme::default();
                        state.values = [
                            defaults.background.clone(),
                            defaults.text.clone(),
                            defaults.accent.clone(),
                        ];
                        SendMessageW(hwnd, WM_USER + 1, 0, 0);
                        state.notify();
                    }
                }
                IDOK => {
                    if let Some(state) = dialog_state::<ThemeState>(hwnd) {
                        state.result = Some(state.current());
                    }
                    DestroyWindow(hwnd);
                }
                IDCANCEL => {
                    DestroyWindow(hwnd);
                }
                _ => {}
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

fn choose_color(owner: HWND, initial: u32) -> Option<u32> {
    use windows_sys::Win32::UI::Controls::Dialogs::{
        ChooseColorW, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW,
    };
    let mut custom = [0u32; 16];
    let mut dialog: CHOOSECOLORW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = std::mem::size_of::<CHOOSECOLORW>() as u32;
    dialog.hwndOwner = owner;
    dialog.rgbResult = initial;
    dialog.lpCustColors = custom.as_mut_ptr();
    dialog.Flags = CC_FULLOPEN | CC_RGBINIT;
    // SAFETY: every pointer refers to this stack frame and outlives the call.
    if unsafe { ChooseColorW(&mut dialog) } == 0 {
        None
    } else {
        Some(dialog.rgbResult)
    }
}

// Keep the instance handle import used (documentation for the class instance).
#[allow(dead_code)]
fn _assert_hinstance(_: HINSTANCE) {}
