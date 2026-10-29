//! Ctrl+Backspace / Ctrl+Delete for the plain edit boxes of dialogs (Find,
//! Link, rename, forms). A standard single-line Edit control types a box
//! character (DEL, 0x7F) for Ctrl+Backspace instead of deleting a word, so
//! every message loop gives those keys to [`handle`] first.

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::{EM_GETSEL, EM_REPLACESEL, EM_SETSEL};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_BACK, VK_CONTROL, VK_DELETE, VK_MENU,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowTextLengthW, GetWindowTextW, SendMessageW, MSG, WM_CHAR, WM_KEYDOWN,
};

/// True for a plain Edit control (also the edit part of a combo box).
fn is_edit(hwnd: HWND) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let mut class = [0u16; 16];
    // SAFETY: the buffer length is passed.
    let len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
    String::from_utf16_lossy(&class[..len.max(0) as usize]).eq_ignore_ascii_case("edit")
}

fn key_down(key: u16) -> bool {
    // SAFETY: plain keyboard state query.
    unsafe { GetKeyState(i32::from(key)) < 0 }
}

/// Start of the word before `caret`: trailing spaces first, then either a run
/// of letters/digits or a run of other symbols.
pub fn word_start(text: &[u16], caret: usize) -> usize {
    let chars: Vec<char> = char::decode_utf16(text[..caret].iter().copied())
        .map(|unit| unit.unwrap_or('\u{FFFD}'))
        .collect();
    let mut index = chars.len();
    while index > 0 && chars[index - 1].is_whitespace() {
        index -= 1;
    }
    if index > 0 {
        let word = chars[index - 1].is_alphanumeric() || chars[index - 1] == '_';
        while index > 0 {
            let ch = chars[index - 1];
            let same = if word {
                ch.is_alphanumeric() || ch == '_'
            } else {
                !ch.is_alphanumeric() && ch != '_' && !ch.is_whitespace()
            };
            if !same {
                break;
            }
            index -= 1;
        }
    }
    chars[..index].iter().map(|ch| ch.len_utf16()).sum()
}

/// End of the word after `caret` (Ctrl+Delete): the word, then its spaces.
pub fn word_end(text: &[u16], caret: usize) -> usize {
    let chars: Vec<char> = char::decode_utf16(text[caret..].iter().copied())
        .map(|unit| unit.unwrap_or('\u{FFFD}'))
        .collect();
    let mut index = 0;
    if let Some(first) = chars.first() {
        let word = first.is_alphanumeric() || *first == '_';
        while index < chars.len() {
            let ch = chars[index];
            let same = if word {
                ch.is_alphanumeric() || ch == '_'
            } else {
                !ch.is_alphanumeric() && ch != '_' && !ch.is_whitespace()
            };
            if !same {
                break;
            }
            index += 1;
        }
    }
    while index < chars.len() && chars[index].is_whitespace() {
        index += 1;
    }
    caret
        + chars[..index]
            .iter()
            .map(|ch| ch.len_utf16())
            .sum::<usize>()
}

/// Handles Ctrl+Backspace / Ctrl+Delete for an Edit control. Returns true
/// when the message was consumed (the caller must not translate it).
pub fn handle(message: &MSG) -> bool {
    let backspace = match (message.message, message.wParam as u16) {
        (WM_KEYDOWN, VK_BACK) => true,
        (WM_KEYDOWN, VK_DELETE) => false,
        // The DEL character a Ctrl+Backspace already produced elsewhere.
        (WM_CHAR, 0x7F) => return is_edit(message.hwnd),
        _ => return false,
    };
    if !key_down(VK_CONTROL) || key_down(VK_MENU) || !is_edit(message.hwnd) {
        return false;
    }
    let hwnd = message.hwnd;
    // SAFETY: messages to an Edit control of our own process; buffers sized
    // from the reported length.
    unsafe {
        let len = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut text = vec![0u16; len + 1];
        let copied = GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32).max(0) as usize;
        text.truncate(copied);
        let mut from = 0u32;
        let mut to = 0u32;
        SendMessageW(
            hwnd,
            EM_GETSEL,
            &mut from as *mut u32 as usize,
            &mut to as *mut u32 as isize,
        );
        let (from, to) = (from as usize, to as usize);
        if from == to {
            let (start, end) = if backspace {
                (word_start(&text, from.min(text.len())), from)
            } else {
                (from, word_end(&text, from.min(text.len())))
            };
            if start == end {
                return true;
            }
            SendMessageW(hwnd, EM_SETSEL, start, end as isize);
        }
        let empty = [0u16];
        SendMessageW(hwnd, EM_REPLACESEL, 1, empty.as_ptr() as isize);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    #[test]
    fn word_boundaries() {
        let text = wide("hello wide  world");
        assert_eq!(word_start(&text, text.len()), 12);
        assert_eq!(word_start(&text, 12), 6);
        assert_eq!(word_start(&text, 3), 0);
        let path = wide("C:\\data\\notes");
        assert_eq!(word_start(&path, path.len()), 8);
        assert_eq!(word_start(&path, 8), 7);
        assert_eq!(word_end(&text, 0), 6);
        assert_eq!(word_end(&text, 6), 12);
    }
}
