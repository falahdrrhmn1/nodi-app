//! Character and paragraph formatting: headings, lists, alignment, inline
//! code, code blocks, highlight, colour and font.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::mem::size_of;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{
    GetDC, GetDeviceCaps, ReleaseDC, DEFAULT_CHARSET, FW_BOLD, FW_NORMAL, LOGFONTW, LOGPIXELSX,
};
use windows_sys::Win32::UI::Controls::Dialogs::{
    ChooseColorW, ChooseFontW, CC_FULLOPEN, CC_RGBINIT, CF_EFFECTS, CF_INITTOLOGFONTSTRUCT,
    CF_SCREENFONTS, CHOOSECOLORW, CHOOSEFONTW,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;

use crate::win;
use crate::wnd::editor::{
    self, CharFormat2W, ParaFormat2, CFE_BOLD, CFE_HIDDEN, CFE_ITALIC, CFE_STRIKEOUT,
    CFE_UNDERLINE, CFM_BOLD, CFM_FACE, CFM_HIDDEN, CFM_ITALIC, CFM_SIZE, CFM_STRIKEOUT,
    CFM_UNDERLINE, PFM_ALIGNMENT, PFM_NUMBERING, PFM_NUMBERINGSTART, PFM_NUMBERINGSTYLE,
    PFM_NUMBERINGTAB, PFM_OFFSET, PFM_SPACEAFTER, PFM_SPACEBEFORE, PFM_STARTINDENT, PFM_TABSTOPS,
};
use crate::wnd::image;

pub const BODY_FONT: &str = "Segoe UI";
pub const CODE_FONT: &str = "Consolas";
/// Twips (1/20 pt).
pub const BODY_SIZE: i32 = 220;
const CODE_SIZE: i32 = 200;
/// Heading sizes in twips for levels 1..=4.
pub const HEADING_SIZES: [i32; 4] = [400, 320, 280, 250];
/// Default tab stop for body text: half of Rich Edit's 720-twip (0.5") stop.
pub const TAB_STOP_TWIPS: i32 = 360;
/// One paragraph/list indentation step (the tab step).
pub const INDENT_STEP_TWIPS: i32 = 360;
/// A tab inside a code block lines up with four characters of the code font.
pub const CODE_TAB_TWIPS: i32 = 480;
/// Colour of a heading's visible `# ` marker while the caret is in it.
const HEADING_MARKER_GREY: u32 = rgb(150, 150, 150);
/// Deeper bullet levels are literal glyphs: Rich Edit has only one native
/// bullet character (`•`).
const BULLET_GLYPHS: [&str; 3] = ["•", "◦", "▪"];

const CFM_COLOR: u32 = 0x4000_0000;
const CFE_AUTOCOLOR: u32 = 0x4000_0000;
const CFM_BACKCOLOR: u32 = 0x0400_0000;
const CFE_AUTOBACKCOLOR: u32 = 0x0400_0000;

pub const PFA_LEFT: u16 = 1;
pub const PFA_RIGHT: u16 = 2;
pub const PFA_CENTER: u16 = 3;
pub const PFA_JUSTIFY: u16 = 4;

const PFN_BULLET: u16 = 1;
const PFN_ARABIC: u16 = 2;
const PFNS_PERIOD: u16 = 0x0200;

pub const fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    red as u32 | (green as u32) << 8 | (blue as u32) << 16
}

const INLINE_CODE_BACK: u32 = rgb(236, 238, 241);
const HIGHLIGHT: u32 = rgb(255, 238, 112);
pub const CODE_BLOCK_BACK: (u8, u8, u8) = (246, 248, 250);
pub const CODE_BLOCK_BORDER: (u8, u8, u8) = (208, 215, 222);

fn face(format: &mut CharFormat2W, name: &str) {
    format.sz_face_name = [0; 32];
    for (slot, unit) in format
        .sz_face_name
        .iter_mut()
        .zip(name.encode_utf16().take(31))
    {
        *slot = unit;
    }
}

pub fn face_name(format: &CharFormat2W) -> String {
    let end = format
        .sz_face_name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(32);
    String::from_utf16_lossy(&format.sz_face_name[..end])
}

/// True when the caret sits in Consolas text (inline code or a code block).
pub fn in_code(editor: HWND) -> bool {
    let format = editor::selection_format(editor);
    format.dw_mask & CFM_FACE != 0 && face_name(&format) == CODE_FONT
}

thread_local! {
    /// Re-entrancy guard: the functions below edit the document and Rich Edit
    /// answers with `EN_CHANGE` / `EN_SELCHANGE`, which must not call back in.
    static BUSY: Cell<bool> = const { Cell::new(false) };
    /// Paragraph the caret was last seen in, per editor. Used to hide the
    /// previous heading's `# ` marker and show the current one cheaply.
    static HEADING_PARAGRAPH: RefCell<HashMap<isize, i32>> = RefCell::new(HashMap::new());
}

/// Holds the [`BUSY`] flag for as long as it is alive.
struct Busy;

impl Busy {
    fn enter() -> Option<Self> {
        BUSY.with(|flag| if flag.replace(true) { None } else { Some(Self) })
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.with(|flag| flag.set(false));
    }
}

fn is_paragraph_break(unit: u16) -> bool {
    matches!(unit, 0x000D | 0x0007 | 0xFFF9 | 0xFFFB)
}

/// Start of the paragraph that contains `pos` in a raw-text snapshot.
fn paragraph_start_in(raw: &[u16], pos: i32) -> i32 {
    let end = (pos.max(0) as usize).min(raw.len());
    raw[..end]
        .iter()
        .rposition(|unit| is_paragraph_break(*unit))
        .map_or(0, |index| index as i32 + 1)
}

