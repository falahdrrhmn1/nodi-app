//! Keyboard behaviour of the note editor, layered over Rich Edit with a window
//! subclass:
//!
//! * Typing `- `, `* `, `1. `, `1) `, `[ ] ` or `[x] ` at the start of a
//!   paragraph turns it into a list item or checklist item, at the
//!   paragraph's own indent; Ctrl+Z right after restores the typed text.
//! * In a list item Enter starts the next item, Enter on an empty item ends the
//!   list, Tab/Shift+Tab nest/unnest (any depth) and Backspace at the start of
//!   the text removes the bullet, number or box but keeps the indentation.
//! * Clicking a checklist box, or Ctrl+Enter in its item, ticks it.
//! * Two spaces typed quickly after a word become ". " (like a phone
//!   keyboard); a pause between them keeps two spaces.
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
    GetKeyState, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_INSERT, VK_LEFT, VK_MENU, VK_NEXT,
    VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::GetMessageTime;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    PostMessageW, SendMessageW, WM_CHAR, WM_COMMAND, WM_HSCROLL, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PASTE,
    WM_SETFOCUS, WM_SIZE, WM_VSCROLL,
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
    /// View > "Ctrl+Up/Down Scrolls the View" (mirrors `Settings`).
    static CTRL_ARROW_SCROLL: Cell<bool> = const { Cell::new(true) };
    /// Preferences > "Two quick spaces type a period" (mirrors `Settings`).
    static DOUBLE_SPACE_PERIOD: Cell<bool> = const { Cell::new(true) };
    /// Message time and caret after the last plain space typed, for the
    /// double-space period.
    static LAST_SPACE: Cell<Option<(u32, i32)>> = const { Cell::new(None) };
}

/// Longest pause between the two spaces that still makes a period.
const DOUBLE_SPACE_MS: u32 = 400;

pub fn set_double_space_period(enabled: bool) {
    DOUBLE_SPACE_PERIOD.with(|flag| flag.set(enabled));
}

