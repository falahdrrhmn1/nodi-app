//! Hyperlinks: web addresses, local files and links between notes
//! (`nodi://note/<id>`, so renaming a note never breaks them).
//!
//! Links are RTF `HYPERLINK` fields with a friendly display text. Ctrl+Click
//! follows a link (plain clicks keep editing the text, as in Word).

use std::ptr;

use nodi_core::links;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::NMHDR;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, SW_SHOWNORMAL};

use crate::app::App;
use crate::win;
use crate::wnd::editor::{self, CharRange};
use crate::wnd::form::{Field, Form};
use crate::wnd::{dialogs, format, status};

pub const EN_LINK: u32 = 0x070B;
const EM_AUTOURLDETECT: u32 = 0x0400 + 91;
const AURL_ENABLEURL: usize = 1;
const AURL_ENABLEEMAILADDR: usize = 2;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_MOUSEMOVE: u32 = 0x0200;

/// `ENLINK` (richedit.h, `pack(4)`): the `WPARAM` lands on offset 28.
#[repr(C, packed(4))]
pub struct EnLink {
    pub header: NMHDR,
    pub msg: u32,
    pub wparam: usize,
    pub lparam: isize,
    pub range: CharRange,
}

const _: () = assert!(std::mem::size_of::<EnLink>() == 52);

/// Typed web addresses and e-mail addresses become links automatically.
pub fn enable_auto_detect(editor: HWND) {
    // SAFETY: plain message to our Rich Edit.
    unsafe {
        SendMessageW(
            editor,
            EM_AUTOURLDETECT,
            AURL_ENABLEURL | AURL_ENABLEEMAILADDR,
            0,
        );
    }
}

/// The address behind the link covering `range`: the `HYPERLINK` field
/// instruction for a friendly link, or the text itself for a detected URL.
pub fn target_of(editor: HWND, range: CharRange) -> String {
    let (from, to) = editor::selection(editor);
    editor::set_selection(editor, range.cp_min, range.cp_max);
    let rtf = editor::selection_rtf(editor)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    editor::set_selection(editor, from, to);
    hyperlink_target(&rtf).unwrap_or_else(|| {
        editor::text_range(editor, range.cp_min, range.cp_max)
            .trim()
            .to_owned()
    })
}

/// First `HYPERLINK "..."` target in an RTF fragment (RTF escapes undone).
pub fn hyperlink_target(rtf: &str) -> Option<String> {
    let at = rtf.find("HYPERLINK")?;
    let rest = &rtf[at + "HYPERLINK".len()..];
    let open = rest.find('"')?;
    let rest = &rest[open + 1..];
    let close = rest.find('"')?;
    Some(rtf_unescape(&rest[..close]))
}

fn rtf_unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('{') => out.push('{'),
                Some('}') => out.push('}'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// `EN_LINK` from an editor. Ctrl+Click opens; hovering shows the target.
pub fn on_link(app: &mut App, link: &EnLink) -> bool {
    let editor = link.header.hwndFrom;
    let range = link.range;
    match link.msg {
        WM_LBUTTONUP => {
            // SAFETY: plain key-state query.
            let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL)) } < 0;
            if !ctrl {
                return false;
            }
            let target = target_of(editor, range);
            if links::parse_math_link(&target).is_some() {
                editor::set_selection(editor, range.cp_min, range.cp_min);
            }
            follow(app, &target);
            true
        }
        WM_MOUSEMOVE => {
            let is_formula = crate::wnd::formula::formula_at(editor, range.cp_min).is_some();
            let message = if is_formula {
                "Formula: double-click or Ctrl+Click to edit".to_owned()
            } else {
                let text = editor::text_range(editor, range.cp_min, range.cp_max);
                format!("Ctrl+Click to open: {}", text.trim())
            };
            status::set_message(app, &message);
            false
        }
        _ => false,
    }
}