/// Position of the paragraph mark that ends the paragraph containing `pos`.
fn paragraph_end_in(raw: &[u16], pos: i32) -> i32 {
    let start = (pos.max(0) as usize).min(raw.len());
    raw[start..]
        .iter()
        .position(|unit| is_paragraph_break(*unit))
        .map_or(raw.len() as i32, |index| (start + index) as i32)
}

fn text_in(raw: &[u16], from: i32, to: i32) -> String {
    let from = from.clamp(0, raw.len() as i32) as usize;
    let to = to.clamp(0, raw.len() as i32) as usize;
    if to <= from {
        return String::new();
    }
    String::from_utf16_lossy(&raw[from..to])
}

/// Starts of the paragraphs touched by `[from, to]`. A selection that ends at
/// the very start of a line does not include that line.
fn selected_paragraphs(raw: &[u16], from: i32, to: i32) -> Vec<i32> {
    let first = paragraph_start_in(raw, from);
    let last = if to > from && paragraph_start_in(raw, to) == to {
        to - 1
    } else {
        to
    };
    let block_end = paragraph_end_in(raw, last.max(first));
    let mut starts = vec![first];
    let start = (first.max(0) as usize).min(raw.len());
    let end = (block_end.max(0) as usize).min(raw.len());
    for (index, unit) in raw[start..end].iter().enumerate() {
        if is_paragraph_break(*unit) {
            let next = first + index as i32 + 1;
            if next <= last {
                starts.push(next);
            }
        }
    }
    starts
}

/// `# `, `## ` … for a heading; empty for body text.
pub fn heading_marker(level: usize) -> String {
    if level == 0 {
        String::new()
    } else {
        format!("{} ", "#".repeat(level.min(4)))
    }
}

/// `(level, UTF-16 length)` of a leading heading marker (`## `).
fn heading_prefix(text: &str) -> Option<(usize, usize)> {
    let hashes = text.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=4).contains(&hashes) {
        return None;
    }
    if text.as_bytes().get(hashes) != Some(&b' ') {
        return None;
    }
    Some((hashes, hashes + 1))
}

fn heading_marker_len(text: &str) -> usize {
    heading_prefix(text).map_or(0, |(_, len)| len)
}

/// `(level, UTF-16 length)` of a literal deeper bullet (`◦` + tab).
fn literal_bullet(text: &str) -> Option<(usize, usize)> {
    for (index, glyph) in BULLET_GLYPHS.iter().enumerate() {
        if let Some(rest) = text.strip_prefix(glyph) {
            if rest.starts_with('\t') {
                return Some((index + 1, glyph.encode_utf16().count() + 1));
            }
        }
    }
    None
}

/// Heading level (1..=4) of the text at the caret, 0 for body text.
pub fn heading_level_at(editor: HWND) -> usize {
    let format = editor::selection_format(editor);
    if format.dw_mask & CFM_SIZE == 0 || format.dw_effects & CFE_BOLD == 0 {
        return 0;
    }
    HEADING_SIZES
        .iter()
        .position(|size| *size == format.y_height)
        .map_or(0, |index| index + 1)
}

/// Character and paragraph formatting for a heading body (`level` 0 = body).
fn format_heading_body(editor: HWND, start: i32, end: i32, level: usize) {
    let (size, bold) = match level {
        1..=4 => (HEADING_SIZES[level - 1], CFE_BOLD),
        _ => (BODY_SIZE, 0),
    };
    let mut format = CharFormat2W {
        dw_mask: CFM_SIZE | CFM_BOLD | CFM_FACE,
        dw_effects: bold,
        y_height: size,
        ..CharFormat2W::default()
    };
    face(&mut format, BODY_FONT);
    editor::set_selection(editor, start, end.max(start));
    editor::set_selection_format(editor, &format);
    let spacing = ParaFormat2 {
        dw_mask: PFM_SPACEBEFORE | PFM_SPACEAFTER,
        dy_space_before: if level > 0 { 180 } else { 0 },
        dy_space_after: if level > 0 { 60 } else { 0 },
        ..ParaFormat2::default()
    };
    editor::set_selection(editor, start, end.max(start));
    editor::set_para_format(editor, &spacing);
}

/// Grey (visible) or hidden marker text at `[start, start + len)`.
fn set_marker_format(editor: HWND, start: i32, len: i32, level: usize, visible: bool) {
    if len <= 0 {
        return;
    }
    let mut format = CharFormat2W {
        dw_mask: CFM_HIDDEN | CFM_COLOR | CFM_SIZE | CFM_BOLD | CFM_FACE,
        dw_effects: if visible { 0 } else { CFE_HIDDEN },
        cr_text_color: HEADING_MARKER_GREY,
        y_height: HEADING_SIZES
            .get(level.saturating_sub(1))
            .copied()
            .unwrap_or(BODY_SIZE),
        ..CharFormat2W::default()
    };
    if level > 0 {
        format.dw_effects |= CFE_BOLD;
    }
    face(&mut format, BODY_FONT);
    editor::set_selection(editor, start, start + len);
    editor::set_selection_format(editor, &format);
}

/// Shows or hides the marker of the paragraph at `start`.
fn set_marker_visibility(editor: HWND, start: i32, visible: bool) {
    let raw = editor::raw_text(editor);
    let end = paragraph_end_in(&raw, start);
    let text = text_in(&raw, start, end);
    let len = heading_marker_len(&text) as i32;
    if len == 0 {
        return;
    }
    let (from, to) = editor::selection(editor);
    let format = CharFormat2W {
        dw_mask: CFM_HIDDEN | CFM_COLOR,
        dw_effects: if visible { 0 } else { CFE_HIDDEN },
        cr_text_color: HEADING_MARKER_GREY,
        ..CharFormat2W::default()
    };
    editor::set_selection(editor, start, start + len);
    editor::set_selection_format(editor, &format);
    editor::set_selection(editor, from, to);
}

