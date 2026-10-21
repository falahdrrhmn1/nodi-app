//! Character and paragraph formatting: headings, lists, alignment, inline
//! code, code blocks, highlight, colour and font.

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
    self, CharFormat2W, ParaFormat2, CFE_BOLD, CFE_ITALIC, CFE_STRIKEOUT, CFE_UNDERLINE, CFM_BOLD,
    CFM_FACE, CFM_ITALIC, CFM_SIZE, CFM_STRIKEOUT, CFM_UNDERLINE, PFM_ALIGNMENT, PFM_NUMBERING,
    PFM_NUMBERINGSTART, PFM_NUMBERINGSTYLE, PFM_NUMBERINGTAB, PFM_OFFSET, PFM_SPACEAFTER,
    PFM_SPACEBEFORE, PFM_STARTINDENT, PFM_TABSTOPS,
};

pub const BODY_FONT: &str = "Segoe UI";
pub const CODE_FONT: &str = "Consolas";
/// Twips (1/20 pt).
pub const BODY_SIZE: i32 = 220;
const CODE_SIZE: i32 = 200;
/// Heading sizes in twips for levels 1..=4.
pub const HEADING_SIZES: [i32; 4] = [400, 320, 280, 250];
/// Default tab stop for body text: half of Rich Edit's 720-twip (0.5") stop.
pub const TAB_STOP_TWIPS: i32 = 360;
/// A tab inside a code block lines up with four characters of the code font.
pub const CODE_TAB_TWIPS: i32 = 480;

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

/// Runs `body` with the selection widened to whole paragraphs, then puts the
/// original selection back.
fn on_paragraphs(editor: HWND, body: impl FnOnce(HWND)) {
    let (from, to) = editor::selection(editor);
    let start = editor::paragraph_start(editor, from);
    let end = editor::paragraph_end(editor, to.max(from));
    editor::set_selection(editor, start, end);
    body(editor);
    editor::set_selection(editor, from, to);
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

/// `level` 1..=4 makes a heading, 0 turns the paragraph back into body text.
pub fn heading(editor: HWND, level: usize) {
    on_paragraphs(editor, |editor| {
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
        editor::set_selection_format(editor, &format);
        let spacing = ParaFormat2 {
            dw_mask: PFM_SPACEBEFORE | PFM_SPACEAFTER,
            dy_space_before: if level > 0 { 180 } else { 0 },
            dy_space_after: if level > 0 { 60 } else { 0 },
            ..ParaFormat2::default()
        };
        editor::set_para_format(editor, &spacing);
    });
}

/// Toggles a bulleted (`numbered == false`) or numbered list.
pub fn toggle_list(editor: HWND, numbered: bool) {
    let wanted = if numbered { PFN_ARABIC } else { PFN_BULLET };
    let current = editor::para_format(editor).w_numbering;
    let on = current != wanted;
    let format = ParaFormat2 {
        dw_mask: PFM_NUMBERING
            | PFM_NUMBERINGSTYLE
            | PFM_NUMBERINGSTART
            | PFM_NUMBERINGTAB
            | PFM_STARTINDENT
            | PFM_OFFSET,
        w_numbering: if on { wanted } else { 0 },
        w_numbering_style: if on && numbered { PFNS_PERIOD } else { 0 },
        w_numbering_start: 1,
        w_numbering_tab: if on { 360 } else { 0 },
        dx_start_indent: if on { 180 } else { 0 },
        dx_offset: if on { 360 } else { 0 },
        ..ParaFormat2::default()
    };
    editor::set_para_format(editor, &format);
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