/// Opens a link target: a formula (editor), another note, a file or a web page.
pub fn follow(app: &mut App, target: &str) {
    if links::parse_math_link(target).is_some() {
        crate::wnd::formula::insert_or_edit(app);
        return;
    }
    if let Some(id) = links::parse_note_link(target) {
        let usable = app.tree.get(&id).is_some_and(|node| !node.trashed);
        if usable {
            app.selected = Some(id.clone());
            app.open_note(id);
        } else {
            dialogs::error(
                app.hwnd,
                "The linked note no longer exists (or is in the Trash).",
            );
        }
        return;
    }
    let target = target.trim();
    if target.is_empty() {
        return;
    }
    let verb = win::wide("open");
    let file = win::wide(target);
    // SAFETY: both strings are NUL terminated and outlive the call.
    let result = unsafe {
        ShellExecuteW(
            app.hwnd,
            verb.as_ptr(),
            file.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if (result as isize) <= 32 {
        dialogs::error(app.hwnd, &format!("Windows could not open:\n{target}"));
    }
}

/// RTF for a friendly hyperlink showing `text` and pointing at `target`.
pub fn link_rtf(text: &str, target: &str) -> String {
    let target = target.replace('"', "%22");
    format!(
        "{{\\rtf1\\ansi\\uc1{{\\field{{\\*\\fldinst{{HYPERLINK \"{}\"}}}}{{\\fldrslt{{{}}}}}}}}}",
        format::rtf_escape(&target),
        format::rtf_escape(text)
    )
}

/// Insert > Link... (Ctrl+K): create a link, or edit the selected one.
pub fn insert_or_edit(app: &mut App) {
    let Some(editor) = app.active_editor() else {
        return;
    };
    let (from, to) = editor::selection(editor);
    let selected_rtf = editor::selection_rtf(editor)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    let existing = if from != to {
        hyperlink_target(&selected_rtf)
    } else {
        None
    };
    let text = if from != to {
        editor::text_range(editor, from, to)
            .trim_end_matches('\r')
            .to_owned()
    } else {
        String::new()
    };

    let notes = app.visible_titles();
    let form = Form::new("Link")
        .note("Address: a web page (https://…), a file path, or pick a note.")
        .field(Field::new("Text:", text))
        .field(Field::new("Address:", existing.unwrap_or_default()))
        .extra(
            "Browse File...",
            Box::new(|owner, values| {
                if let Some(path) = dialogs::open_any_file(owner) {
                    if values[0].trim().is_empty() {
                        values[0] = std::path::Path::new(&path)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.clone());
                    }
                    values[1] = path;
                }
            }),
        )
        .extra(
            "Link to Note...",
            Box::new(move |owner, values| {
                if let Some(id) = dialogs::find_note(owner, &notes) {
                    if values[0].trim().is_empty() {
                        if let Some((_, title)) = notes.iter().find(|(note, _)| *note == id) {
                            values[0] = title.clone();
                        }
                    }
                    values[1] = links::format_note_link(&id);
                }
            }),
        );
    let Some(values) = form.show(app.hwnd) else {
        return;
    };
    let target = values[1].trim().to_owned();
    let mut text = values[0].trim().to_owned();
    if target.is_empty() {
        return;
    }
    if text.is_empty() {
        text = target.clone();
    }
    editor::set_selection(editor, from, to);
    if editor::insert_rtf(editor, link_rtf(&text, &target).as_bytes()) {
        app.mark_dirty(editor);
    } else {
        dialogs::error(app.hwnd, "Rich Edit refused the link.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_hyperlink_target() {
        let rtf = r#"{\rtf1{\field{\*\fldinst{HYPERLINK "nodi://note/abc"}}{\fldrslt{Gradient}}}}"#;
        assert_eq!(hyperlink_target(rtf).as_deref(), Some("nodi://note/abc"));
        assert_eq!(
            hyperlink_target(r#"HYPERLINK "C:\\Users\\x.pdf""#).as_deref(),
            Some(r"C:\Users\x.pdf")
        );
        assert!(hyperlink_target("{\\rtf1 plain}").is_none());
    }

    #[test]
    fn link_rtf_round_trips_target() {
        let rtf = link_rtf("Docs {x}", r"C:\a b\c.txt");
        assert_eq!(hyperlink_target(&rtf).as_deref(), Some(r"C:\a b\c.txt"));
        assert!(rtf.contains("Docs \\{x\\}"));
    }
}