/// Where a position moves when the marker at a paragraph start is replaced.
fn shifted(pos: i32, start: i32, end: i32, old_len: i32, new_len: i32) -> i32 {
    if pos <= start {
        pos
    } else if pos >= end {
        pos + new_len - old_len
    } else if pos < start + old_len {
        start + new_len
    } else {
        pos + new_len - old_len
    }
}

/// Replaces the leading marker of a paragraph, leaving its body alone.
/// Returns the new marker length (0 when body text).
fn rewrite_marker(editor: HWND, start: i32, text: &str, level: usize) -> i32 {
    let old_len = heading_marker_len(text) as i32;
    let marker = heading_marker(level);
    let new_len = marker.len() as i32;
    if old_len == new_len && text.starts_with(&marker) {
        return old_len;
    }
    if old_len > 0 {
        editor::set_selection(editor, start, start + old_len);
        editor::replace_selection(editor, "", true);
    }
    if new_len > 0 {
        editor::set_selection(editor, start, start);
        editor::replace_selection(editor, &marker, true);
    }
    new_len
}

/// `level` 1..=4 makes a heading (with its editable `# ` marker), 0 turns the
/// paragraph back into body text.
pub fn heading(editor: HWND, level: usize) {
    let Some(_busy) = Busy::enter() else {
        return;
    };
    if editor::in_table(editor) {
        return;
    }
    let level = level.min(4);
    let (from, to) = editor::selection(editor);
    let raw = editor::raw_text(editor);
    let starts = selected_paragraphs(&raw, from, to);
    let mut new_from = from;
    let mut new_to = to;
    for &start in starts.iter().rev() {
        let end = paragraph_end_in(&raw, start);
        let text = text_in(&raw, start, end);
        let old_len = heading_marker_len(&text) as i32;
        let body_len = (end - start - old_len).max(0);
        let marker_len = rewrite_marker(editor, start, &text, level);
        let body_start = start + marker_len;
        format_heading_body(editor, body_start, body_start + body_len.max(0), level);
        if marker_len > 0 {
            let visible = from <= end && to >= start;
            set_marker_format(editor, start, marker_len, level, visible);
        }
        new_from = shifted(new_from, start, end, old_len, marker_len);
        new_to = shifted(new_to, start, end, old_len, marker_len);
    }
    editor::set_selection(editor, new_from, new_to);
}

/// Called on `EN_CHANGE`: keeps the marker of the caret's paragraph in sync
/// with the `#`s the user typed, and turns a deleted marker back into body
/// text. Only the caret paragraph is touched.
pub fn maintain_heading(editor: HWND) {
    let Some(_busy) = Busy::enter() else {
        return;
    };
    if editor::in_table(editor) {
        return;
    }
    let (from, _) = editor::selection(editor);
    let raw = editor::raw_text(editor);
    let start = paragraph_start_in(&raw, from);
    let end = paragraph_end_in(&raw, start);
    let text = text_in(&raw, start, end);
    let prefix = heading_prefix(&text);
    let old_len = heading_marker_len(&text) as i32;
    let probe = start + old_len;
    editor::set_selection(editor, probe, probe);
    let level = heading_level_at(editor);
    match prefix {
        Some((wanted, _)) => {
            if wanted != level {
                format_heading_body(editor, probe, end, wanted);
            }
            let visible = from >= start && from <= end;
            set_marker_format(editor, start, old_len, wanted, visible);
        }
        None => {
            if level > 0 && end > start {
                // The user deleted the `#`s (or the space after them).
                format_heading_body(editor, start, end, 0);
            }
        }
    }
    editor::set_selection(editor, from, from);
}

/// Called on `EN_SELCHANGE`: shows the marker of the new paragraph and hides
/// the one the caret just left. Only two paragraphs are reformatted.
pub fn update_heading_marker(editor: HWND) {
    let Some(_busy) = Busy::enter() else {
        return;
    };
    let (from, _) = editor::selection(editor);
    let raw = editor::raw_text(editor);
    let current = paragraph_start_in(&raw, from);
    let previous = HEADING_PARAGRAPH.with(|map| map.borrow_mut().insert(editor as isize, current));
    if let Some(previous) = previous {
        if previous != current {
            set_marker_visibility(editor, previous, false);
        }
    }
    if previous != Some(current) {
        set_marker_visibility(editor, current, true);
    }
}

/// After loading a note: ensures every heading paragraph carries a marker of
/// the right depth. Markers are hidden except in the caret's paragraph.
pub fn normalize_heading_markers(editor: HWND) {
    let Some(_busy) = Busy::enter() else {
        return;
    };
    let (from, to) = editor::selection(editor);
    let raw = editor::raw_text(editor);
    let mut starts = vec![0];
    for (index, unit) in raw.iter().enumerate() {
        if is_paragraph_break(*unit) && index + 1 < raw.len() {
            starts.push(index as i32 + 1);
        }
    }
    let caret = paragraph_start_in(&raw, from);
    for &start in starts.iter().rev() {
        let end = paragraph_end_in(&raw, start);
        let text = text_in(&raw, start, end);
        let old_len = heading_marker_len(&text) as i32;
        let body_len = (end - start - old_len).max(0);
        let probe = if body_len > 0 { start + old_len } else { start };
        editor::set_selection(editor, probe, probe);
        let level = heading_level_at(editor);
        if level == 0 {
            if old_len > 0 {
                editor::set_selection(editor, start, start + old_len);
                editor::replace_selection(editor, "", true);
            }
            continue;
        }
        let marker_len = rewrite_marker(editor, start, &text, level);
        let visible = start == caret;
        set_marker_format(editor, start, marker_len, level, visible);
    }
    editor::set_selection(editor, from, to);
    HEADING_PARAGRAPH.with(|map| {
        map.borrow_mut().insert(editor as isize, caret);
    });
}

