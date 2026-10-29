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

/// Default body font (Preferences); see `nodi_core::markdown_in::body_font`.
pub fn body_font() -> String {
    nodi_core::markdown_in::body_font()
}
pub const CODE_FONT: &str = "Consolas";
/// Twips (1/20 pt).
pub fn body_size() -> i32 {
    nodi_core::markdown_in::body_size() * 10
}
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
        _ => (body_size(), 0),
    };
    let mut format = CharFormat2W {
        dw_mask: CFM_SIZE | CFM_BOLD | CFM_FACE | CFM_COLOR,
        dw_effects: bold | CFE_AUTOCOLOR,
        y_height: size,
        ..CharFormat2W::default()
    };
    face(&mut format, &body_font());
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
            .unwrap_or(body_size()),
        ..CharFormat2W::default()
    };
    if level > 0 {
        format.dw_effects |= CFE_BOLD;
    }
    face(&mut format, &body_font());
    editor::set_selection(editor, start, start + len);
    // Called on every keystroke in a heading: only format when something
    // differs, so the undo stack does not fill up with formatting steps.
    let current = editor::selection_format(editor);
    let same = current.dw_mask & format.dw_mask == format.dw_mask
        && current.dw_effects & (CFE_HIDDEN | CFE_BOLD)
            == format.dw_effects & (CFE_HIDDEN | CFE_BOLD)
        && current.cr_text_color == format.cr_text_color
        && current.y_height == format.y_height
        && face_name(&current) == body_font();
    if !same {
        editor::set_selection_format(editor, &format);
    }
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
    let _frozen = editor::Frozen::new(editor);
    let format = CharFormat2W {
        dw_mask: CFM_HIDDEN | CFM_COLOR,
        dw_effects: if visible { 0 } else { CFE_HIDDEN },
        cr_text_color: HEADING_MARKER_GREY,
        ..CharFormat2W::default()
    };
    editor::set_selection(editor, start, start + len);
    let current = editor::selection_format(editor);
    let same = current.dw_mask & format.dw_mask == format.dw_mask
        && current.dw_effects & CFE_HIDDEN == format.dw_effects
        && current.cr_text_color == HEADING_MARKER_GREY;
    if !same {
        editor::set_selection_format(editor, &format);
    }
    editor::set_selection(editor, from, to);
}

/// After the `# ` marker has been painted grey, typing continues right after
/// it and would inherit that grey. This resets the heading body to the normal
/// text colour (when it inherited the marker's grey) and sets the insertion
/// colour at a caret that sits at the start of the body.
fn normalise_heading_colour(editor: HWND, caret: i32) {
    if editor::in_table(editor) {
        return;
    }
    let start = editor::paragraph_start(editor, caret);
    let end = editor::paragraph_end(editor, caret);
    let text = editor::text_range(editor, start, end);
    let marker = heading_marker_len(&text) as i32;
    if marker == 0 {
        return;
    }
    let body = start + marker;
    let auto = CharFormat2W {
        dw_mask: CFM_COLOR,
        dw_effects: CFE_AUTOCOLOR,
        ..CharFormat2W::default()
    };
    let mut fixed = false;
    if end > body {
        let saved = editor::selection(editor);
        editor::set_selection(editor, body, body + 1);
        let first = editor::selection_format(editor);
        editor::set_selection(editor, saved.0, saved.1);
        let grey = first.dw_mask & CFM_COLOR != 0
            && first.dw_effects & CFE_AUTOCOLOR == 0
            && first.cr_text_color == HEADING_MARKER_GREY;
        if grey {
            editor::set_selection(editor, body, end);
            editor::set_selection_format(editor, &auto);
            fixed = true;
        }
    }
    if fixed || caret <= body + 1 {
        editor::set_selection(editor, caret, caret);
        editor::set_selection_format(editor, &auto);
    }
}

