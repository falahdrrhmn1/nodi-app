//! Tiny helpers around Win32 string and handle plumbing.
//!
//! Everything here is deliberately small so that the `unsafe` surface of the
//! UI crate stays contained in the `wnd` modules and `app.rs`.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{LPARAM, RECT};

/// NUL-terminated UTF-16 buffer for the `*W` API variants.
pub fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Integer resource identifier (e.g. icon `1`) in the form the API expects.
pub fn int_resource(id: u16) -> *const u16 {
    id as usize as *const u16
}

pub fn loword(value: usize) -> u16 {
    (value & 0xFFFF) as u16
}

pub fn hiword(value: usize) -> u16 {
    ((value >> 16) & 0xFFFF) as u16
}

/// Low word of a mouse `LPARAM`, sign extended as Windows does.
pub fn x_from_lparam(lparam: LPARAM) -> i32 {
    (lparam as i32) as i16 as i32
}

/// High word of a mouse `LPARAM`, sign extended as Windows does.
pub fn y_from_lparam(lparam: LPARAM) -> i32 {
    ((lparam as i32) >> 16) as i16 as i32
}

pub fn rect_width(rect: &RECT) -> i32 {
    rect.right - rect.left
}

pub fn rect_height(rect: &RECT) -> i32 {
    rect.bottom - rect.top
}

/// DPI-aware scaling without pulling in a math crate.
pub fn mul_div(value: i32, numerator: i32, denominator: i32) -> i32 {
    if denominator == 0 {
        return value;
    }
    ((value as i64 * numerator as i64) / denominator as i64) as i32
}

/// `LOGPIXELSY`: the vertical DPI of a device context.
const LOGPIXELSY: i32 = 90;

/// DPI that the system metrics (`SPI_GETNONCLIENTMETRICS`, menus, dialogs) are
/// already scaled for.
///
/// A font height that comes from those metrics is in *this* DPI, not 96: on a
/// 150% display `lfMessageFont.lfHeight` is already -18. Scaling that again
/// with `dpi / 96` gave a 27px dialog font, which is why field labels were
/// cut off and the whole chrome looked oversized. Scale from here instead.
pub fn system_dpi() -> i32 {
    // SAFETY: the screen DC is released straight away.
    unsafe {
        let dc = windows_sys::Win32::Graphics::Gdi::GetDC(std::ptr::null_mut());
        if dc.is_null() {
            return 96;
        }
        let dpi = windows_sys::Win32::Graphics::Gdi::GetDeviceCaps(dc, LOGPIXELSY);
        windows_sys::Win32::Graphics::Gdi::ReleaseDC(std::ptr::null_mut(), dc);
        dpi.max(48)
    }
}