/// Drops the caret paragraph remembered for a closed tab.
pub fn forget_editor(editor: HWND) {
    HEADING_PARAGRAPH.with(|map| {
        map.borrow_mut().remove(&(editor as isize));
    });
}

/// Character format of `[from, to)` without disturbing the caret for the test
/// hook: the temporary selection must not make a hidden marker visible.
pub fn format_at(editor: HWND, from: i32, to: i32) -> CharFormat2W {
    let Some(_busy) = Busy::enter() else {
        return editor::selection_format(editor);
    };
    let saved = editor::selection(editor);
    editor::set_selection(editor, from, to);
    let format = editor::selection_format(editor);
    editor::set_selection(editor, saved.0, saved.1);
    format
}

/// True when `[from, to)` is hidden text (`CFE_HIDDEN`).
pub fn hidden_at(editor: HWND, from: i32, to: i32) -> bool {
    format_at(editor, from, to).dw_effects & CFE_HIDDEN != 0
}

// ------------------------------------------------------------------- lists

/// Geometry shared by the three bullet levels.
const LIST_MAX_LEVEL: usize = 3;

/// Increase the start indent of every selected paragraph by `delta` twips.
pub fn change_indent(editor: HWND, delta: i32) {
    let Some(_busy) = Busy::enter() else {
        return;
    };
    if delta == 0 {
        return;
    }
    let (from, to) = editor::selection(editor);
    let raw = editor::raw_text(editor);
    let starts = selected_paragraphs(&raw, from, to);
    for &start in starts.iter().rev() {
        let end = paragraph_end_in(&raw, start);
        editor::set_selection(editor, start, end);
        let current = editor::para_format(editor).dx_start_indent.max(0);
        let next = (current + delta).max(0);
        if next == current {
            continue;
        }
        let format = ParaFormat2 {
            dw_mask: PFM_STARTINDENT,
            dx_start_indent: next,
            ..ParaFormat2::default()
        };
        editor::set_para_format(editor, &format);
    }
    editor::set_selection(editor, from, to);
}

/// Start indent of the caret paragraph, in twips.
pub fn paragraph_indent(editor: HWND) -> i32 {
    editor::para_format(editor).dx_start_indent.max(0)
}

/// True when the caret paragraph is a list item, and whether it is numbered.
pub fn list_kind(editor: HWND) -> Option<bool> {
    let format = editor::para_format(editor);
    if format.w_numbering != 0 {
        return Some(format.w_numbering == PFN_ARABIC);
    }
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let end = editor::paragraph_end(editor, from);
    let text = editor::text_range(editor, start, end);
    literal_bullet(&text).map(|_| false)
}

/// Level (1..=3) of the caret's list item; 0 for ordinary text.
pub fn list_level(editor: HWND) -> usize {
    let format = editor::para_format(editor);
    if format.w_numbering != 0 {
        return ((format.dx_start_indent.max(0) / INDENT_STEP_TWIPS) as usize + 1)
            .min(LIST_MAX_LEVEL);
    }
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let end = editor::paragraph_end(editor, from);
    let text = editor::text_range(editor, start, end);
    literal_bullet(&text).map_or(0, |(level, _)| level)
}

fn list_para_format(numbered: bool, level: usize) -> ParaFormat2 {
    let indent = (level.saturating_sub(1) as i32) * INDENT_STEP_TWIPS;
    // Rich Edit has one native bullet glyph, so deeper bullet levels are a
    // literal glyph + tab with a hanging indent instead.
    let native = numbered || level == 1;
    ParaFormat2 {
        dw_mask: PFM_NUMBERING
            | PFM_NUMBERINGSTYLE
            | PFM_NUMBERINGSTART
            | PFM_NUMBERINGTAB
            | PFM_STARTINDENT
            | PFM_OFFSET,
        w_numbering: if !native {
            0
        } else if numbered {
            PFN_ARABIC
        } else {
            PFN_BULLET
        },
        w_numbering_style: if native && numbered { PFNS_PERIOD } else { 0 },
        w_numbering_start: if native { 1 } else { 0 },
        w_numbering_tab: if native { INDENT_STEP_TWIPS as u16 } else { 0 },
        dx_start_indent: indent,
        dx_offset: INDENT_STEP_TWIPS,
        ..ParaFormat2::default()
    }
}

/// Applies a bulleted/numbered list at `level` to every selected paragraph.
pub fn apply_list_level(editor: HWND, numbered: bool, level: usize) {
    let Some(_busy) = Busy::enter() else {
        return;
    };
    let level = level.clamp(1, LIST_MAX_LEVEL);
    let (from, to) = editor::selection(editor);
    let raw = editor::raw_text(editor);
    let starts = selected_paragraphs(&raw, from, to);
    let mut new_from = from;
    let mut new_to = to;
    for &start in starts.iter().rev() {
        let end = paragraph_end_in(&raw, start);
        let text = text_in(&raw, start, end);
        let old_len = literal_bullet(&text).map_or(0, |(_, len)| len as i32);
        if old_len > 0 {
            editor::set_selection(editor, start, start + old_len);
            editor::replace_selection(editor, "", true);
        }
        let mut new_len = 0;
        if !numbered && level > 1 {
            let marker = format!("{}\t", BULLET_GLYPHS[level - 1]);
            editor::set_selection(editor, start, start);
            editor::replace_selection(editor, &marker, true);
            new_len = marker.len() as i32;
        }
        editor::set_selection(
            editor,
            start,
            paragraph_end_in(&raw, start) + new_len - old_len,
        );
        editor::set_para_format(editor, &list_para_format(numbered, level));
        new_from = shifted(new_from, start, end, old_len, new_len);
        new_to = shifted(new_to, start, end, old_len, new_len);
    }
    editor::set_selection(editor, new_from, new_to);
}

/// Turns the typed `- ` (already removed) into a level-1 list item.
pub fn start_list(editor: HWND, numbered: bool) {
    apply_list_level(editor, numbered, 1);
}