/// Where a position moves when the marker at a paragraph start is replaced.
fn shifted(pos: i32, start: i32, end: i32, old_len: i32, new_len: i32) -> i32 {
    if pos < start {
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
    let _frozen = editor::Frozen::new(editor);
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
    if new_from == new_to {
        normalise_heading_colour(editor, new_from);
    }
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
    // Typing in a body paragraph (no `#` marker, body-size text at the caret)
    // needs nothing: return before touching the selection at all.
    if prefix.is_none() && heading_level_at(editor) == 0 {
        return;
    }
    // The probing below moves the selection; nothing of it may be painted.
    let _frozen = editor::Frozen::new(editor);
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
    normalise_heading_colour(editor, from);
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
    let _frozen = editor::Frozen::new(editor);
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

/// Deepest list level Tab can reach: 30 steps of a quarter inch, more than a
/// page is wide, so in practice there is no limit.
const LIST_MAX_LEVEL: usize = 30;
/// `PARAFORMAT2::wNumberingStyle` flag: start a new run at `wNumberingStart`.
const PFNS_NEWNUMBER: u16 = 0x8000;
/// Number-to-text distance of an item the user restarted at 1. Rich Edit
/// does not keep `PFNS_NEWNUMBER` for a start of 1 in RTF (it writes the same
/// `\pnstart1` as for a continuing item), but it keeps this distance
/// (`\pnindent`), one twip more than usual and invisible.
const RESTART_TAB: u16 = INDENT_STEP_TWIPS as u16 + 1;
/// Checklist markers: an open and a ticked box, each followed by a tab.
pub const CHECK_OPEN: &str = "☐";
pub const CHECK_DONE: &str = "☑";

/// What kind of list a paragraph belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Bullet,
    Numbered,
    Check,
}

/// `(kind, marker)` of a literal list marker at the start of `text`: a deeper
/// bullet glyph or a checkbox, followed by a tab. The marker slice is what is
/// stored in the text, so callers can measure it in bytes or UTF-16 units.
fn literal_marker(text: &str) -> Option<(ListKind, &str)> {
    let glyphs = BULLET_GLYPHS.iter().map(|glyph| (ListKind::Bullet, *glyph));
    let checks = [CHECK_OPEN, CHECK_DONE].map(|glyph| (ListKind::Check, glyph));
    for (kind, glyph) in glyphs.chain(checks) {
        if text
            .strip_prefix(glyph)
            .is_some_and(|rest| rest.starts_with('\t'))
        {
            return Some((kind, &text[..glyph.len() + 1]));
        }
    }
    None
}

fn units(text: &str) -> i32 {
    text.encode_utf16().count() as i32
}

/// The literal marker a new item of `kind` at `level` starts with (none for
/// native items: numbers and the first bullet level).
fn marker_for(kind: ListKind, level: usize) -> Option<String> {
    match kind {
        ListKind::Numbered => None,
        ListKind::Bullet if level <= 1 => None,
        ListKind::Bullet => Some(format!(
            "{}\t",
            BULLET_GLYPHS[(level - 1) % BULLET_GLYPHS.len()]
        )),
        ListKind::Check => Some(format!("{CHECK_OPEN}\t")),
    }
}

/// Increase the start indent of every selected paragraph by `delta` twips.
pub fn change_indent(editor: HWND, delta: i32) {
    let Some(_busy) = Busy::enter() else {
        return;
    };
    let _quiet = editor::QuietSelection::new(editor);
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

/// The list the caret paragraph belongs to, if any.
pub fn list_kind(editor: HWND) -> Option<ListKind> {
    let format = editor::para_format(editor);
    if format.w_numbering != 0 {
        return Some(if format.w_numbering == PFN_ARABIC {
            ListKind::Numbered
        } else {
            ListKind::Bullet
        });
    }
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let end = editor::paragraph_end(editor, from);
    let text = editor::text_range(editor, start, end);
    literal_marker(&text).map(|(kind, _)| kind)
}

/// Level (1, 2, ...) of the caret's list item, from its indent; 0 for
/// ordinary text.
pub fn list_level(editor: HWND) -> usize {
    if list_kind(editor).is_none() {
        return 0;
    }
    level_for_indent(paragraph_indent(editor))
}

/// The list level whose marker sits at `indent` twips.
fn level_for_indent(indent: i32) -> usize {
    ((indent.max(0) / INDENT_STEP_TWIPS) as usize + 1).min(LIST_MAX_LEVEL)
}

fn list_para_format(kind: ListKind, level: usize) -> ParaFormat2 {
    let indent = (level.saturating_sub(1) as i32) * INDENT_STEP_TWIPS;
    // Rich Edit has one native bullet glyph, so deeper bullet levels and
    // checkboxes are a literal glyph + tab with a hanging indent instead.
    let native = marker_for(kind, level).is_none();
    ParaFormat2 {
        dw_mask: PFM_NUMBERING
            | PFM_NUMBERINGSTYLE
            | PFM_NUMBERINGSTART
            | PFM_NUMBERINGTAB
            | PFM_STARTINDENT
            | PFM_OFFSET,
        w_numbering: match (native, kind) {
            (false, _) => 0,
            (true, ListKind::Numbered) => PFN_ARABIC,
            (true, _) => PFN_BULLET,
        },
        w_numbering_style: if native && kind == ListKind::Numbered {
            PFNS_PERIOD
        } else {
            0
        },
        w_numbering_start: if native { 1 } else { 0 },
        w_numbering_tab: if native { INDENT_STEP_TWIPS as u16 } else { 0 },
        dx_start_indent: indent,
        dx_offset: INDENT_STEP_TWIPS,
        ..ParaFormat2::default()
    }
}

/// Applies a list of `kind` at `level` to every selected paragraph.
pub fn apply_list_level(editor: HWND, kind: ListKind, level: usize) {
    {
        let Some(_busy) = Busy::enter() else {
            return;
        };
        apply_list_level_impl(editor, kind, level);
    }
    renumber_near(editor);
}

fn apply_list_level_impl(editor: HWND, kind: ListKind, level: usize) {
    let _quiet = editor::QuietSelection::new(editor);
    let level = level.clamp(1, LIST_MAX_LEVEL);
    let (from, to) = editor::selection(editor);
    let raw = editor::raw_text(editor);
    let starts = selected_paragraphs(&raw, from, to);
    let mut new_from = from;
    let mut new_to = to;
    for &start in starts.iter().rev() {
        let end = paragraph_end_in(&raw, start);
        let text = text_in(&raw, start, end);
        let old = literal_marker(&text);
        let old_len = old.map_or(0, |(_, marker)| units(marker));
        if old_len > 0 {
            editor::set_selection(editor, start, start + old_len);
            editor::replace_selection(editor, "", true);
        }
        // Switch to the target format first: as long as the paragraph is a
        // native list item, a typed/inserted tab changes the level instead of
        // inserting a character.
        editor::set_selection(editor, start, end - old_len);
        editor::set_para_format(editor, &list_para_format(kind, level));
        let mut new_len = 0;
        // A ticked box stays ticked when it moves to another level.
        let marker = match old {
            Some((ListKind::Check, marker)) if kind == ListKind::Check => Some(marker.to_owned()),
            _ => marker_for(kind, level),
        };
        if let Some(marker) = marker {
            new_len = units(&marker);
            editor::set_selection(editor, start, start);
            editor::replace_selection(editor, &marker, true);
        }
        new_from = shifted(new_from, start, end, old_len, new_len);
        new_to = shifted(new_to, start, end, old_len, new_len);
    }
    editor::set_selection(editor, new_from, new_to);
}

/// Turns the typed `- ` / `1. ` / `[ ] ` (already removed) into a list item
/// whose marker sits where the text was: an indented paragraph becomes a
/// deeper item instead of jumping back to the left margin.
pub fn start_list(editor: HWND, kind: ListKind) {
    let level = level_for_indent(paragraph_indent(editor));
    apply_list_level(editor, kind, level);
}

/// Enter on an empty item: removes the bullet/number and keeps the text at the
/// column where it was, as a plain indented paragraph. Returns false when the
/// paragraph is not an empty list item.
pub fn end_list(editor: HWND) -> bool {
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let end = editor::paragraph_end(editor, from);
    let text = editor::text_range(editor, start, end);
    let content = match literal_marker(&text) {
        Some((_, marker)) => &text[marker.len()..],
        None => &text[..],
    };
    if !content.trim().is_empty() {
        return false;
    }
    remove_list_marker(editor)
}

/// Enter inside a literal item (deeper bullet, checkbox): starts the next item
/// with the same kind of marker and indent. Returns false for native lists
/// (Rich Edit does it).
pub fn new_literal_item(editor: HWND) -> bool {
    let Some(_busy) = Busy::enter() else {
        return false;
    };
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let end = editor::paragraph_end(editor, from);
    let text = editor::text_range(editor, start, end);
    let Some((kind, _)) = literal_marker(&text) else {
        return false;
    };
    let level = level_for_indent(paragraph_indent(editor));
    let Some(marker) = marker_for(kind, level) else {
        return false;
    };
    editor::replace_selection(editor, "\r", true);
    let new_start = editor::paragraph_start(editor, from + 1);
    editor::set_selection(editor, new_start, new_start);
    editor::replace_selection(editor, &marker, true);
    let caret = new_start + units(&marker);
    editor::set_selection(editor, caret, caret);
    true
}

/// End of the caret paragraph's literal marker (where its text starts), when
/// it has one.
pub fn literal_marker_end(editor: HWND) -> Option<i32> {
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let end = editor::paragraph_end(editor, from);
    let text = editor::text_range(editor, start, end);
    literal_marker(&text).map(|(_, marker)| start + units(marker))
}

/// Backspace at the start of a list item's text: removes the bullet, number or
/// checkbox but keeps the indentation, so the text stays where it was.
pub fn remove_list_marker(editor: HWND) -> bool {
    {
        let Some(_busy) = Busy::enter() else {
            return false;
        };
        let format = editor::para_format(editor);
        let (from, _) = editor::selection(editor);
        let start = editor::paragraph_start(editor, from);
        let end = editor::paragraph_end(editor, from);
        let text = editor::text_range(editor, start, end);
        let literal = literal_marker(&text);
        if format.w_numbering == 0 && literal.is_none() {
            return false;
        }
        if let Some((_, marker)) = literal {
            editor::set_selection(editor, start, start + units(marker));
            editor::replace_selection(editor, "", true);
        }
        let para = ParaFormat2 {
            dw_mask: PFM_NUMBERING
                | PFM_NUMBERINGSTYLE
                | PFM_NUMBERINGSTART
                | PFM_NUMBERINGTAB
                | PFM_STARTINDENT
                | PFM_OFFSET,
            dx_start_indent: format.dx_start_indent.max(0) + INDENT_STEP_TWIPS,
            ..ParaFormat2::default()
        };
        editor::set_selection(editor, start, editor::paragraph_end(editor, start));
        editor::set_para_format(editor, &para);
        editor::set_selection(editor, start, start);
    }
    renumber_near(editor);
    true
}

/// Ticks or clears the checkbox of the paragraph containing `pos`. Returns
/// false when that paragraph is not a checklist item.
pub fn toggle_check_at(editor: HWND, pos: i32) -> bool {
    let Some(_busy) = Busy::enter() else {
        return false;
    };
    let start = editor::paragraph_start(editor, pos);
    let end = editor::paragraph_end(editor, pos);
    let text = editor::text_range(editor, start, end);
    let Some((ListKind::Check, marker)) = literal_marker(&text) else {
        return false;
    };
    let next = if marker.starts_with(CHECK_OPEN) {
        CHECK_DONE
    } else {
        CHECK_OPEN
    };
    let (from, to) = editor::selection(editor);
    let glyph = units(&marker[..marker.len() - 1]);
    editor::set_selection(editor, start, start + glyph);
    editor::replace_selection(editor, next, true);
    editor::set_selection(editor, from, to);
    true
}

/// The checkbox under a click at `pos` (a character position): the paragraph
/// start when that character is a checklist box.
pub fn checkbox_at(editor: HWND, pos: i32) -> Option<i32> {
    let start = editor::paragraph_start(editor, pos);
    if pos != start {
        return None;
    }
    let end = editor::paragraph_end(editor, pos);
    let text = editor::text_range(editor, start, end);
    match literal_marker(&text) {
        Some((ListKind::Check, _)) => Some(start),
        _ => None,
    }
}

/// Recomputes the numbers of numbered items, the way an outline reads: a
/// list continues at its own indent across deeper items and indented
/// continuation paragraphs, a deeper list starts at 1, and plain text, a
/// bullet or a table at the same indent ends it. An item the user restarted
/// (Restart Numbering) keeps 1.
///
/// Rich Edit itself numbers every run of consecutive numbered paragraphs
/// 1, 2, 3... whatever their indent, so items that do not simply follow the
/// paragraph before them get an explicit start.
pub fn renumber(editor: HWND) {
    renumber_impl(editor, true);
}

/// [`renumber`] after an edit: only the stretch of the note around the
/// selection that a list can reach — from the last paragraph before it that
/// ends every list (plain text or a bullet at the left margin, a table) to
/// the first such paragraph after it. A big note with a short list then
/// stays fast while typing.
pub fn renumber_near(editor: HWND) {
    renumber_impl(editor, false);
}

/// True when this paragraph ends every open list (and the numbering state is
/// empty after it).
fn ends_all_lists(format: &ParaFormat2) -> bool {
    format.w_effects & editor::PFE_TABLE != 0
        || (format.w_numbering != PFN_ARABIC && format.dx_start_indent <= 0)
}

fn renumber_impl(editor: HWND, whole: bool) {
    let Some(_busy) = Busy::enter() else {
        return;
    };
    let raw = editor::raw_text(editor);
    let (from, to) = editor::selection(editor);
    let _frozen = editor::Frozen::new(editor);
    // Every selection move below would reach the frame as EN_SELCHANGE
    // (status bar, toolbar, heading upkeep): switch that off meanwhile.
    let _quiet = editor::QuietSelection::new(editor);
    let mut starts = vec![0usize];
    for (index, unit) in raw.iter().enumerate() {
        if is_paragraph_break(*unit) {
            starts.push(index + 1);
        }
    }
    let paragraph_of = |pos: i32| {
        let pos = pos.max(0) as usize;
        match starts.binary_search(&pos) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        }
    };
    let first_edit = paragraph_of(from);
    let last_edit = paragraph_of(to);
    let format_at = |index: usize| {
        editor::set_selection(editor, starts[index] as i32, starts[index] as i32);
        editor::para_format(editor)
    };
    let mut first = 0;
    if !whole {
        let mut index = first_edit;
        while index > 0 {
            index -= 1;
            if ends_all_lists(&format_at(index)) {
                first = index + 1;
                break;
            }
        }
    }

    let mut open: Vec<(i32, u16)> = Vec::new();
    let mut previous: Option<u16> = None;
    for index in first..starts.len() {
        let start = starts[index];
        if start > raw.len() {
            break;
        }
        let end = starts.get(index + 1).map_or(raw.len(), |next| next - 1);
        let format = format_at(index);
        let indent = format.dx_start_indent.max(0);
        if format.w_effects & editor::PFE_TABLE != 0 {
            open.clear();
            previous = None;
        } else if format.w_numbering == PFN_ARABIC {
            open.retain(|(level, _)| *level <= indent);
            let restarted = format.w_numbering_tab == RESTART_TAB;
            let number = match open.last_mut() {
                Some((level, count)) if *level == indent => {
                    *count = if restarted { 1 } else { *count + 1 };
                    *count
                }
                _ => {
                    open.push((indent, 1));
                    1
                }
            };
            let follows = previous == Some(number.wrapping_sub(1));
            let want_new = !follows && (previous.is_some() || number > 1);
            let has_new = format.w_numbering_style & PFNS_NEWNUMBER != 0;
            let start_wrong = !follows && format.w_numbering_start != number;
            if has_new != want_new || start_wrong {
                let style = (format.w_numbering_style & !PFNS_NEWNUMBER)
                    | if want_new { PFNS_NEWNUMBER } else { 0 };
                editor::set_selection(editor, start as i32, end as i32);
                editor::set_para_format(
                    editor,
                    &ParaFormat2 {
                        dw_mask: PFM_NUMBERINGSTYLE | PFM_NUMBERINGSTART,
                        w_numbering_style: style,
                        w_numbering_start: if follows {
                            format.w_numbering_start
                        } else {
                            number
                        },
                        ..ParaFormat2::default()
                    },
                );
            }
            previous = Some(number);
        } else {
            open.retain(|(level, _)| *level < indent);
            previous = None;
        }
        if !whole && index > last_edit && ends_all_lists(&format) {
            break;
        }
    }
    editor::set_selection(editor, from, to);
}

/// Format > Restart Numbering (`restart`) / Continue Numbering for the caret
/// item of a numbered list.
pub fn set_numbering_restart(editor: HWND, restart: bool) {
    if list_kind(editor) != Some(ListKind::Numbered) {
        return;
    }
    let (from, to) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    editor::set_selection(editor, start, editor::paragraph_end(editor, start));
    editor::set_para_format(
        editor,
        &ParaFormat2 {
            dw_mask: PFM_NUMBERINGTAB,
            w_numbering_tab: if restart {
                RESTART_TAB
            } else {
                INDENT_STEP_TWIPS as u16
            },
            ..ParaFormat2::default()
        },
    );
    editor::set_selection(editor, from, to);
    renumber_near(editor);
}

/// True when the caret item was restarted at 1 by the user.
pub fn numbering_restarted(editor: HWND) -> bool {
    let format = editor::para_format(editor);
    format.w_numbering == PFN_ARABIC && format.w_numbering_tab == RESTART_TAB
}

/// Enter in a numbered list: the new item copied the old one's paragraph
/// format, including a user restart; only the first item keeps that.
pub fn after_new_numbered_item(editor: HWND) {
    if numbering_restarted(editor) {
        let (from, to) = editor::selection(editor);
        let start = editor::paragraph_start(editor, from);
        editor::set_selection(editor, start, editor::paragraph_end(editor, start));
        editor::set_para_format(
            editor,
            &ParaFormat2 {
                dw_mask: PFM_NUMBERINGTAB,
                w_numbering_tab: INDENT_STEP_TWIPS as u16,
                ..ParaFormat2::default()
            },
        );
        editor::set_selection(editor, from, to);
    }
    renumber_near(editor);
}

// --------------------------------------------------------------- separators

/// A wide, thin grey bar, encoded without a LaTeX chunk so the Markdown reader
/// recognises it as a separator rather than a formula.
fn separator_png() -> Vec<u8> {
    const WIDTH: u32 = 600;
    const HEIGHT: u32 = 6;
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 2) as usize);
    for _ in 0..WIDTH * HEIGHT {
        pixels.push(0x88);
        pixels.push(0xFF);
    }
    nodi_core::math::encode_png(WIDTH, HEIGHT, &pixels, "")
}

