//! Keyboard behaviour of the note editor, layered over Rich Edit with a window
//! subclass:
//!
//! * Typing `- `, `* `, `1. ` or `1) ` at the start of a paragraph turns it
//!   into a list item; Ctrl+Z right after restores the typed text.
//! * In a list item Enter starts the next item, Enter on an empty item ends the
//!   list, Tab/Shift+Tab nest/unnest and Backspace at the start removes the
//!   bullet but keeps the indentation.
//! * Tab at the start of an ordinary paragraph (or on a multi-line selection)
//!   changes the paragraph start indent, so wrapped lines stay aligned.
//! * Typing `# ` … `#### ` at the start of a paragraph makes a heading, and
//!   Enter at the end of a heading continues with body text.
//! * `---` + Enter becomes a horizontal separator.
//! * Ctrl+V / Shift+Insert paste a clipboard picture (Rich Edit alone only
//!   pastes text/RTF), Ctrl+Shift+V pastes plain text.

use std::cell::{Cell, RefCell};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_BACK, VK_CONTROL, VK_INSERT, VK_MENU, VK_RETURN, VK_SHIFT,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    PostMessageW, WM_CHAR, WM_COMMAND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_PASTE,
};

use crate::wnd::{editor, format, image, link};

const SUBCLASS_ID: usize = 0x4E4F_4445; // 'NODE'

/// What the `- `/`1. ` conversion replaced, so Ctrl+Z can put it back.
struct MarkerUndo {
    editor: isize,
    start: i32,
    marker: String,
}

thread_local! {
    /// Set when a Ctrl+V keydown was fully handled here, so the WM_CHAR 0x16
    /// that `TranslateMessage` generates for it is swallowed too.
    static SWALLOW_SYN: Cell<bool> = const { Cell::new(false) };
    /// The last `- `/`1. ` marker conversion, until the user types anything.
    static MARKER_UNDO: RefCell<Option<MarkerUndo>> = const { RefCell::new(None) };
}

pub fn install(editor: HWND, frame: HWND) {
    // SAFETY: `editor` is our Rich Edit; the frame handle is the ref data.
    unsafe {
        SetWindowSubclass(editor, Some(subclass), SUBCLASS_ID, frame as usize);
    }
}

fn key_down(key: u16) -> bool {
    // SAFETY: plain key state query.
    unsafe { GetKeyState(i32::from(key)) < 0 }
}

/// Forgets the remembered marker conversion (any later edit invalidates it).
pub fn clear_marker_undo() {
    MARKER_UNDO.with(|slot| {
        slot.borrow_mut().take();
    });
}

fn remember_marker(editor: HWND, start: i32, marker: String) {
    MARKER_UNDO.with(|slot| {
        *slot.borrow_mut() = Some(MarkerUndo {
            editor: editor as isize,
            start,
            marker,
        });
    });
}

