//! View > Cursor: the shape of the text caret. Rich Edit draws a thin line
//! itself; for the thick line, block and underline shapes the system caret is
//! created here, right after Rich Edit has handled the message that moved it,
//! so typing, selection and scrolling behave exactly as before.

use std::cell::Cell;
use std::ptr;

use nodi_core::storage::CaretStyle;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject, HBITMAP};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateCaret, GetGUIThreadInfo, SendMessageW, SetCaretPos, ShowCaret, GUITHREADINFO,
    WM_KILLFOCUS, WM_SETFOCUS,
};

use crate::wnd::{editor, format};

/// Editor and client rect (left, top, right, bottom) of a caret we made.
type Applied = (isize, (i32, i32, i32, i32));

thread_local! {
    /// Chosen caret shape (mirrors `Settings::caret_style`).
    static STYLE: Cell<CaretStyle> = const { Cell::new(CaretStyle::Line) };
    /// Editor and caret rect of the caret we made, so Rich Edit's own caret is
    /// only replaced when it moved.
    static APPLIED: Cell<Option<Applied>> = const { Cell::new(None) };
    /// `HBITMAP` of the underline caret, freed once it is replaced.
    static BITMAP: Cell<isize> = const { Cell::new(0) };
    /// Set while `refresh` runs, so the messages it sends cannot re-enter it.
    static BUSY: Cell<bool> = const { Cell::new(false) };
}

/// Remembers the shape; the editor draws it with its next message.
pub fn set_style(style: CaretStyle) {
    STYLE.with(|cell| cell.set(style));
}

/// Re-creates our caret where Rich Edit just put its own. Called after the
/// editor has processed a message, so the caret rect is up to date.
pub fn refresh(editor: HWND) {
    if STYLE.with(Cell::get) == CaretStyle::Line {
        return;
    }
    // Our own queries below re-enter the editor's subclass; only the outer
    // call may touch the caret.
    let Some(_busy) = Busy::enter() else {
        return;
    };
    // SAFETY: plain queries of our own editor; the caret calls belong to it.
    unsafe {
        if GetFocus() != editor {
            return;
        }
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..GUITHREADINFO::default()
        };
        if GetGUIThreadInfo(0, &mut info) == 0 || info.hwndCaret != editor {
            return;
        }
        let rc = info.rcCaret;
        // Still the caret we made: nothing moved.
        if APPLIED.with(Cell::get)
            == Some((editor as isize, (rc.left, rc.top, rc.right, rc.bottom)))
        {
            return;
        }
        let height = rc.bottom - rc.top;
        if height <= 0 {
            return;
        }
        let style = STYLE.with(Cell::get);
        let width = match style {
            CaretStyle::Line => return,
            CaretStyle::Thick => (3 * format::editor_dpi(editor) / 96).max(1),
            CaretStyle::Block | CaretStyle::Underline => char_width(editor, height),
        };
        if style == CaretStyle::Underline {
            create_underline(editor, width, height);
        } else {
            // A solid caret: Windows inverts the whole rectangle on screen,
            // so the block covers exactly the character after the caret.
            CreateCaret(editor, ptr::null_mut(), width, height);
            delete_bitmap();
        }
        SetCaretPos(rc.left, rc.top);
        ShowCaret(editor);
        APPLIED.with(|cell| {
            cell.set(Some((
                editor as isize,
                (rc.left, rc.top, rc.left + width, rc.top + height),
            )));
        });
    }
}

/// Back to Rich Edit's thin line: it creates its own caret again on focus.
pub fn reset(editor: HWND) {
    APPLIED.with(|cell| cell.set(None));
    // SAFETY: both messages go to our own editor; Rich Edit creates a fresh
    // caret while handling WM_SETFOCUS.
    unsafe {
        SendMessageW(editor, WM_KILLFOCUS, 0, 0);
        SendMessageW(editor, WM_SETFOCUS, 0, 0);
    }
    delete_bitmap();
}

/// Width of the character after the caret, or half the line height when there
/// is nothing to cover (end of a line, a paragraph mark, a picture).
fn char_width(editor: HWND, height: i32) -> i32 {
    let (caret, _) = editor::selection(editor);
    let x0 = editor::char_point(editor, caret).0;
    let x1 = editor::char_point(editor, caret + 1).0;
    let next = editor::text_range(editor, caret, caret + 1);
    if x1 > x0 && next != "\r" {
        x1 - x0
    } else {
        (height / 2).max(1)
    }
}

/// The underline caret: a monochrome bitmap whose bottom two pixel rows are
/// set. Rows are stored top-down and padded to a whole 16-bit word.
fn create_underline(editor: HWND, width: i32, height: i32) {
    let width = width.max(1) as usize;
    let height = height.max(1) as usize;
    let stride = width.div_ceil(16) * 2;
    let mut bits = vec![0u8; stride * height];
    for y in height.saturating_sub(2)..height {
        for x in 0..width {
            bits[y * stride + x / 8] |= 0x80 >> (x % 8);
        }
    }
    // SAFETY: `bits` is a valid buffer of the size asked for (one bit per
    // pixel, two rows per 16-bit word); the handle is our own.
    let bitmap = unsafe {
        CreateBitmap(
            width as i32,
            height as i32,
            1,
            1,
            bits.as_ptr() as *const core::ffi::c_void,
        )
    };
    if bitmap.is_null() {
        // SAFETY: a solid caret of the same size still marks the position.
        unsafe {
            CreateCaret(editor, ptr::null_mut(), width as i32, height as i32);
        }
        delete_bitmap();
        return;
    }
    // SAFETY: `bitmap` is a monochrome bitmap we own; CreateCaret XORs it with
    // the screen, which turns the two set rows into a visible underline.
    unsafe {
        CreateCaret(editor, bitmap, 0, 0);
    }
    // The new caret holds the new bitmap: the old one can go.
    delete_bitmap();
    BITMAP.with(|cell| cell.set(bitmap as isize));
}

/// Frees the underline bitmap once no caret uses it.
fn delete_bitmap() {
    let bitmap = BITMAP.with(|cell| cell.replace(0));
    if bitmap != 0 {
        // SAFETY: the handle came from `create_underline` and the caret no
        // longer references it.
        unsafe {
            DeleteObject(bitmap as HBITMAP);
        }
    }
}

/// Re-entrancy guard for the editor messages `refresh` sends itself.
struct Busy;

impl Busy {
    fn enter() -> Option<Self> {
        BUSY.with(|busy| (!busy.replace(true)).then_some(Busy))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.with(|busy| busy.set(false));
    }
}
