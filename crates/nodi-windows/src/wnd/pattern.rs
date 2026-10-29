//! View > Background: ruled lines, a grid or dots behind the note text, drawn
//! only on screen (the note files never change).
//!
//! Rich Edit paints itself into a memory bitmap (`WM_PRINTCLIENT`) on a key
//! colour one step away from the theme's background; every pixel still in
//! that key colour is then replaced from a pattern bitmap, so text, pictures,
//! tables and the selection stay exactly as Rich Edit drew them. The pattern
//! is anchored to the document, so it scrolls with the text.

use std::cell::Cell;
use std::ptr;

use nodi_core::Background;
use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateBitmap, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen,
    CreateSolidBrush, DeleteDC, DeleteObject, EndPaint, FillRect, LineTo, MaskBlt, MoveToEx,
    SelectObject, SetBkColor, HGDIOBJ, PAINTSTRUCT, PS_SOLID, SRCCOPY,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetGUIThreadInfo, HideCaret, SendMessageW, ShowCaret, GUITHREADINFO,
};

thread_local! {
    /// (pattern, pattern colour, real background colour).
    static CURRENT: Cell<(Background, u32, u32)> = const { Cell::new((Background::Plain, 0, 0)) };
}

const EM_GETSCROLLPOS: u32 = 0x0400 + 221;
const WM_PRINTCLIENT: u32 = 0x0318;
const PRF_CLIENT: isize = 0x0004;
const PRF_ERASEBKGND: isize = 0x0008;
/// `MAKEROP4(SRCCOPY, DSTCOPY)`: pattern where the mask is set, keep the rest.
const ROP_PATTERN_ON_BACKGROUND: u32 = 0xAACC_0020;

/// Sets the pattern for every editor (repaint them afterwards).
pub fn set(background: Background, pattern: u32, editor_bg: u32) {
    CURRENT.with(|current| current.set((background, pattern, editor_bg)));
}

pub fn active() -> bool {
    CURRENT.with(|current| current.get().0 != Background::Plain)
}

/// The colour the editor itself is given: the theme background, or with a
/// pattern a key colour that differs from it by one step in blue (pictures
/// with the theme's exact background colour are then left alone).
pub fn key_colour(editor_bg: u32) -> u32 {
    if !active() {
        return editor_bg;
    }
    let blue = (editor_bg >> 16) & 0xFF;
    let blue = if blue >= 0xFF { blue - 1 } else { blue + 1 };
    (editor_bg & 0x0000_FFFF) | (blue << 16)
}

/// `WM_PAINT` of an editor with a pattern. Returns false when there is no
/// pattern (Rich Edit paints normally).
///
/// # Safety
/// `editor` must be one of our Rich Edit controls, inside its `WM_PAINT`.
pub unsafe fn paint(editor: HWND, dpi: i32) -> bool {
    let (background, line, editor_bg) = CURRENT.with(Cell::get);
    if background == Background::Plain {
        return false;
    }
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let dc = BeginPaint(editor, &mut ps);
    // Rich Edit draws its insertion caret into the bitmap it prints for us.
    // Hidden while the pattern is painted, that caret never reaches the
    // screen; `clear_printed_caret` also clears the strip it left behind, so
    // moving the caret cannot leave a second one where it used to be.
    HideCaret(editor);
    let mut rect = RECT::default();
    GetClientRect(editor, &mut rect);
    let (width, height) = (rect.right.max(1), rect.bottom.max(1));

    let content = CreateCompatibleDC(dc);
    let content_bitmap = CreateCompatibleBitmap(dc, width, height);
    let old_content = SelectObject(content, content_bitmap as HGDIOBJ);
    // Rich Edit sometimes prints nothing while its scrollbar is being added
    // (the bitmap then keeps its initial black), so start from the key colour
    // the print would have used for the page.
    let key_brush = CreateSolidBrush(key_colour(editor_bg));
    FillRect(content, &rect, key_brush);
    DeleteObject(key_brush as HGDIOBJ);
    SendMessageW(
        editor,
        WM_PRINTCLIENT,
        content as usize,
        PRF_CLIENT | PRF_ERASEBKGND,
    );
    clear_printed_caret(editor, content);

    let pattern = CreateCompatibleDC(dc);
    let pattern_bitmap = CreateCompatibleBitmap(dc, width, height);
    let old_pattern = SelectObject(pattern, pattern_bitmap as HGDIOBJ);
    let brush = CreateSolidBrush(editor_bg);
    FillRect(pattern, &rect, brush);
    DeleteObject(brush as HGDIOBJ);
    let mut scroll = POINT { x: 0, y: 0 };
    SendMessageW(
        editor,
        EM_GETSCROLLPOS,
        0,
        &mut scroll as *mut POINT as isize,
    );
    draw_pattern(pattern, background, line, width, height, scroll, dpi);

    // Mask: 1 where Rich Edit left the key colour (the page), 0 elsewhere.
    let mask = CreateCompatibleDC(dc);
    let mask_bitmap = CreateBitmap(width, height, 1, 1, ptr::null());
    let old_mask = SelectObject(mask, mask_bitmap as HGDIOBJ);
    SetBkColor(content, key_colour(editor_bg));
    BitBlt(mask, 0, 0, width, height, content, 0, 0, SRCCOPY);
    MaskBlt(
        content,
        0,
        0,
        width,
        height,
        pattern,
        0,
        0,
        mask_bitmap,
        0,
        0,
        ROP_PATTERN_ON_BACKGROUND,
    );
    // `WM_PRINTCLIENT` hides and shows the caret around its own painting, so
    // it may be visible again even though it is still inside our paint. Hide
    // it a second time (matched by the second `ShowCaret` below) while the
    // finished bitmap replaces the pixels on screen.
    HideCaret(editor);
    let paint = ps.rcPaint;
    BitBlt(
        dc,
        paint.left,
        paint.top,
        paint.right - paint.left,
        paint.bottom - paint.top,
        content,
        paint.left,
        paint.top,
        SRCCOPY,
    );

    SelectObject(mask, old_mask);
    DeleteObject(mask_bitmap as HGDIOBJ);
    DeleteDC(mask);
    SelectObject(pattern, old_pattern);
    DeleteObject(pattern_bitmap as HGDIOBJ);
    DeleteDC(pattern);
    SelectObject(content, old_content);
    DeleteObject(content_bitmap as HGDIOBJ);
    DeleteDC(content);
    ShowCaret(editor);
    ShowCaret(editor);
    EndPaint(editor, &ps);
    true
}

