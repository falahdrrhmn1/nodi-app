//! Writing to the clipboard (reading lives next to its users in `image.rs`).

use windows_sys::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

use crate::win;

const CF_UNICODETEXT: u32 = 13;

/// Puts `text` (CF_UNICODETEXT) and `rtf` ("Rich Text Format") on the
/// clipboard in one go. Returns false when the clipboard could not be opened.
pub fn set_text_and_rtf(owner: HWND, text: &str, rtf: &str) -> bool {
    let mut wide: Vec<u8> = text
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut rtf_bytes = rtf.as_bytes().to_vec();
    rtf_bytes.push(0);
    let rtf_format = {
        let name = win::wide("Rich Text Format");
        // SAFETY: NUL-terminated name.
        unsafe { RegisterClipboardFormatW(name.as_ptr()) }
    };
    // SAFETY: the clipboard is opened and closed here; ownership of each
    // global block passes to the clipboard on a successful SetClipboardData.
    unsafe {
        if OpenClipboard(owner) == 0 {
            return false;
        }
        EmptyClipboard();
        put(CF_UNICODETEXT, &mut wide);
        put(rtf_format, &mut rtf_bytes);
        CloseClipboard();
    }
    true
}

/// Puts `text` on the clipboard as plain text (CF_UNICODETEXT).
pub fn set_text(owner: HWND, text: &str) -> bool {
    let mut wide: Vec<u8> = text
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    // SAFETY: the clipboard is opened and closed here; a successful
    // SetClipboardData hands the memory block to the clipboard.
    unsafe {
        if OpenClipboard(owner) == 0 {
            return false;
        }
        EmptyClipboard();
        put(CF_UNICODETEXT, &mut wide);
        CloseClipboard();
    }
    true
}

unsafe fn put(format: u32, bytes: &mut [u8]) {
    let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
    if memory.is_null() {
        return;
    }
    let target = GlobalLock(memory) as *mut u8;
    if target.is_null() {
        GlobalFree(memory);
        return;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len());
    GlobalUnlock(memory);
    if SetClipboardData(format, memory as HANDLE).is_null() {
        GlobalFree(memory);
    }
}