/// Ctrl+Up/Down scroll the view by one line without moving the caret
/// (Notepad++) when on; Rich Edit's paragraph jump when off.
pub fn set_ctrl_arrow_scroll(enabled: bool) {
    CTRL_ARROW_SCROLL.with(|flag| flag.set(enabled));
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

/// `EM_GETLINECOUNT`: how many display lines the editor holds.
const EM_GETLINECOUNT: u32 = 0x00BA;

/// True when pressing `key` can neither move the caret nor change the text,
/// so Rich Edit would only beep (or do nothing) if it got the key. Used while
/// Preferences > Sounds is off, where that beep is unwanted.
///
/// Ctrl+Left/Right/Back/Delete keep their word-wise behaviour: their boundary
/// cases are the same as here (nothing to move over or delete). Ctrl+Up/Down
/// are left alone because they scroll (`CTRL_ARROW_SCROLL`) or jump by
/// paragraph. Shift is irrelevant: a selection at the edge beeps too.
fn silent_boundary(editor: HWND, key: u16, ctrl: bool) -> bool {
    if ctrl && matches!(key, VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT) {
        return false;
    }
    let (from, to) = editor::selection(editor);
    let length = editor::text_length(editor);
    let line_of = |pos: i32| -> i32 {
        // SAFETY: `editor` is the live Rich Edit control of this subclass.
        unsafe { SendMessageW(editor, editor::EM_EXLINEFROMCHAR, 0, pos as isize) as i32 }
    };
    match key {
        VK_DOWN | VK_NEXT => {
            // SAFETY: plain query of our own control.
            let lines = unsafe { SendMessageW(editor, EM_GETLINECOUNT, 0, 0) as i32 };
            line_of(to) >= lines - 1
        }
        VK_UP | VK_PRIOR => line_of(from) == 0,
        VK_RIGHT | VK_DELETE => from == to && to >= length,
        VK_LEFT | VK_BACK => from == to && from == 0,
        _ => false,
    }
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

thread_local! {
    /// How deep the editor subclass is nested (Rich Edit sends messages to
    /// itself while handling one).
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    frame: usize,
) -> LRESULT {
    let depth = DEPTH.with(|depth| depth.replace(depth.get() + 1));
    let result = subclass_inner(hwnd, msg, wparam, lparam, id, frame);
    DEPTH.with(|cell| cell.set(depth));
    // The caret shape is re-applied once Rich Edit has finished with a user
    // action — never in the middle of it: replacing its caret while it still
    // handles a message makes it send itself more messages, without end.
    let user_action = matches!(
        msg,
        WM_KEYDOWN
            | WM_KEYUP
            | WM_CHAR
            | WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_MOUSEMOVE
            | WM_MOUSEWHEEL
            | WM_SETFOCUS
            | WM_VSCROLL
            | WM_HSCROLL
            | WM_SIZE
    );
    if depth == 0 && user_action {
        crate::wnd::caret::refresh(hwnd);
    }
    // A scrollbar appearing or disappearing changes the client width. Rich
    // Edit's printed page can come back blank during that transition, and the
    // strip it reveals would otherwise stay unpainted, so a size change always
    // repaints the whole editor instead of only the new strip.
    if msg == WM_SIZE {
        // SAFETY: invalidating our own editor.
        windows_sys::Win32::Graphics::Gdi::InvalidateRect(hwnd, std::ptr::null(), 1);
    }
    result
}

unsafe fn subclass_inner(
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
    // A locked note: Rich Edit itself refuses typing; Nodi's own editing
    // (list Enter/Backspace, markers, paste conversions, checkbox clicks)
    // must not change it either. Scrolling and selecting still work.
    if editor::is_read_only(hwnd) {
        // A locked note beeps for every refused key. With Preferences > Sounds
        // off those keys are swallowed here instead of reaching Rich Edit.
        if !crate::wnd::dialogs::sounds()
            && ((msg == WM_CHAR && wparam >= 0x20)
                || (msg == WM_KEYDOWN && matches!(wparam as u16, VK_RETURN | VK_BACK | VK_DELETE)))
        {
            return 0;
        }
        let editing_key = msg == WM_KEYDOWN
            && matches!(wparam as u16, VK_RETURN | VK_BACK | VK_INSERT)
            || (msg == WM_KEYDOWN && ctrl && wparam as u16 == u16::from(b'V'));
        if msg == WM_CHAR || msg == WM_PASTE || msg == WM_LBUTTONDOWN || editing_key {
            return DefSubclassProc(hwnd, msg, wparam, lparam);
        }
    }
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
                    LAST_SPACE.with(|last| last.set(None));
                    return 0;
                }
                if double_space_period(hwnd) {
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
            if ch != 0x20 {
                LAST_SPACE.with(|last| last.set(None));
            }
        }
        WM_KEYDOWN => {
            let key = wparam as u16;
            if ctrl
                && !alt
                && !shift
                && (key == VK_UP || key == VK_DOWN)
                && CTRL_ARROW_SCROLL.with(|flag| flag.get())
            {
                editor::scroll_lines(hwnd, if key == VK_UP { -1 } else { 1 });
                return 0;
            }
            // Backspace at the very start still removes a list marker or a
            // heading first (below); only a Backspace that does nothing is
            // swallowed, after that.
            let silent = !alt && !crate::wnd::dialogs::sounds();
            if silent && key != VK_BACK && silent_boundary(hwnd, key, ctrl) {
                return 0;
            }
            if ctrl || alt {
                if key == VK_RETURN {
                    clear_marker_undo();
                    let (from, _) = editor::selection(hwnd);
                    if ctrl && !alt && !shift {
                        // Ctrl+Enter ticks a checklist item; anywhere else
                        // outside tables it starts a new page (as in Word).
                        if !format::toggle_check_at(hwnd, from) && !editor::in_table(hwnd) {
                            format::insert_page_break(hwnd);
                        }
                        return 0;
                    }
                }
            } else if key == VK_RETURN && !shift {
                // Finish a typed path before the paragraph break splits it.
                link::auto_link_before_caret(hwnd);
                if newline_keep_indent(hwnd) {
                    return 0;
                }
                if format::list_kind(hwnd) == Some(format::ListKind::Numbered) {
                    // The new item copies the old one's paragraph format,
                    // including an explicit start number: count again.
                    let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                    format::after_new_numbered_item(hwnd);
                    return result;
                }
            } else if key == VK_BACK && !shift && backspace_key(hwnd) {
                return 0;
            }
            if silent && key == VK_BACK && silent_boundary(hwnd, key, ctrl) {
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
        windows_sys::Win32::UI::WindowsAndMessaging::WM_PAINT if crate::wnd::pattern::active() => {
            if crate::wnd::pattern::paint(hwnd, format::editor_dpi(hwnd)) {
                return 0;
            }
        }
        WM_LBUTTONDOWN if !ctrl && !shift => {
            // A click on a checklist box ticks it and leaves the caret alone.
            let x = (lparam & 0xFFFF) as i16 as i32;
            let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
            if let Some(start) =
                editor::char_at_point(hwnd, x, y).and_then(|pos| format::checkbox_at(hwnd, pos))
            {
                if editor::point_on_char(hwnd, start, x, y) {
                    format::toggle_check_at(hwnd, start);
                    return 0;
                }
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
    // Inside a deeper bullet or checklist item the typed marker follows the
    // item's own marker; it then changes that item's kind.
    let content = format::literal_marker_end(editor)
        .filter(|end| *end <= from)
        .unwrap_or(start);
    let head = editor::text_range(editor, content, from);
    if head.is_empty() {
        return false;
    }
    let (kind, ticked) = match head.as_str() {
        "-" | "*" => (format::ListKind::Bullet, false),
        "1." | "1)" => (format::ListKind::Numbered, false),
        "[]" | "[ ]" => (format::ListKind::Check, false),
        "[x]" | "[X]" => (format::ListKind::Check, true),
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
    let marker = format!("{head} ");
    let level = format::list_level(editor);
    editor::set_selection(editor, content, from);
    editor::replace_selection(editor, "", true);
    if level > 0 {
        format::apply_list_level(editor, kind, level);
    } else {
        format::start_list(editor, kind);
    }
    if ticked {
        format::toggle_check_at(editor, start);
    }
    if level == 0 {
        remember_marker(editor, start, marker);
    }
    let caret = format::literal_marker_end(editor).unwrap_or(start);
    editor::set_selection(editor, caret, caret);
    true
}

/// The second of two quickly typed spaces after a word replaces the first
/// with ". ". Returns true when it did.
fn double_space_period(editor: HWND) -> bool {
    // SAFETY: plain query of the current message's time stamp.
    let now = unsafe { GetMessageTime() } as u32;
    let (from, to) = editor::selection(editor);
    let last = LAST_SPACE.with(|last| last.replace(Some((now, from + 1))));
    if !DOUBLE_SPACE_PERIOD.with(|flag| flag.get()) || from != to || from < 2 {
        return false;
    }
    let Some((time, caret)) = last else {
        return false;
    };
    if caret != from || now.wrapping_sub(time) > DOUBLE_SPACE_MS || format::in_code(editor) {
        return false;
    }
    // Only right after a word: "word␣" becomes "word. ", never ".␣" or "␣␣".
    let before = editor::text_range(editor, from - 2, from);
    let mut chars = before.chars();
    let (Some(word), Some(' ')) = (chars.next(), chars.next()) else {
        return false;
    };
    if !word.is_alphanumeric() {
        return false;
    }
    editor::set_selection(editor, from - 1, from);
    editor::replace_selection(editor, ". ", true);
    LAST_SPACE.with(|last| last.set(None));
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
        // Right after a literal marker (deeper bullet, checkbox) the text
        // starts here: remove the marker like a native bullet or number.
        if format::literal_marker_end(editor) == Some(from) {
            clear_marker_undo();
            return format::remove_list_marker(editor);
        }
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

    if let Some(kind) = format::list_kind(editor) {
        let level = format::list_level(editor).max(1);
        if outdent {
            if level <= 1 {
                format::remove_list_marker(editor);
            } else {
                format::apply_list_level(editor, kind, level - 1);
            }
        } else {
            format::apply_list_level(editor, kind, level + 1);
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
    // Terminal output with a drawn table (pm2, docker, MySQL...) becomes a
    // real table, whatever other formats the terminal put on the clipboard.
    if !format::in_code(editor) && !editor::in_table(editor) {
        if let Some(markdown) = image::clipboard_text(frame)
            .and_then(|text| nodi_core::markdown_in::text_table_markdown(&text))
        {
            if paste_markdown(editor, frame, &markdown) {
                return true;
            }
        }
    }
    if !image::clipboard_is_image_only() {
        // Rich Text Format wins: Rich Edit pastes it natively, unchanged.
        if !image::clipboard_has_rtf() {
            if let Some(html) = image::clipboard_html(frame) {
                let markdown = nodi_core::html_in::cf_html_to_markdown(&html);
                if paste_markdown(editor, frame, &markdown) {
                    return true;
                }
            }
            if let Some(text) = image::clipboard_text(frame) {
                if nodi_core::markdown_in::looks_like_markdown(&text)
                    && paste_markdown(editor, frame, &text)
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

/// Pictures `![alt](target)` in pasted Markdown: embedded `data:` pictures,
/// local files and web addresses (downloaded through WinHTTP). Each one that
/// could be read becomes a placeholder `U+E002 n U+E003`; the rest stay
/// links. At most a dozen pictures of 15 MB each.
fn take_pictures(markdown: &str) -> (String, Vec<Vec<u8>>) {
    const MAX_PICTURES: usize = 12;
    const MAX_BYTES: usize = 15 * 1024 * 1024;
    // The paste waits for downloads on the UI thread: at most 10 s for one
    // picture and 20 s for all of them; slower ones stay links.
    let all_done = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut pictures = Vec::new();
    let mut text = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(at) = rest.find("![") {
        text.push_str(&rest[..at]);
        let candidate = &rest[at..];
        let parsed = candidate.find("](").and_then(|middle| {
            let close = candidate[middle + 2..].find(')')? + middle + 2;
            let target = candidate[middle + 2..close]
                .split_whitespace()
                .next()?
                .trim_matches(['<', '>'])
                .to_owned();
            Some((close, target))
        });
        let bytes = parsed.as_ref().and_then(|(_, target)| {
            if pictures.len() >= MAX_PICTURES {
                return None;
            }
            let bytes = if let Some(bytes) = nodi_core::links::data_uri_bytes(target) {
                Some(bytes)
            } else if target.starts_with("http://") || target.starts_with("https://") {
                let one = std::time::Instant::now() + std::time::Duration::from_secs(10);
                crate::wnd::net::get(target, MAX_BYTES, one.min(all_done))
            } else {
                // Size checked before reading; only ordinary files.
                nodi_core::links::local_path(target)
                    .filter(|path| {
                        std::fs::metadata(path)
                            .is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_BYTES as u64)
                    })
                    .and_then(|path| std::fs::read(path).ok())
            };
            bytes.filter(|bytes| nodi_core::image::sniff(bytes).is_some())
        });
        match (parsed, bytes) {
            (Some((close, _)), Some(bytes)) => {
                text.push('\u{E002}');
                text.push_str(&pictures.len().to_string());
                text.push('\u{E003}');
                pictures.push(bytes);
                rest = &candidate[close + 1..];
            }
            _ => {
                text.push_str("![");
                rest = &candidate[2..];
            }
        }
    }
    text.push_str(rest);
    (text, pictures)
}

/// Replaces the picture placeholders [`take_pictures`] left in the note with
/// the pictures themselves. Returns how many went in.
fn insert_pictures(editor: HWND, pictures: &[Vec<u8>]) -> usize {
    let mut inserted = 0;
    let mut caret = editor::selection(editor).1;
    for _ in 0..pictures.len() {
        let raw = editor::raw_text(editor);
        let Some(start) = raw.iter().position(|unit| *unit == 0xE002) else {
            break;
        };
        let Some(length) = raw[start..].iter().position(|unit| *unit == 0xE003) else {
            break;
        };
        let end = start + length;
        let index: usize = String::from_utf16_lossy(&raw[start + 1..end])
            .parse()
            .unwrap_or(usize::MAX);
        let before = editor::text_length(editor);
        editor::set_selection(editor, start as i32, end as i32 + 1);
        editor::replace_selection(editor, "", true);
        if let Some(bytes) = pictures.get(index) {
            if image::insert_bytes(editor, bytes).is_ok() {
                inserted += 1;
            }
        }
        caret += editor::text_length(editor) - before;
    }
    editor::set_selection(editor, caret, caret);
    inserted
}

/// Converts Markdown to the RTF Nodi uses and inserts it at the caret, then
/// renders the formulas and pictures it carried.
fn paste_markdown(editor: HWND, frame: HWND, markdown: &str) -> bool {
    if markdown.trim().is_empty() {
        return false;
    }
    let (markdown, pictures) = if markdown.contains("![") {
        take_pictures(markdown)
    } else {
        (markdown.to_owned(), Vec::new())
    };
    let rtf = nodi_core::markdown_in::markdown_to_rtf(&markdown, format::full_width_twips(editor));
    if !editor::insert_rtf(editor, rtf.as_bytes()) {
        return false;
    }
    let pictures = if pictures.is_empty() {
        0
    } else {
        insert_pictures(editor, &pictures)
    };
    if pictures > 0 {
        if let Some(app) = unsafe { crate::wnd::app_mut(frame) } {
            crate::wnd::status::set_message(app, &format!("Pasted with {pictures} picture(s)"));
        }
    }
    let (rendered, kept) = crate::wnd::formula::render_pasted(editor);
    if rendered > 0 || !kept.is_empty() {
        let mut message = format!("Pasted with {rendered} formula(s)");
        if !kept.is_empty() {
            message.push_str(&format!(
                "; {} kept as text (unsupported LaTeX): {}",
                kept.len(),
                kept.join(", ")
            ));
        }
        // SAFETY: `frame` is our main window, alive while its editors are.
        if let Some(app) = unsafe { crate::wnd::app_mut(frame) } {
            crate::wnd::status::set_message(app, &message);
        }
    }
    true
}
