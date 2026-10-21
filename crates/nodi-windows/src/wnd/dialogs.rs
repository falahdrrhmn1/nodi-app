//! Small modal dialogs. Phase 1 only needs About and a discard warning.

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDOK, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_OKCANCEL,
};

use crate::win;

pub fn about(owner: HWND) {
    let text = format!(
        "Nodi Mark III — {}\n\nNative Win32 notes app (Rust + Win32 + Rich Edit).\n\n\
         Phase 1: native shell. Saving, the note tree and the data directory \
         arrive in later phases.",
        env!("CARGO_PKG_VERSION")
    );
    let caption = "About Nodi";
    let text = win::wide(&text);
    let caption = win::wide(caption);
    // SAFETY: both buffers are NUL terminated and outlive the call.
    unsafe {
        MessageBoxW(
            owner,
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

/// True when the user accepts losing the unsaved text.
pub fn confirm_discard(owner: HWND) -> bool {
    let text = "This build does not save notes yet (saving arrives in Phase 3).\n\n\
                Close Nodi and discard the text in the open tabs?";
    let caption = "Nodi";
    let text = win::wide(text);
    let caption = win::wide(caption);
    // SAFETY: both buffers are NUL terminated and outlive the call.
    let result = unsafe {
        MessageBoxW(
            owner,
            text.as_ptr(),
            caption.as_ptr(),
            MB_OKCANCEL | MB_ICONWARNING,
        )
    };
    result == IDOK
}