/// Enter on an empty item: removes the bullet/number and keeps the text at the
/// column where it was, as a plain indented paragraph. Returns false when the
/// paragraph is not an empty list item.
pub fn end_list(editor: HWND) -> bool {
    let Some(_busy) = Busy::enter() else {
        return false;
    };
    let format = editor::para_format(editor);
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let end = editor::paragraph_end(editor, from);
    let text = editor::text_range(editor, start, end);
    let literal = literal_bullet(&text);
    if format.w_numbering == 0 && literal.is_none() {
        return false;
    }
    let content = match literal {
        Some((_, len)) => &text[len..],
        None => &text[..],
    };
    if !content.trim().is_empty() {
        return false;
    }
    let mut indent = format.dx_start_indent.max(0) + INDENT_STEP_TWIPS;
    if let Some((_, len)) = literal {
        editor::set_selection(editor, start, start + len as i32);
        editor::replace_selection(editor, "", true);
        indent = format.dx_start_indent.max(0) + INDENT_STEP_TWIPS;
    }
    let para = ParaFormat2 {
        dw_mask: PFM_NUMBERING
            | PFM_NUMBERINGSTYLE
            | PFM_NUMBERINGSTART
            | PFM_NUMBERINGTAB
            | PFM_STARTINDENT
            | PFM_OFFSET,
        dx_start_indent: indent,
        ..ParaFormat2::default()
    };
    editor::set_selection(editor, start, editor::paragraph_end(editor, start));
    editor::set_para_format(editor, &para);
    editor::set_selection(editor, start, start);
    true
}

/// Enter inside a deeper (literal) bullet item: starts the next item with the
/// same glyph and indent. Returns false for native lists (Rich Edit does it).
pub fn new_literal_item(editor: HWND) -> bool {
    let Some(_busy) = Busy::enter() else {
        return false;
    };
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let end = editor::paragraph_end(editor, from);
    let text = editor::text_range(editor, start, end);
    let Some((level, _)) = literal_bullet(&text) else {
        return false;
    };
    editor::replace_selection(editor, "\r", true);
    let new_start = editor::paragraph_start(editor, from + 1);
    let marker = format!("{}\t", BULLET_GLYPHS[level - 1]);
    editor::set_selection(editor, new_start, new_start);
    editor::replace_selection(editor, &marker, true);
    let caret = new_start + marker.encode_utf16().count() as i32;
    editor::set_selection(editor, caret, caret);
    true
}

/// Backspace at the start of a list item: removes the bullet/number but keeps
/// the indentation, so the text stays where it was.
pub fn remove_list_marker(editor: HWND) -> bool {
    let Some(_busy) = Busy::enter() else {
        return false;
    };
    let format = editor::para_format(editor);
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let end = editor::paragraph_end(editor, from);
    let text = editor::text_range(editor, start, end);
    let literal = literal_bullet(&text);
    if format.w_numbering == 0 && literal.is_none() {
        return false;
    }
    let mut indent = format.dx_start_indent.max(0) + INDENT_STEP_TWIPS;
    if let Some((_, len)) = literal {
        editor::set_selection(editor, start, start + len as i32);
        editor::replace_selection(editor, "", true);
        indent = format.dx_start_indent.max(0) + INDENT_STEP_TWIPS;
    }
    let para = ParaFormat2 {
        dw_mask: PFM_NUMBERING
            | PFM_NUMBERINGSTYLE
            | PFM_NUMBERINGSTART
            | PFM_NUMBERINGTAB
            | PFM_STARTINDENT
            | PFM_OFFSET,
        dx_start_indent: indent,
        ..ParaFormat2::default()
    };
    editor::set_selection(editor, start, editor::paragraph_end(editor, start));
    editor::set_para_format(editor, &para);
    editor::set_selection(editor, start, start);
    true
}

// --------------------------------------------------------------- separators

/// A wide, thin grey bar, encoded without a LaTeX chunk so the Markdown reader
/// recognises it as a separator rather than a formula.
fn separator_png() -> Vec<u8> {
    const WIDTH: u32 = 600;
    const HEIGHT: u32 = 3;
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 2) as usize);
    for _ in 0..WIDTH * HEIGHT {
        pixels.push(0x88);
        pixels.push(0xFF);
    }
    nodi_core::math::encode_png(WIDTH, HEIGHT, &pixels, "")
}

/// Inserts the horizontal rule picture wrapped in a `nodi-hr:` hyperlink.
fn insert_separator_impl(editor: HWND) {
    let (from, to) = editor::selection(editor);
    if to > from {
        editor::set_selection(editor, from, from);
        editor::replace_selection(editor, "", true);
    }
    let width = full_width_twips(editor);
    let height = 30;
    if image::insert_sized(editor, &separator_png(), width, height, height, "separator").is_err() {
        return;
    }
    editor::set_selection(editor, from, from + 1);
    let picture = editor::selection_rtf(editor)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .and_then(|rtf| nodi_core::rtf::picture_group(&rtf).map(str::to_owned));
    let Some(picture) = picture else {
        return;
    };
    let wrapped = format!(
        "{{\\rtf1{{\\field{{\\*\\fldinst{{HYPERLINK \"{}\"}}}}{{\\fldrslt{{{}}}}}}}}}",
        nodi_core::links::format_rule_link(),
        picture
    );
    if editor::insert_rtf(editor, wrapped.as_bytes()) {
        editor::set_selection(editor, from + 1, from + 1);
    }
}

/// Insert > Separator (Ctrl+Shift+-).
pub fn insert_separator(editor: HWND) {
    let Some(_busy) = Busy::enter() else {
        return;
    };
    insert_separator_impl(editor);
}