/// Ctrl+Z right after `- `: removes the list and restores the typed text.
/// Returns false when there is nothing remembered for this editor.
pub fn undo_marker_conversion(editor: HWND) -> bool {
    let entry = MARKER_UNDO.with(|slot| slot.borrow_mut().take());
    let Some(entry) = entry else {
        return false;
    };
    if entry.editor != editor as isize {
        return false;
    }
    let start = editor::paragraph_start(editor, entry.start);
    editor::set_selection(editor, start, editor::paragraph_end(editor, start));
    editor::set_para_format(
        editor,
        &editor::ParaFormat2 {
            dw_mask: editor::PFM_NUMBERING
                | editor::PFM_NUMBERINGSTYLE
                | editor::PFM_NUMBERINGSTART
                | editor::PFM_NUMBERINGTAB
                | editor::PFM_STARTINDENT
                | editor::PFM_OFFSET,
            ..editor::ParaFormat2::default()
        },
    );
    editor::set_selection(editor, start, start);
    editor::replace_selection(editor, &entry.marker, true);
    let caret = start + entry.marker.encode_utf16().count() as i32;
    editor::set_selection(editor, caret, caret);
    true
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    frame: usize,
) -> LRESULT {
    let ctrl = key_down(VK_CONTROL);
    let shift = key_down(VK_SHIFT);
    let alt = key_down(VK_MENU);
    match msg {
        WM_CHAR => {
            let ch = wparam as u32;
            if ch == 0x16 && SWALLOW_SYN.with(|flag| flag.replace(false)) {
                return 0;
            }
            // A space after `-`, `*`, `1.`, `1)` or `#{1,4}` is a Markdown
            // marker; it is converted before Rich Edit inserts the space.
            if ch == 0x20 && !ctrl && !alt {
                if space_marker(hwnd, shift) {
                    return 0;
                }
                // A full local path just typed becomes a link; the space is
                // then inserted after it.
                if link::auto_link_before_caret(hwnd) {
                    editor::replace_selection(hwnd, " ", true);
                    return 0;
                }
            }
            // Enter is handled on WM_KEYDOWN (Rich Edit inserts the paragraph
            // there and ignores the WM_CHAR CR); Tab arrives as WM_CHAR.
            if ch == 0x09 {
                clear_marker_undo();
                if !ctrl && !alt && tab_key(hwnd, shift) {
                    return 0;
                }
            } else if ch == 0x22 && !ctrl && !alt {
                // A quoted path is complete once the closing quote is in.
                let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                link::auto_link_before_caret(hwnd);
                return result;
            } else if ch >= 0x20 {
                clear_marker_undo();
            }
        }
        WM_KEYDOWN => {
            let key = wparam as u16;
            if ctrl || alt {
                if key == VK_RETURN {
                    clear_marker_undo();
                }
            } else if key == VK_RETURN && !shift {
                // Finish a typed path before the paragraph break splits it.
                link::auto_link_before_caret(hwnd);
                if newline_keep_indent(hwnd) {
                    return 0;
                }
            } else if key == VK_BACK && !shift && backspace_key(hwnd) {
                return 0;
            }
            let paste =
                (ctrl && !alt && key == u16::from(b'V')) || (shift && !ctrl && key == VK_INSERT);
            if paste {
                let plain = ctrl && shift;
                if paste_special(hwnd, frame as HWND, plain) {
                    SWALLOW_SYN.with(|flag| flag.set(ctrl));
                    return 0;
                }
            }
        }
        WM_PASTE => {
            clear_marker_undo();
            if paste_special(hwnd, frame as HWND, false) {
                return 0;
            }
        }
        WM_LBUTTONDBLCLK => {
            // Double-clicking a formula opens it for editing; a picture opens
            // the Image Size dialog.
            let result = DefSubclassProc(hwnd, msg, wparam, lparam);
            if crate::wnd::formula::selected_latex(hwnd).is_some() {
                PostMessageW(
                    frame as HWND,
                    WM_COMMAND,
                    usize::from(crate::wnd::menu::IDM_INSERT_FORMULA),
                    0,
                );
            } else if crate::wnd::image::pick_picture(hwnd).is_some() {
                PostMessageW(
                    frame as HWND,
                    WM_COMMAND,
                    usize::from(crate::wnd::menu::IDM_IMAGE_SIZE),
                    0,
                );
            }
            return result;
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// Turns a typed Markdown marker into formatting when the space is pressed.
fn space_marker(editor: HWND, _shift: bool) -> bool {
    if editor::in_table(editor) {
        return false;
    }
    let (from, to) = editor::selection(editor);
    if from != to {
        return false;
    }
    let start = editor::paragraph_start(editor, from);
    let head = editor::text_range(editor, start, from);
    if head.is_empty() {
        return false;
    }
    let (numbered, marker) = match head.as_str() {
        "-" | "*" => (false, format!("{head} ")),
        "1." | "1)" => (true, format!("{head} ")),
        _ => {
            let hashes = head.bytes().take_while(|byte| *byte == b'#').count();
            if hashes == head.len() && (1..=4).contains(&hashes) {
                // Keep the `#`s and insert the space ourselves; `heading`
                // then keeps this text as the editable marker.
                editor::replace_selection(editor, " ", true);
                clear_marker_undo();
                format::heading(editor, hashes);
                return true;
            }
            return false;
        }
    };
    editor::set_selection(editor, start, from);
    editor::replace_selection(editor, "", true);
    format::start_list(editor, numbered);
    remember_marker(editor, start, marker);
    editor::set_selection(editor, start, start);
    true
}

/// Enter: list items, headings, separators, then the plain indentation rule.
/// Returns false to let Rich Edit handle the key natively.
pub fn newline_keep_indent(editor: HWND) -> bool {
    clear_marker_undo();
    if editor::in_table(editor) {
        // Ordinary table cells keep Rich Edit's native Enter (next row); a
        // code block is a one-cell table and must get a paragraph inside it.
        if !format::in_code(editor) {
            return false;
        }
        return code_newline(editor);
    }
    // `---` on its own line becomes a horizontal rule.
    if format::separator_from_marker(editor) {
        return true;
    }
    if format::list_kind(editor).is_some() {
        if format::end_list(editor) {
            return true;
        }
        if format::new_literal_item(editor) {
            return true;
        }
        // Native list: Rich Edit starts the next item itself.
        return false;
    }
    // Enter at the end of a heading continues with body text, like Word.
    if format::heading_level_at(editor) > 0 {
        let (from, _) = editor::selection(editor);
        if from == editor::paragraph_end(editor, from) {
            editor::replace_selection(editor, "\r", true);
            format::heading(editor, 0);
            return true;
        }
    }
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let head = editor::text_range(editor, start, from);
    let indent: String = head
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect();
    if indent.is_empty() {
        return false;
    }
    format::insert_text(editor, &format!("\r{indent}"));
    true
}

/// Enter inside a code block: a paragraph inside the cell that carries the
/// line's own leading whitespace (tabs are characters there).
fn code_newline(editor: HWND) -> bool {
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let head = editor::text_range(editor, start, from);
    let indent: String = head
        .chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .collect();
    format::insert_text(editor, &format!("\r{indent}"));
    true
}

/// Backspace at the very start of a paragraph: remove a list bullet (keeping
/// the indent), turn a heading back into body text, or outdent one step.
fn backspace_key(editor: HWND) -> bool {
    let (from, to) = editor::selection(editor);
    if from != to || editor::in_table(editor) {
        return false;
    }
    let start = editor::paragraph_start(editor, from);
    if from != start {
        return false;
    }
    clear_marker_undo();
    if format::list_kind(editor).is_some() {
        return format::remove_list_marker(editor);
    }
    if format::heading_level_at(editor) > 0 {
        format::heading(editor, 0);
        return true;
    }
    if format::paragraph_indent(editor) > 0 {
        format::change_indent(editor, -format::INDENT_STEP_TWIPS);
        return true;
    }
    false
}

/// Tab / Shift+Tab. Returns false to let Rich Edit insert a plain tab.
pub fn tab_key(editor: HWND, outdent: bool) -> bool {
    let in_code_block = editor::in_table(editor) && format::in_code(editor);
    if editor::in_table(editor) && !in_code_block {
        return false;
    }
    if in_code_block {
        return code_tab_key(editor, outdent);
    }

    if let Some(numbered) = format::list_kind(editor) {
        let level = format::list_level(editor).max(1);
        if outdent {
            if level <= 1 {
                format::remove_list_marker(editor);
            } else {
                format::apply_list_level(editor, numbered, level - 1);
            }
        } else {
            format::apply_list_level(editor, numbered, (level + 1).min(3));
        }
        return true;
    }

    let (from, to) = editor::selection(editor);
    let first = editor::paragraph_start(editor, from);
    let last_pos = if to > from && editor::paragraph_start(editor, to) == to {
        to - 1
    } else {
        to
    };
    let multi_line = editor::paragraph_end(editor, first) < last_pos;
    if multi_line {
        if outdent {
            outdent_block(editor, first, last_pos);
        } else {
            format::change_indent(editor, format::INDENT_STEP_TWIPS);
        }
        return true;
    }
    let start = editor::paragraph_start(editor, from);
    let head = editor::text_range(editor, start, from);
    let at_start = from == start || head.chars().all(|ch| ch == ' ' || ch == '\t');
    if outdent {
        // Notes written before paragraph indents have leading tab characters:
        // outdent removes one of those first, so they keep working unchanged.
        outdent_line(editor, start, from);
        return true;
    }
    if at_start {
        // Paragraph indentation, not a tab character: wrapped lines stay
        // aligned with the indented text.
        format::change_indent(editor, format::INDENT_STEP_TWIPS);
        return true;
    }
    // In the middle of a line Tab still inserts a tab character.
    false
}

/// One indent unit at the start of `text` (a tab or up to four spaces).
fn leading_indent_len(text: &str) -> i32 {
    if text.starts_with('\t') {
        1
    } else {
        text.chars().take_while(|ch| *ch == ' ').count().min(4) as i32
    }
}

/// Shift+Tab on one paragraph: remove leading whitespace characters first,
/// otherwise decrease the paragraph indent by one step.
fn outdent_line(editor: HWND, start: i32, caret: i32) {
    // Paragraph indentation is the most recent level; remove it first.
    if format::paragraph_indent(editor) > 0 {
        format::change_indent(editor, -format::INDENT_STEP_TWIPS);
        return;
    }
    let end = editor::paragraph_end(editor, start);
    let text = editor::text_range(editor, start, end);
    let remove = leading_indent_len(&text);
    if remove > 0 {
        editor::set_selection(editor, start, start + remove);
        editor::replace_selection(editor, "", true);
        let caret = (caret - remove).max(start);
        editor::set_selection(editor, caret, caret);
    }
}

/// Shift+Tab on a multi-line selection: every line loses one leading indent
/// (characters when it has them, a paragraph step otherwise).
fn outdent_block(editor: HWND, first: i32, last_pos: i32) {
    let block_end = editor::paragraph_end(editor, last_pos.max(first));
    let units = editor::text_units(editor, first, block_end);
    let mut starts = vec![first];
    for (index, unit) in units.iter().enumerate() {
        if *unit == 0x000D {
            starts.push(first + index as i32 + 1);
        }
    }
    for &start in starts.iter().rev() {
        if start > last_pos {
            continue;
        }
        editor::set_selection(editor, start, start);
        if format::paragraph_indent(editor) > 0 {
            format::change_indent(editor, -format::INDENT_STEP_TWIPS);
            continue;
        }
        let end = editor::paragraph_end(editor, start);
        let text = editor::text_range(editor, start, end);
        let remove = leading_indent_len(&text);
        if remove > 0 {
            editor::set_selection(editor, start, start + remove);
            editor::replace_selection(editor, "", true);
        }
    }
    let end = editor::paragraph_end(editor, first);
    editor::set_selection(editor, first, end);
}

/// The old indent-with-characters behaviour inside a code block, where
/// paragraph indents would move the whole cell.
fn code_tab_key(editor: HWND, outdent: bool) -> bool {
    let (from, to) = editor::selection(editor);
    let first = editor::paragraph_start(editor, from);
    let last_pos = if to > from && editor::paragraph_start(editor, to) == to {
        to - 1
    } else {
        to
    };
    let multi_line = editor::paragraph_end(editor, first) < last_pos;
    if !outdent && !multi_line {
        // Rich Edit would jump to the next cell (or add a row) instead.
        format::insert_text(editor, "\t");
        return true;
    }

    let block_end = editor::paragraph_end(editor, last_pos.max(first));
    let units = editor::text_units(editor, first, block_end);
    let mut starts = vec![first];
    for (index, unit) in units.iter().enumerate() {
        if *unit == 0x000D {
            starts.push(first + index as i32 + 1);
        }
    }

    let mut shift_from = 0;
    let mut shift_total = 0;
    for &start in starts.iter().rev() {
        let delta = if outdent {
            let head = editor::text_units(editor, start, start + 4);
            let remove = match head.first() {
                Some(0x0009) => 1,
                _ => head.iter().take_while(|unit| **unit == 0x0020).count() as i32,
            };
            if remove == 0 {
                continue;
            }
            editor::set_selection(editor, start, start + remove);
            editor::replace_selection(editor, "", true);
            -remove
        } else {
            editor::set_selection(editor, start, start);
            format::insert_text(editor, "\t");
            1
        };
        shift_total += delta;
        if start == first {
            shift_from = delta;
        }
    }

    if multi_line {
        let new_end = editor::paragraph_end(editor, (block_end + shift_total).max(first));
        editor::set_selection(editor, first, new_end);
    } else {
        let caret = (from + shift_from).max(first);
        editor::set_selection(editor, caret, caret);
    }
    true
}

/// Handles the paste cases Rich Edit cannot: pictures, HTML and Markdown, and
/// Ctrl+Shift+V (plain text). Returns false for an ordinary paste.
fn paste_special(editor: HWND, frame: HWND, plain: bool) -> bool {
    if plain {
        if let Some(text) = image::clipboard_text(frame) {
            editor::replace_selection(
                editor,
                &text.replace("\r\n", "\r").replace('\n', "\r"),
                true,
            );
        }
        return true;
    }
    if !image::clipboard_is_image_only() {
        // Rich Text Format wins: Rich Edit pastes it natively, unchanged.
        if !image::clipboard_has_rtf() {
            if let Some(html) = image::clipboard_html(frame) {
                let markdown = nodi_core::html_in::cf_html_to_markdown(&html);
                if paste_markdown(editor, &markdown) {
                    return true;
                }
            }
            if let Some(text) = image::clipboard_text(frame) {
                if nodi_core::markdown_in::looks_like_markdown(&text)
                    && paste_markdown(editor, &text)
                {
                    return true;
                }
                if link::paste_path_link(editor, &text) {
                    return true;
                }
            }
        }
        return false;
    }
    match image::clipboard_image(frame) {
        Some(bytes) => {
            if let Err(message) = image::insert_bytes(editor, &bytes) {
                crate::wnd::dialogs::error(frame, &message);
            }
            true
        }
        None => false,
    }
}

/// Converts Markdown to the RTF Nodi uses and inserts it at the caret.
fn paste_markdown(editor: HWND, markdown: &str) -> bool {
    if markdown.trim().is_empty() {
        return false;
    }
    let rtf = nodi_core::markdown_in::markdown_to_rtf(markdown, format::full_width_twips(editor));
    editor::insert_rtf(editor, rtf.as_bytes())
}
