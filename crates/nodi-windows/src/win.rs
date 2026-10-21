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