/// Enter after a paragraph that contains only `---`: replaces it with the
/// separator and continues in the next paragraph. Returns false otherwise.
pub fn separator_from_marker(editor: HWND) -> bool {
    let Some(_busy) = Busy::enter() else {
        return false;
    };
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let end = editor::paragraph_end(editor, from);
    let text = editor::text_range(editor, start, end);
    if text.trim() != "---" || from < start {
        return false;
    }
    // Remove the marker first, then insert at a collapsed caret: replacing a
    // multi-character selection with the field makes Rich Edit move it.
    editor::set_selection(editor, start, end);
    editor::replace_selection(editor, "", true);
    editor::set_selection(editor, start, start);
    insert_separator_impl(editor);
    // The field is several characters wide, so its end must come from the
    // paragraph itself; `start + 1` would land inside the field.
    let after = editor::paragraph_end(editor, start);
    editor::set_selection(editor, after, after);
    editor::replace_selection(editor, "\r", true);
    true
}

/// Toggles a bulleted (`numbered == false`) or numbered list.
pub fn toggle_list(editor: HWND, numbered: bool) {
    if editor::in_table(editor) {
        return;
    }
    if editor::para_format(editor).w_numbering != 0 {
        remove_lists(editor);
    } else {
        apply_list_level(editor, numbered, 1);
    }
}

/// Turns lists off for every selected paragraph (Format > Normal / toolbar).
fn remove_lists(editor: HWND) {
    let Some(_busy) = Busy::enter() else {
        return;
    };
    let (from, _) = editor::selection(editor);
    let raw = editor::raw_text(editor);
    let starts = selected_paragraphs(&raw, from, from);
    let start = starts[0];
    let end = paragraph_end_in(&raw, start);
    let text = text_in(&raw, start, end);
    let literal = literal_bullet(&text).map_or(0, |(_, len)| len as i32);
    if literal > 0 {
        editor::set_selection(editor, start, start + literal);
        editor::replace_selection(editor, "", true);
    }
    editor::set_selection(editor, start, editor::paragraph_end(editor, start));
    editor::set_para_format(
        editor,
        &ParaFormat2 {
            dw_mask: PFM_NUMBERING
                | PFM_NUMBERINGSTYLE
                | PFM_NUMBERINGSTART
                | PFM_NUMBERINGTAB
                | PFM_STARTINDENT
                | PFM_OFFSET,
            ..ParaFormat2::default()
        },
    );
    editor::set_selection(editor, from, from);
}

pub fn align(editor: HWND, alignment: u16) {
    let format = ParaFormat2 {
        dw_mask: PFM_ALIGNMENT,
        w_alignment: alignment,
        ..ParaFormat2::default()
    };
    editor::set_para_format(editor, &format);
}

/// Inline code: Consolas on a light grey background. Toggles off when the
/// selection already is code.
pub fn toggle_inline_code(editor: HWND) {
    let to_code = !in_code(editor);
    let mut format = CharFormat2W {
        dw_mask: CFM_FACE | CFM_SIZE | CFM_BACKCOLOR,
        ..CharFormat2W::default()
    };
    if to_code {
        face(&mut format, CODE_FONT);
        format.y_height = CODE_SIZE;
        format.cr_back_color = INLINE_CODE_BACK;
    } else {
        face(&mut format, BODY_FONT);
        format.y_height = BODY_SIZE;
        format.dw_effects = CFE_AUTOBACKCOLOR;
    }
    editor::set_selection_format(editor, &format);
}

/// Yellow highlighter; toggles off when already highlighted.
pub fn toggle_highlight(editor: HWND) {
    let current = editor::selection_format(editor);
    let highlighted = current.dw_mask & CFM_BACKCOLOR != 0
        && current.dw_effects & CFE_AUTOBACKCOLOR == 0
        && current.cr_back_color == HIGHLIGHT;
    let format = CharFormat2W {
        dw_mask: CFM_BACKCOLOR,
        dw_effects: if highlighted { CFE_AUTOBACKCOLOR } else { 0 },
        cr_back_color: HIGHLIGHT,
        ..CharFormat2W::default()
    };
    editor::set_selection_format(editor, &format);
}

/// Applies a text colour (`None` restores the automatic colour).
pub fn set_text_color(editor: HWND, colour: Option<u32>) {
    let format = CharFormat2W {
        dw_mask: CFM_COLOR,
        dw_effects: if colour.is_none() { CFE_AUTOCOLOR } else { 0 },
        cr_text_color: colour.unwrap_or(0),
        ..CharFormat2W::default()
    };
    editor::set_selection_format(editor, &format);
}

/// Applies a highlight colour (`None` removes the highlight).
pub fn set_highlight(editor: HWND, colour: Option<u32>) {
    let format = CharFormat2W {
        dw_mask: CFM_BACKCOLOR,
        dw_effects: if colour.is_none() {
            CFE_AUTOBACKCOLOR
        } else {
            0
        },
        cr_back_color: colour.unwrap_or(0),
        ..CharFormat2W::default()
    };
    editor::set_selection_format(editor, &format);
}

/// Asks for a custom highlight colour with the standard colour dialog.
pub fn choose_highlight_color(editor: HWND, owner: HWND) {
    let current = editor::selection_format(editor);
    if let Some(colour) = choose_color(owner, current.cr_back_color) {
        set_highlight(editor, Some(colour));
    }
}

/// Removes character formatting (Word's Ctrl+Space).
pub fn clear_formatting(editor: HWND) {
    let mut format = CharFormat2W {
        dw_mask: CFM_FACE
            | CFM_SIZE
            | CFM_BOLD
            | CFM_ITALIC
            | CFM_UNDERLINE
            | CFM_STRIKEOUT
            | CFM_COLOR
            | CFM_BACKCOLOR,
        dw_effects: CFE_AUTOCOLOR | CFE_AUTOBACKCOLOR,
        y_height: BODY_SIZE,
        ..CharFormat2W::default()
    };
    face(&mut format, BODY_FONT);
    editor::set_selection_format(editor, &format);
}