/// Paints the key colour over the caret Rich Edit printed into `dc`. Without
/// this the bitmap would carry a copy of the caret to its old position when a
/// key moved it, showing a second caret until that area repainted.
unsafe fn clear_printed_caret(editor: HWND, dc: windows_sys::Win32::Graphics::Gdi::HDC) {
    if GetFocus() != editor {
        return;
    }
    // A selection is drawn instead of the caret, so there is nothing to clear
    // (and clearing would punch a pattern hole through the highlight).
    let (from, to) = crate::wnd::editor::selection(editor);
    if from != to {
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
    if rc.bottom <= rc.top {
        return;
    }
    // The insertion point is the left edge even for a block caret; a two-pixel
    // strip is enough for the line Rich Edit draws. The pixel before the caret
    // is left alone: it belongs to the character (or picture) before it.
    let strip = RECT {
        left: rc.left,
        top: rc.top,
        right: rc.left + 2,
        bottom: rc.bottom,
    };
    let (_, _, editor_bg) = CURRENT.with(Cell::get);
    let brush = CreateSolidBrush(key_colour(editor_bg));
    FillRect(dc, &strip, brush);
    DeleteObject(brush as HGDIOBJ);
}

unsafe fn draw_pattern(
    dc: windows_sys::Win32::Graphics::Gdi::HDC,
    background: Background,
    colour: u32,
    width: i32,
    height: i32,
    scroll: POINT,
    dpi: i32,
) {
    let step = (24 * dpi / 96).max(8);
    let first_y = -(scroll.y.rem_euclid(step));
    let first_x = -(scroll.x.rem_euclid(step));
    match background {
        Background::Plain => {}
        Background::Lines | Background::Grid => {
            let pen = CreatePen(PS_SOLID, 1, colour);
            let old = SelectObject(dc, pen as HGDIOBJ);
            let mut y = first_y + step - 1;
            while y < height {
                MoveToEx(dc, 0, y, ptr::null_mut());
                LineTo(dc, width, y);
                y += step;
            }
            if background == Background::Grid {
                let mut x = first_x + step - 1;
                while x < width {
                    MoveToEx(dc, x, 0, ptr::null_mut());
                    LineTo(dc, x, height);
                    x += step;
                }
            }
            SelectObject(dc, old);
            DeleteObject(pen as HGDIOBJ);
        }
        Background::Dots => {
            let brush = CreateSolidBrush(colour);
            let size = (2 * dpi / 96).max(2);
            let mut y = first_y + step - size;
            while y < height {
                let mut x = first_x + step - size;
                while x < width {
                    let dot = RECT {
                        left: x,
                        top: y,
                        right: x + size,
                        bottom: y + size,
                    };
                    FillRect(dc, &dot, brush);
                    x += step;
                }
                y += step;
            }
            DeleteObject(brush as HGDIOBJ);
        }
    }
}