/// Resets the caret paragraph to plain body geometry (no list, no indent), so a
/// block inserted at the caret starts at the left margin.
fn plain_paragraph(editor: HWND) {
    let (from, _) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let end = editor::paragraph_end(editor, from);
    editor::set_selection(editor, start, end);
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

/// Inserts the horizontal-rule picture in a paragraph of its own and leaves
/// the caret at the start of a new paragraph after it. The rule is a plain
/// picture, recognised by its shape (`nodi_core::image::is_rule_picture`): a
/// `nodi-hr:` link around it would be underlined by Rich Edit.
fn insert_separator_impl(editor: HWND) {
    // 60 twips = 4 px at 96 DPI: a visibly "thick" rule (>= 3 px at 100 %).
    insert_flat_picture(editor, &separator_png(), 60, "separator");
}

/// Insert > Page Break (Ctrl+Enter, or `----` + Enter): the end of one page
/// and the start of the next, drawn across the note.
pub fn insert_page_break(editor: HWND) {
    let Some(_busy) = Busy::enter() else {
        return;
    };
    insert_flat_picture(
        editor,
        &nodi_core::image::page_break_png(),
        360,
        "page break",
    );
}

/// A full-width picture `height_twips` high in a paragraph of its own; the
/// caret ends at the start of a new paragraph after it.
fn insert_flat_picture(editor: HWND, png: &[u8], height_twips: i64, label: &str) {
    let (mut from, to) = editor::selection(editor);
    if to > from {
        editor::set_selection(editor, from, from);
        editor::replace_selection(editor, "", true);
    }
    if from > editor::paragraph_start(editor, from) {
        editor::replace_selection(editor, "\r", true);
        from += 1;
    }
    plain_paragraph(editor);
    // `EM_INSERTIMAGE` takes HIMETRIC (0.01 mm); the editor width is in twips.
    let width_twips = full_width_twips(editor);
    let width = (width_twips as i64 * 2540 / 1440) as i32;
    let height = (height_twips * 2540 / 1440) as i32;
    if image::insert_sized(editor, png, width, height, height, label).is_err() {
        return;
    }
    editor::set_selection(editor, from + 1, from + 1);
    editor::replace_selection(editor, "\r", true);
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
    let marker = text.trim();
    // `---` is a separator, `----` (or longer) a page break.
    let page = marker.len() >= 4 && marker.chars().all(|ch| ch == '-');
    if (marker != "---" && !page) || from < start {
        return false;
    }
    // Remove the marker first, then insert at a collapsed caret: replacing a
    // multi-character selection with the field makes Rich Edit move it.
    editor::set_selection(editor, start, end);
    editor::replace_selection(editor, "", true);
    editor::set_selection(editor, start, start);
    if page {
        insert_flat_picture(
            editor,
            &nodi_core::image::page_break_png(),
            360,
            "page break",
        );
    } else {
        insert_separator_impl(editor);
    }
    true
}

/// Toggles a list of `kind` (toolbar, Format menu): off when the caret item
/// already is one, otherwise every selected paragraph becomes one, keeping
/// the level of an existing item or the indent of plain text.
pub fn toggle_list(editor: HWND, kind: ListKind) {
    if editor::in_table(editor) {
        return;
    }
    match list_kind(editor) {
        Some(current) if current == kind => remove_lists(editor),
        Some(_) => apply_list_level(editor, kind, list_level(editor)),
        None => start_list(editor, kind),
    }
}

/// Turns lists off for every selected paragraph (Format > Normal / toolbar);
/// the text keeps its column.
fn remove_lists(editor: HWND) {
    {
        let Some(_busy) = Busy::enter() else {
            return;
        };
        let _quiet = editor::QuietSelection::new(editor);
        let (from, to) = editor::selection(editor);
        let raw = editor::raw_text(editor);
        let mut removed_before = 0;
        for &start in selected_paragraphs(&raw, from, to).iter().rev() {
            let end = paragraph_end_in(&raw, start);
            let text = text_in(&raw, start, end);
            let literal = literal_marker(&text).map_or(0, |(_, marker)| units(marker));
            editor::set_selection(editor, start, start);
            let format = editor::para_format(editor);
            let listed = literal > 0 || format.w_numbering != 0;
            if literal > 0 {
                editor::set_selection(editor, start, start + literal);
                editor::replace_selection(editor, "", true);
                if start <= from {
                    removed_before = literal.min(from - start);
                }
            }
            if !listed {
                continue;
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
                    dx_start_indent: format.dx_start_indent.max(0),
                    ..ParaFormat2::default()
                },
            );
        }
        // Only the first paragraph's marker can sit before the caret.
        let first = editor::paragraph_start(editor, from);
        let caret = (from - removed_before).max(first);
        editor::set_selection(editor, caret, caret);
    }
    renumber_near(editor);
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
        face(&mut format, &body_font());
        format.y_height = body_size();
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
        y_height: body_size(),
        ..CharFormat2W::default()
    };
    face(&mut format, &body_font());
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
        body_size()
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
pub(crate) fn choose_color(owner: HWND, initial: u32) -> Option<u32> {
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
        body_size()
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
    (win::mul_div(rect.right, 1440, dpi) - 300).clamp(3000, 20000)
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

/// Insert > Code Block: a two-row, bordered table — a header strip with the
/// language label and a Copy link, then the Consolas code cell. Plain
/// paragraph shading is not rendered by Rich Edit; table cells are, so the
/// rows are the box. The selected text (if any) becomes the code.
pub fn insert_code_block(editor: HWND) {
    insert_code(editor, "", false);
}

/// Insert > Output Block (Ctrl+Shift+O): the same structure with a dark
/// background, light monospace text and the label `output`.
pub fn insert_output_block(editor: HWND) {
    insert_code(editor, "", true);
}

fn insert_code(editor: HWND, language: &str, output: bool) {
    let (from, to) = editor::selection(editor);
    let code = if from != to {
        editor::text_range(editor, from, to)
    } else {
        String::new()
    };
    let at = prepare_block(editor);
    let width = full_width_twips(editor);
    let rtf = code_block_rtf(language, &code, width, output);
    if editor::insert_rtf(editor, rtf.as_bytes()) {
        if let Some(start) = code_start_after(&editor::raw_text(editor), at.max(0) as usize) {
            let caret = start as i32 + code.trim_end_matches('\r').encode_utf16().count() as i32;
            editor::set_selection(editor, caret, caret);
        }
    }
}

/// Colours of a code-block table (see [`code_block_rtf`]).
fn code_colours() -> String {
    let (br, bg, bb) = CODE_BLOCK_BACK;
    let (lr, lg, lb) = CODE_BLOCK_BORDER;
    let mut out = String::from("{\\colortbl ;");
    // 1..=5: light block, 6..=11: output block, 12: dark code text.
    for (red, green, blue) in [
        (br, bg, bb),
        (lr, lg, lb),
        (235, 238, 241),
        (110, 118, 129),
        (0, 102, 204),
        (30, 30, 30),
        (60, 60, 60),
        (45, 45, 45),
        (200, 200, 200),
        (220, 220, 220),
        (110, 170, 255),
        (31, 31, 31),
    ] {
        out.push_str(&format!("\\red{red}\\green{green}\\blue{blue};"));
    }
    out.push('}');
    out
}

/// RTF for a code block: header row (label + `nodi-copy:` link) and code row.
fn code_block_rtf(language: &str, code: &str, width: i32, output: bool) -> String {
    let (code_back, _border, header_back, label_colour, link_colour, text_colour) = if output {
        (6, 7, 8, 9, 11, "\\cf10")
    } else {
        // #1F1F1F: readable on the light grey code cell and clearly darker
        // than the grey label.
        (1, 2, 3, 4, 5, "\\cf12")
    };
    let label = match language.trim() {
        "" if output => "output",
        "" => "code",
        other => other,
    };
    let edge = if output { 7 } else { 2 };
    let border = format!("\\brdrs\\brdrw10\\brdrcf{edge}");
    let mut rtf = String::from("{\\rtf1\\ansi\\uc1\\deff0");
    rtf.push_str(&format!(
        "{{\\fonttbl{{\\f0\\fnil {};}}{{\\f1\\fmodern {CODE_FONT};}}}}",
        body_font()
    ));
    rtf.push_str(&code_colours());

    // Header row: label on the left, Copy at the right tab stop.
    rtf.push_str("\\trowd\\trgaph108\\trleft0");
    rtf.push_str(&format!(
        "\\clcbpat{header_back}\\clbrdrt{border}\\clbrdrl{border}\
         \\clbrdrb{border}\\clbrdrr{border}\\cellx{width}"
    ));
    rtf.push_str(&format!(
        "\\pard\\intbl\\tqr\\tx{}\\f1\\fs16{{\\cf{label_colour} ",
        (width - 200).max(0)
    ));
    rtf.push_str(&rtf_escape(label));
    rtf.push_str("}\\cf0\\tab{\\field{\\*\\fldinst{HYPERLINK \"");
    rtf.push_str(&nodi_core::links::format_copy_link());
    rtf.push_str("\"}}{\\fldrslt{\\ul\\cf");
    rtf.push_str(&link_colour.to_string());
    rtf.push_str(" Copy}}}\\cell\\row");

    // Code row.
    rtf.push_str("\\trowd\\trgaph108\\trleft0");
    rtf.push_str(&format!(
        "\\clcbpat{code_back}\\clbrdrt{border}\\clbrdrl{border}\
         \\clbrdrb{border}\\clbrdrr{border}\\cellx{width}"
    ));
    rtf.push_str(&format!(
        "\\pard\\intbl\\f1\\fs20{text_colour}{} ",
        code_tab_stops_rtf()
    ));
    rtf.push_str(&rtf_escape(code.trim_end_matches('\r')));
    rtf.push_str("\\cell\\row}");
    rtf
}

/// Start of the code text: after the second row marker (`U+FFF9`).
fn code_start_after(raw: &[u16], from: usize) -> Option<usize> {
    let from = from.min(raw.len());
    let first = raw[from..].iter().position(|unit| *unit == 0xFFF9)? + from;
    let second = raw[first + 1..].iter().position(|unit| *unit == 0xFFF9)? + first + 1;
    let mut start = second + 1;
    while start < raw.len() && raw[start] == 0x0D {
        start += 1;
    }
    Some(start)
}

/// The code text of the block whose header contains `at` (the Copy link).
pub fn code_block_text(editor: HWND, at: i32) -> Option<String> {
    let raw = editor::raw_text(editor);
    let at = at.max(0) as usize;
    let header_end = raw.get(at..)?.iter().position(|unit| *unit == 0x0007)? + at;
    let row = raw[header_end + 1..]
        .iter()
        .position(|unit| *unit == 0xFFF9)?
        + header_end
        + 1;
    let mut start = row + 1;
    while start < raw.len() && raw[start] == 0x0D {
        start += 1;
    }
    let mut units = Vec::new();
    for unit in &raw[start..] {
        match *unit {
            0x0007 => break,
            0x000D => units.push(0x000A),
            0xFFF9 | 0xFFFB | 0xFFFC => {}
            unit => units.push(unit),
        }
    }
    while units.last() == Some(&0x000A) {
        units.pop();
    }
    Some(String::from_utf16_lossy(&units))
}