/// Ctrl+] / Ctrl+[: next/previous size on a Word-like ladder.
pub fn step_font_size(editor: HWND, larger: bool) {
    const LADDER: [i32; 14] = [
        160, 180, 200, 220, 240, 260, 280, 320, 360, 400, 480, 560, 720, 960,
    ];
    let current = editor::selection_format(editor);
    let size = if current.dw_mask & CFM_SIZE != 0 {
        current.y_height
    } else {
        BODY_SIZE
    };
    let next = if larger {
        LADDER
            .iter()
            .copied()
            .find(|step| *step > size)
            .unwrap_or(size)
    } else {
        LADDER
            .iter()
            .rev()
            .copied()
            .find(|step| *step < size)
            .unwrap_or(size)
    };
    let format = CharFormat2W {
        dw_mask: CFM_SIZE,
        y_height: next,
        ..CharFormat2W::default()
    };
    editor::set_selection_format(editor, &format);
}

/// Format > Text Color...
pub fn choose_text_color(editor: HWND, owner: HWND) {
    let current = editor::selection_format(editor);
    if let Some(colour) = choose_color(owner, current.cr_text_color) {
        set_text_color(editor, Some(colour));
    }
}

/// The standard colour dialog. Returns the picked `COLORREF`.
fn choose_color(owner: HWND, initial: u32) -> Option<u32> {
    static mut CUSTOM: [u32; 16] = [0x00FF_FFFF; 16];
    let mut dialog: CHOOSECOLORW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = size_of::<CHOOSECOLORW>() as u32;
    dialog.hwndOwner = owner;
    dialog.rgbResult = initial;
    // The UI thread is the only user of `CUSTOM`; the dialog is modal.
    dialog.lpCustColors = std::ptr::addr_of_mut!(CUSTOM) as *mut u32;
    dialog.Flags = CC_RGBINIT | CC_FULLOPEN;
    // SAFETY: `dialog` is fully initialised for the call.
    if unsafe { ChooseColorW(&mut dialog) } == 0 {
        return None;
    }
    Some(dialog.rgbResult)
}

/// Format > Font... (the standard Windows font dialog).
pub fn choose_font(editor: HWND, owner: HWND) {
    let current = editor::selection_format(editor);
    // SAFETY: `GetDpiForWindow` only reads window state.
    let dpi = match unsafe { GetDpiForWindow(editor) } {
        0 => 96,
        value => value as i32,
    };
    let mut logfont: LOGFONTW = unsafe { std::mem::zeroed() };
    let size = if current.dw_mask & CFM_SIZE != 0 {
        current.y_height
    } else {
        BODY_SIZE
    };
    logfont.lfHeight = -win::mul_div(size, dpi, 1440);
    logfont.lfWeight = if current.dw_effects & CFE_BOLD != 0 {
        FW_BOLD as i32
    } else {
        FW_NORMAL as i32
    };
    logfont.lfItalic = u8::from(current.dw_effects & CFE_ITALIC != 0);
    logfont.lfUnderline = u8::from(current.dw_effects & CFE_UNDERLINE != 0);
    logfont.lfStrikeOut = u8::from(current.dw_effects & CFE_STRIKEOUT != 0);
    logfont.lfCharSet = DEFAULT_CHARSET;
    logfont.lfFaceName = current.sz_face_name;

    let mut dialog: CHOOSEFONTW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = size_of::<CHOOSEFONTW>() as u32;
    dialog.hwndOwner = owner;
    dialog.lpLogFont = &mut logfont;
    dialog.Flags = CF_SCREENFONTS | CF_EFFECTS | CF_INITTOLOGFONTSTRUCT;
    dialog.rgbColors = current.cr_text_color;
    // SAFETY: `dialog` and `logfont` outlive the modal call.
    if unsafe { ChooseFontW(&mut dialog) } == 0 {
        return;
    }
    let mut effects = 0;
    if logfont.lfWeight >= FW_BOLD as i32 {
        effects |= CFE_BOLD;
    }
    if logfont.lfItalic != 0 {
        effects |= CFE_ITALIC;
    }
    if logfont.lfUnderline != 0 {
        effects |= CFE_UNDERLINE;
    }
    if logfont.lfStrikeOut != 0 {
        effects |= CFE_STRIKEOUT;
    }
    let format = CharFormat2W {
        dw_mask: CFM_FACE
            | CFM_SIZE
            | CFM_BOLD
            | CFM_ITALIC
            | CFM_UNDERLINE
            | CFM_STRIKEOUT
            | CFM_COLOR,
        dw_effects: effects,
        // `iPointSize` is in tenths of a point; twips are twentieths.
        y_height: dialog.iPointSize * 2,
        cr_text_color: dialog.rgbColors,
        sz_face_name: logfont.lfFaceName,
        ..CharFormat2W::default()
    };
    editor::set_selection_format(editor, &format);
}

/// Width (twips) for a block that spans the editor. Rich Edit converts twips
/// with its device context's DPI, so that is the DPI used here too.
pub fn full_width_twips(editor: HWND) -> i32 {
    let mut rect = unsafe { std::mem::zeroed() };
    // SAFETY: `rect` is a valid out parameter.
    let ok = unsafe { GetClientRect(editor, &mut rect) };
    let dpi = editor_dpi(editor);
    if ok == 0 || rect.right <= 0 {
        return 9000;
    }
    // Leave room for the margins and the vertical scrollbar.
    (win::mul_div(rect.right, 1440, dpi) - 900).clamp(3000, 20000)
}

/// DPI of the device context Rich Edit lays text out with (falls back to the
/// window DPI when no DC is available).
pub fn editor_dpi(editor: HWND) -> i32 {
    // SAFETY: the DC is released right away.
    let dpi = unsafe {
        let dc = GetDC(editor);
        if dc.is_null() {
            GetDpiForWindow(editor) as i32
        } else {
            let value = GetDeviceCaps(dc, LOGPIXELSX as i32);
            ReleaseDC(editor, dc);
            value
        }
    };
    if dpi <= 0 {
        96
    } else {
        dpi
    }
}

/// Every paragraph of the document gets [`TAB_STOP_TWIPS`] tab stops, so a tab
/// is half as wide as Rich Edit's 0.5" default. New paragraphs inherit the
/// paragraph format of the one they were split from.
pub fn apply_document_tabs(editor: HWND) {
    let (from, to) = editor::selection(editor);
    editor::set_selection(editor, 0, -1);
    set_tab_stops(editor, TAB_STOP_TWIPS);
    editor::set_selection(editor, from, to);
}

/// Sets the tab stops of the selected paragraphs to `step` twips apart.
pub fn set_tab_stops(editor: HWND, step: i32) {
    let mut format = ParaFormat2 {
        dw_mask: PFM_TABSTOPS,
        c_tab_count: 32,
        ..ParaFormat2::default()
    };
    for (index, tab) in format.rgx_tabs.iter_mut().enumerate() {
        *tab = step * (index as i32 + 1);
    }
    editor::set_para_format(editor, &format);
}

/// RTF paragraph properties for the code-block tab stops (4 characters).
pub fn code_tab_stops_rtf() -> String {
    let mut out = String::new();
    for step in 1..=12 {
        out.push_str(&format!("\\tx{}", CODE_TAB_TWIPS * step));
    }
    out
}

/// Inserts plain text at the selection. Inside a table cell (a code block)
/// a tab typed through `EM_REPLACESEL` acts like the Tab key and jumps to the
/// next cell or adds a row, so the text goes in as RTF in the code font.
pub fn insert_text(editor: HWND, text: &str) {
    if !editor::in_table(editor) || !text.contains('\t') {
        editor::replace_selection(editor, text, true);
        return;
    }
    // Paragraph breaks are safe through EM_REPLACESEL; only tabs need RTF.
    let mut first = true;
    for line in text.split('\r') {
        if !first {
            editor::replace_selection(editor, "\r", true);
        }
        first = false;
        if line.is_empty() {
            continue;
        }
        let rtf = format!(
            "{{\\rtf1\\ansi\\uc1{{\\fonttbl{{\\f0\\fmodern {CODE_FONT};}}}}\\f0\\fs{} {}}}",
            CODE_SIZE / 10,
            rtf_escape(line)
        );
        editor::insert_rtf(editor, rtf.as_bytes());
    }
}

/// Escapes plain text for RTF (`\par` for line breaks, `\uN?` beyond ASCII).
pub fn rtf_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    for ch in normalized.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\t' => out.push_str("\\tab "),
            '\r' => out.push_str("\\par "),
            ch if (ch as u32) < 0x80 => out.push(ch),
            ch => {
                let mut units = [0u16; 2];
                for unit in ch.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
    out
}

/// Makes sure the caret sits in an empty paragraph, splitting the current one
/// if needed, so a table can be inserted there.
pub fn ensure_empty_paragraph(editor: HWND) {
    let (from, to) = editor::selection(editor);
    if from != to {
        editor::replace_selection(editor, "", true);
    }
    let (caret, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, caret);
    let end = editor::paragraph_end(editor, caret);
    if start == end {
        return;
    }
    editor::set_selection(editor, end, end);
    editor::replace_selection(editor, "\r", true);
}

/// Prepares an empty paragraph for a table/code block and makes sure another
/// paragraph follows it (so the user can keep typing below the block).
/// Returns the position where the block goes.
pub fn prepare_block(editor: HWND) -> i32 {
    ensure_empty_paragraph(editor);
    let (at, _) = editor::selection(editor);
    let length = editor::text_length(editor);
    // Last paragraph of the story: add one after it.
    if editor::paragraph_end(editor, at) + 1 >= length {
        editor::replace_selection(editor, "\r", true);
        editor::set_selection(editor, at, at);
    }
    at
}

/// Insert > Code Block: a one-cell, grey, bordered table in Consolas. Plain
/// paragraph shading and borders are not rendered by Rich Edit; table cells
/// are, so the cell is the box. The selected text (if any) becomes the code.
pub fn insert_code_block(editor: HWND) {
    let (from, to) = editor::selection(editor);
    let code = if from != to {
        editor::text_range(editor, from, to)
    } else {
        String::new()
    };
    let at = prepare_block(editor);

    let (br, bg, bb) = CODE_BLOCK_BACK;
    let (lr, lg, lb) = CODE_BLOCK_BORDER;
    let border = "\\brdrs\\brdrw10\\brdrcf2";
    let tabs = code_tab_stops_rtf();
    let rtf = format!(
        "{{\\rtf1\\ansi\\uc1\\deff0{{\\fonttbl{{\\f0\\fnil {BODY_FONT};}}{{\\f1\\fmodern {CODE_FONT};}}}}\
         {{\\colortbl ;\\red{br}\\green{bg}\\blue{bb};\\red{lr}\\green{lg}\\blue{lb};}}\
         \\trowd\\trgaph144\\trleft0\\clcbpat1\\clbrdrt{border}\\clbrdrl{border}\\clbrdrb{border}\\clbrdrr{border}\\cellx{width}\
         \\pard\\intbl\\f1\\fs20{tabs} {body}\\cell\\row}}",
        width = full_width_twips(editor),
        body = rtf_escape(code.trim_end_matches('\r')),
    );
    if editor::insert_rtf(editor, rtf.as_bytes()) {
        // Row start (U+FFF9) + its paragraph mark, then the cell text.
        let caret = at + 2 + code.trim_end_matches('\r').encode_utf16().count() as i32;
        editor::set_selection(editor, caret, caret);
    }
}
