//! Rich Edit (`RICHEDIT50W`, msftedit.dll) wrapper.
//!
//! `windows-sys` does not bind `richedit.h`, so the Rich Edit message numbers,
//! `CHARFORMAT2W`, `CHARRANGE`, `EDITSTREAM` and `GETTEXTEX` are declared here
//! with the values from the Windows SDK.

use std::mem::size_of;
use std::ptr;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::EM_SETMARGINS;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SendMessageW, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_NOHIDESEL, ES_WANTRETURN,
    WM_COPY, WM_CUT, WM_PASTE, WS_CHILD, WS_HSCROLL, WS_TABSTOP, WS_VSCROLL,
};

use crate::app::App;
use crate::win;
use crate::wnd::menu;
use crate::wnd::status;

pub const CLASS_NAME: &str = "RICHEDIT50W";

const WM_USER: u32 = 0x0400;

pub const EM_GETCHARFORMAT: u32 = WM_USER + 58;
pub const EM_SETCHARFORMAT: u32 = WM_USER + 68;
pub const EM_SETEVENTMASK: u32 = WM_USER + 69;
pub const EM_SETTEXTMODE: u32 = WM_USER + 89;
pub const EM_EXGETSEL: u32 = WM_USER + 52;
pub const EM_EXLIMITTEXT: u32 = WM_USER + 53;
pub const EM_EXLINEFROMCHAR: u32 = WM_USER + 54;
pub const EM_EXSETSEL: u32 = WM_USER + 55;
pub const EM_SETUNDOLIMIT: u32 = WM_USER + 82;
pub const EM_REDO: u32 = WM_USER + 84;
pub const EM_STREAMIN: u32 = WM_USER + 73;
pub const EM_STREAMOUT: u32 = WM_USER + 74;
pub const EM_GETTEXTEX: u32 = WM_USER + 94;
pub const EM_SETTYPOGRAPHYOPTIONS: u32 = WM_USER + 202;
/// Rich Edit keeps the Edit-control value for undo (`EM_UNDO`); there is no
/// `WM_USER`-based variant, unlike [`EM_REDO`].
pub const EM_UNDO: u32 = 0x00C7;

/// Standard edit-control messages, also honoured by Rich Edit.
const EM_LINEINDEX_MSG: u32 = 0x00BB;
const EM_SETMODIFY: u32 = 0x00B9;
const EM_GETFIRSTVISIBLELINE: u32 = 0x00CE;
const EM_LINESCROLL: u32 = 0x00B6;
const EM_EMPTYUNDOBUFFER: u32 = 0x00CD;
const WM_GETTEXTLENGTH: u32 = 0x000E;
const WM_SETTEXT: u32 = 0x000C;

pub const SCF_SELECTION: usize = 0x0001;
pub const SCF_ALL: usize = 0x0004;
const SCF_DEFAULT: usize = 0x0000;

pub const CFM_BOLD: u32 = 0x0000_0001;
pub const CFM_ITALIC: u32 = 0x0000_0002;
pub const CFM_UNDERLINE: u32 = 0x0000_0004;
pub const CFM_STRIKEOUT: u32 = 0x0000_0008;
pub const CFM_FACE: u32 = 0x2000_0000;
pub const CFM_SIZE: u32 = 0x8000_0000;
pub const CFM_CHARSET: u32 = 0x0800_0000;
/// `CFM_HIDDEN` / `CFE_HIDDEN`: text that is not displayed or printed (used
/// for the editable `# ` heading marker).
pub const CFM_HIDDEN: u32 = 0x0000_0100;
pub const CFE_HIDDEN: u32 = 0x0000_0100;

pub const CFE_BOLD: u32 = 0x0000_0001;
pub const CFE_ITALIC: u32 = 0x0000_0002;
pub const CFE_UNDERLINE: u32 = 0x0000_0004;
pub const CFE_STRIKEOUT: u32 = 0x0000_0008;

pub const ENM_CHANGE: usize = 0x0000_0001;
pub const ENM_SELCHANGE: usize = 0x0008_0000;
pub const ENM_LINK: usize = 0x0400_0000;

/// `SF_RTF` for `EM_STREAMIN`/`EM_STREAMOUT`.
const SF_RTF: usize = 0x0002;

/// `CP_UTF16` used by `EM_GETTEXTEX` for the plain-text sidecar.
const CP_UTF16: u32 = 1200;

const TM_RICHTEXT: usize = 0x0002;
const TO_ADVANCEDTYPOGRAPHY: usize = 0x0001;
const EC_LEFTMARGIN: usize = 0x0001;
const EC_RIGHTMARGIN: usize = 0x0002;

/// `WM_COMMAND` notification code sent for `ENM_CHANGE`.
pub const EN_CHANGE_NOTIFY: u16 = 0x0300;
/// `WM_NOTIFY` code sent for `ENM_SELCHANGE`.
pub const EN_SELCHANGE_NOTIFY: u32 = 0x0702;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CharRange {
    pub cp_min: i32,
    pub cp_max: i32,
}

impl Default for CharRange {
    fn default() -> Self {
        // SAFETY: all-zero is a valid `CHARRANGE`.
        unsafe { std::mem::zeroed() }
    }
}

/// `CHARFORMAT2W` from `richedit.h` (116 bytes on both 32- and 64-bit).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CharFormat2W {
    pub cb_size: u32,
    pub dw_mask: u32,
    pub dw_effects: u32,
    pub y_height: i32,
    pub y_offset: i32,
    pub cr_text_color: u32,
    pub b_char_set: u8,
    pub b_pitch_and_family: u8,
    pub sz_face_name: [u16; 32],
    pub w_weight: u16,
    pub s_spacing: i16,
    pub cr_back_color: u32,
    pub lcid: u32,
    pub dw_cookie: u32,
    pub s_style: i16,
    pub w_kerning: u16,
    pub b_under_line_type: u8,
    pub b_animation: u8,
    pub b_rev_author: u8,
    pub b_underline_color: u8,
}

impl Default for CharFormat2W {
    fn default() -> Self {
        // SAFETY: all-zero is a valid `CHARFORMAT2W` (cbSize is set by callers).
        unsafe { std::mem::zeroed() }
    }
}

const _: () = assert!(size_of::<CharFormat2W>() == 116);

/// `EDITSTREAM` from `richedit.h`.
///
/// `richedit.h` wraps its structures in `#pragma pack(4)`, so on x64 the
/// callback pointer sits at offset 12, not 16. With natural alignment Rich Edit
/// reads a garbage callback address and `EM_STREAMOUT` never returns.
#[repr(C, packed(4))]
#[derive(Clone, Copy)]
pub struct EditStream {
    pub dw_cookie: usize,
    pub dw_error: u32,
    pub pfn_callback: Option<unsafe extern "system" fn(usize, *mut u8, i32, *mut i32) -> u32>,
}

const _: () = assert!(size_of::<EditStream>() == 20);

/// `GETTEXTEX` from `richedit.h` (also `pack(4)`).
#[repr(C, packed(4))]
struct GetTextEx {
    cb: u32,
    flags: u32,
    codepage: u32,
    lp_default_char: *const u8,
    lp_used_def_char: *mut i32,
}

const _: () = assert!(size_of::<GetTextEx>() == 28);

/// Reads from a byte slice for `EM_STREAMIN`.
struct StreamReader {
    data: *const u8,
    len: usize,
    pos: usize,
}

// SAFETY: `StreamReader` only lives on the thread that runs the synchronous
// `SendMessageW` it is passed to, so it is never shared across threads.
unsafe impl Send for StreamReader {}

unsafe extern "system" fn stream_in_callback(
    cookie: usize,
    buffer: *mut u8,
    count: i32,
    written: *mut i32,
) -> u32 {
    // SAFETY: `cookie` is the `StreamReader` passed in the `EDITSTREAM`.
    let reader = &mut *(cookie as *mut StreamReader);
    let want = count.max(0) as usize;
    let remaining = reader.len.saturating_sub(reader.pos);
    let take = want.min(remaining);
    if take > 0 {
        ptr::copy_nonoverlapping(reader.data.add(reader.pos), buffer, take);
        reader.pos += take;
    }
    *written = take as i32;
    0
}

unsafe extern "system" fn stream_out_callback(
    cookie: usize,
    buffer: *mut u8,
    count: i32,
    written: *mut i32,
) -> u32 {
    // SAFETY: `cookie` is the `Vec<u8>` passed in the `EDITSTREAM`.
    let sink = &mut *(cookie as *mut Vec<u8>);
    let length = count.max(0) as usize;
    if length > 0 {
        sink.extend_from_slice(std::slice::from_raw_parts(buffer, length));
    }
    *written = count;
    0
}

/// Creates a Rich Edit child for a new tab. Returns `None` if Windows refused
/// to create the window (e.g. msftedit.dll failed to load).
pub fn create(app: &App) -> Option<HWND> {
    let class = win::wide(CLASS_NAME);
    let style = WS_CHILD
        | WS_TABSTOP
        | WS_VSCROLL
        | WS_HSCROLL
        | (ES_MULTILINE | ES_AUTOVSCROLL | ES_AUTOHSCROLL | ES_NOHIDESEL | ES_WANTRETURN) as u32;

    // SAFETY: `class` outlives the call; all handles belong to `app`.
    let window = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::CreateWindowExW(
            0,
            class.as_ptr(),
            ptr::null(),
            style,
            0,
            0,
            0,
            0,
            app.hwnd,
            ptr::null_mut(),
            app.hinstance,
            ptr::null(),
        )
    };
    if window.is_null() {
        return None;
    }

    // SAFETY: `window` is a freshly created Rich Edit control.
    unsafe {
        SendMessageW(window, EM_SETTEXTMODE, TM_RICHTEXT, 0);
        SendMessageW(window, EM_SETUNDOLIMIT, 100, 0);
        SendMessageW(window, EM_SETTYPOGRAPHYOPTIONS, TO_ADVANCEDTYPOGRAPHY, 0);
        SendMessageW(window, EM_EXLIMITTEXT, 0, 0x7FFF_FFFF);
        SendMessageW(
            window,
            EM_SETEVENTMASK,
            0,
            (ENM_SELCHANGE | ENM_LINK) as isize,
        );
        apply_default_format(window);

        let margin = app.scale(6);
        SendMessageW(
            window,
            EM_SETMARGINS,
            EC_LEFTMARGIN | EC_RIGHTMARGIN,
            ((margin as isize) << 16) | margin as isize,
        );
        SendMessageW(window, EM_SETMODIFY, 0, 0);
    }
    crate::wnd::editkeys::install(window, app.hwnd);
    crate::wnd::link::enable_auto_detect(window);
    set_word_wrap(window, app.word_wrap);
    if app.zoom != 100 {
        set_zoom_percent(window, app.zoom);
    }

    Some(window)
}

fn apply_default_format(window: HWND) {
    let mut format = CharFormat2W {
        cb_size: size_of::<CharFormat2W>() as u32,
        dw_mask: CFM_FACE | CFM_SIZE | CFM_CHARSET,
        y_height: 220, // 11 pt, in twips
        b_char_set: 1, // DEFAULT_CHARSET
        ..CharFormat2W::default()
    };
    let face: Vec<u16> = "Segoe UI".encode_utf16().collect();
    for (slot, unit) in format.sz_face_name.iter_mut().zip(face) {
        *slot = unit;
    }
    format.sz_face_name[31] = 0;

    // SAFETY: the format lives on the stack for the duration of the call.
    unsafe {
        SendMessageW(
            window,
            EM_SETCHARFORMAT,
            SCF_ALL | SCF_DEFAULT,
            &format as *const CharFormat2W as isize,
        );
    }
}

/// Enables the `EN_CHANGE` notification. It stays disabled while the initial
/// RTF is streamed in (streaming fires `EN_CHANGE` too), so a freshly opened
/// note is never marked dirty by its own load.
pub fn set_change_events(editor: HWND, enabled: bool) {
    let mask = if enabled {
        ENM_CHANGE | ENM_SELCHANGE | ENM_LINK
    } else {
        ENM_SELCHANGE | ENM_LINK
    };
    // SAFETY: `editor` is a live Rich Edit control.
    unsafe {
        SendMessageW(editor, EM_SETEVENTMASK, 0, mask as isize);
    }
}

/// Clears the editor (a missing note file is an empty note).
pub fn clear(editor: HWND) {
    let empty = win::wide("");
    // SAFETY: `empty` is NUL terminated; `editor` is our control.
    unsafe {
        SendMessageW(editor, WM_SETTEXT, 0, empty.as_ptr() as isize);
        SendMessageW(editor, EM_SETMODIFY, 0, 0);
        SendMessageW(editor, EM_EMPTYUNDOBUFFER, 0, 0);
    }
}

/// Streams RTF bytes into the editor. Returns false when Rich Edit reported a
/// parse error (the caller then offers the `.prev` generation).
pub fn load_rtf(editor: HWND, bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        clear(editor);
        return true;
    }
    let ok = stream_in(editor, bytes, SF_RTF);

    // SAFETY: the control is alive and owned by us.
    unsafe {
        SendMessageW(editor, EM_SETMODIFY, 0, 0);
        SendMessageW(editor, EM_EMPTYUNDOBUFFER, 0, 0);
    }
    ok
}

/// `EM_STREAMIN` from a byte slice with the given `SF_*` flags.
fn stream_in(editor: HWND, bytes: &[u8], flags: usize) -> bool {
    let mut reader = StreamReader {
        data: bytes.as_ptr(),
        len: bytes.len(),
        pos: 0,
    };
    let mut stream = EditStream {
        dw_cookie: &mut reader as *mut StreamReader as usize,
        dw_error: 0,
        pfn_callback: Some(stream_in_callback),
    };

    // SAFETY: `stream` and `reader` outlive the synchronous call; the callback
    // only touches them from this thread.
    let inserted = unsafe {
        SendMessageW(
            editor,
            EM_STREAMIN,
            flags,
            &mut stream as *mut EditStream as isize,
        )
    };
    let error = stream.dw_error;
    // An empty but well-formed document inserts zero characters.
    error == 0 && (inserted != 0 || bytes.starts_with(b"{\\rtf"))
}

/// Streams the whole document out as RTF. `None` when Rich Edit failed.
pub fn stream_out_rtf(editor: HWND) -> Option<Vec<u8>> {
    stream_out(editor, SF_RTF)
}

/// `EM_STREAMOUT` into a byte vector with the given `SF_*` flags.
fn stream_out(editor: HWND, flags: usize) -> Option<Vec<u8>> {
    let mut sink: Vec<u8> = Vec::new();
    let mut stream = EditStream {
        dw_cookie: &mut sink as *mut Vec<u8> as usize,
        dw_error: 0,
        pfn_callback: Some(stream_out_callback),
    };
    // SAFETY: `stream` and `sink` outlive the synchronous call.
    let written = unsafe {
        SendMessageW(
            editor,
            EM_STREAMOUT,
            flags,
            &mut stream as *mut EditStream as isize,
        )
    };
    let error = stream.dw_error;
    if written != 0 && error == 0 {
        Some(sink)
    } else {
        None
    }
}

/// Plain text (without RTF markup) for the search sidecar.
pub fn plain_text(editor: HWND) -> String {
    // SAFETY: `editor` is a live Rich Edit control.
    let length = unsafe { SendMessageW(editor, WM_GETTEXTLENGTH, 0, 0) } as usize;
    let capacity = length + 2;
    let mut buffer = vec![0u8; capacity * 2];
    let mut request = GetTextEx {
        cb: (capacity * 2) as u32,
        flags: 0,
        codepage: CP_UTF16,
        lp_default_char: ptr::null(),
        lp_used_def_char: ptr::null_mut(),
    };

    // SAFETY: `request` and `buffer` are valid for the synchronous call.
    unsafe {
        SendMessageW(
            editor,
            EM_GETTEXTEX,
            &mut request as *mut GetTextEx as usize,
            buffer.as_mut_ptr() as isize,
        );
    }

    let units: Vec<u16> = buffer
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// Character format of the current selection (`dw_mask` says which fields are
/// uniform across it).
pub fn selection_format(editor: HWND) -> CharFormat2W {
    let mut format = CharFormat2W {
        cb_size: size_of::<CharFormat2W>() as u32,
        ..CharFormat2W::default()
    };
    // SAFETY: `format` is a valid `CHARFORMAT2W` for the call.
    unsafe {
        SendMessageW(
            editor,
            EM_GETCHARFORMAT,
            SCF_SELECTION,
            &mut format as *mut CharFormat2W as isize,
        );
    }
    format
}

/// Applies `format` (only the fields named in its `dw_mask`) to the selection.
pub fn set_selection_format(editor: HWND, format: &CharFormat2W) {
    let mut format = *format;
    format.cb_size = size_of::<CharFormat2W>() as u32;
    // SAFETY: `format` is a valid `CHARFORMAT2W` for the call.
    unsafe {
        SendMessageW(
            editor,
            EM_SETCHARFORMAT,
            SCF_SELECTION,
            &format as *const CharFormat2W as isize,
        );
    }
}

/// `PARAFORMAT2` from `richedit.h` (188 bytes; every field is naturally
/// aligned, so `pack(4)` changes nothing here).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ParaFormat2 {
    pub cb_size: u32,
    pub dw_mask: u32,
    pub w_numbering: u16,
    pub w_effects: u16,
    pub dx_start_indent: i32,
    pub dx_right_indent: i32,
    pub dx_offset: i32,
    pub w_alignment: u16,
    pub c_tab_count: i16,
    pub rgx_tabs: [i32; 32],
    pub dy_space_before: i32,
    pub dy_space_after: i32,
    pub dy_line_spacing: i32,
    pub s_style: i16,
    pub b_line_spacing_rule: u8,
    pub b_outline_level: u8,
    pub w_shading_weight: u16,
    pub w_shading_style: u16,
    pub w_numbering_start: u16,
    pub w_numbering_style: u16,
    pub w_numbering_tab: u16,
    pub w_border_space: u16,
    pub w_border_width: u16,
    pub w_borders: u16,
}

const _: () = assert!(size_of::<ParaFormat2>() == 188);

impl Default for ParaFormat2 {
    fn default() -> Self {
        // SAFETY: all-zero is a valid `PARAFORMAT2`; callers set the mask.
        let mut format: Self = unsafe { std::mem::zeroed() };
        format.cb_size = size_of::<Self>() as u32;
        format
    }
}

pub const EM_GETPARAFORMAT: u32 = WM_USER + 61;
pub const EM_SETPARAFORMAT: u32 = WM_USER + 71;
pub const PFM_STARTINDENT: u32 = 0x0000_0001;
pub const PFM_OFFSET: u32 = 0x0000_0004;
pub const PFM_ALIGNMENT: u32 = 0x0000_0008;
pub const PFM_TABSTOPS: u32 = 0x0000_0010;
pub const PFM_NUMBERING: u32 = 0x0000_0020;
pub const PFM_SPACEBEFORE: u32 = 0x0000_0040;
pub const PFM_SPACEAFTER: u32 = 0x0000_0080;
pub const PFM_NUMBERINGSTYLE: u32 = 0x0000_2000;
pub const PFM_NUMBERINGTAB: u32 = 0x0000_4000;
pub const PFM_NUMBERINGSTART: u32 = 0x0000_8000;
/// `wEffects` bit for "paragraph is inside a table row".
pub const PFE_TABLE: u16 = 0x4000;

/// Paragraph format at the selection.
pub fn para_format(editor: HWND) -> ParaFormat2 {
    let mut format = ParaFormat2::default();
    // SAFETY: `format` is a valid `PARAFORMAT2` for the call.
    unsafe {
        SendMessageW(
            editor,
            EM_GETPARAFORMAT,
            0,
            &mut format as *mut ParaFormat2 as isize,
        );
    }
    format
}

/// Applies the fields named in `format.dw_mask` to the selected paragraphs.
pub fn set_para_format(editor: HWND, format: &ParaFormat2) {
    let mut format = *format;
    format.cb_size = size_of::<ParaFormat2>() as u32;
    // SAFETY: `format` is a valid `PARAFORMAT2` for the call.
    unsafe {
        SendMessageW(
            editor,
            EM_SETPARAFORMAT,
            0,
            &format as *const ParaFormat2 as isize,
        );
    }
}

pub fn in_table(editor: HWND) -> bool {
    para_format(editor).w_effects & PFE_TABLE != 0
}

const EM_GETTEXTLENGTHEX: u32 = WM_USER + 95;
const EM_REPLACESEL: u32 = 0x00C2;
/// `GETTEXTEX.flags`: text exactly as stored, including table delimiters
/// (U+FFF9/U+FFFB/U+0007) and object markers (U+FFFC), so indexes match
/// Rich Edit character positions. `EM_GETTEXTRANGE` drops the table marks
/// and shifts every position after a table.
const GT_RAWTEXT: u32 = 4;

/// Whole document as stored in memory (see [`GT_RAWTEXT`]).
pub fn raw_text(editor: HWND) -> Vec<u16> {
    #[repr(C)]
    struct GetTextLengthEx {
        flags: u32,
        codepage: u32,
    }
    const GTL_PRECISE: u32 = 2;
    const GTL_NUMCHARS: u32 = 8;
    let request = GetTextLengthEx {
        flags: GTL_PRECISE | GTL_NUMCHARS,
        codepage: CP_UTF16,
    };
    // SAFETY: `request` is valid for the call.
    let estimate = unsafe {
        SendMessageW(
            editor,
            EM_GETTEXTLENGTHEX,
            &request as *const GetTextLengthEx as usize,
            0,
        )
    }
    .max(0) as usize;
    // Raw text can be longer than the plain length (table marks): grow until
    // the whole story fits.
    let mut capacity = estimate * 2 + 64;
    loop {
        let mut buffer = vec![0u16; capacity];
        let mut query = GetTextEx {
            cb: (capacity * 2) as u32,
            flags: GT_RAWTEXT,
            codepage: CP_UTF16,
            lp_default_char: ptr::null(),
            lp_used_def_char: ptr::null_mut(),
        };
        // SAFETY: the buffer holds `capacity` UTF-16 units.
        let copied = unsafe {
            SendMessageW(
                editor,
                EM_GETTEXTEX,
                &mut query as *mut GetTextEx as usize,
                buffer.as_mut_ptr() as isize,
            )
        }
        .max(0) as usize;
        if copied + 1 < capacity || capacity > 64 * 1024 * 1024 {
            buffer.truncate(copied);
            return buffer;
        }
        capacity *= 2;
    }
}

/// Number of characters in the document (paragraph marks count as one).
pub fn text_length(editor: HWND) -> i32 {
    raw_text(editor).len() as i32
}

/// UTF-16 text of `[from, to)` with positions that match the selection.
pub fn text_units(editor: HWND, from: i32, to: i32) -> Vec<u16> {
    let raw = raw_text(editor);
    let from = from.clamp(0, raw.len() as i32) as usize;
    let to = to.clamp(0, raw.len() as i32) as usize;
    if to <= from {
        return Vec::new();
    }
    raw[from..to].to_vec()
}

pub fn text_range(editor: HWND, from: i32, to: i32) -> String {
    String::from_utf16_lossy(&text_units(editor, from, to))
}

/// Start of the paragraph that contains `pos`.
pub fn paragraph_start(editor: HWND, pos: i32) -> i32 {
    let raw = raw_text(editor);
    let end = (pos.max(0) as usize).min(raw.len());
    raw[..end]
        .iter()
        .rposition(|unit| is_paragraph_break(*unit))
        .map_or(0, |index| index as i32 + 1)
}

/// Position of the paragraph mark that ends the paragraph containing `pos`
/// (or the document length when there is none).
pub fn paragraph_end(editor: HWND, pos: i32) -> i32 {
    let raw = raw_text(editor);
    let start = (pos.max(0) as usize).min(raw.len());
    raw[start..]
        .iter()
        .position(|unit| is_paragraph_break(*unit))
        .map_or(raw.len() as i32, |index| (start + index) as i32)
}

/// `\r`, the table cell mark and the table row delimiters all end a paragraph.
fn is_paragraph_break(unit: u16) -> bool {
    matches!(unit, 0x000D | 0x0007 | 0xFFF9 | 0xFFFB)
}

/// Replaces the selection with plain text (inherits the formatting at the
/// selection). `undoable` adds the change to the undo stack.
pub fn replace_selection(editor: HWND, text: &str, undoable: bool) {
    let wide = win::wide(text);
    // SAFETY: `wide` is NUL terminated and outlives the call.
    unsafe {
        SendMessageW(
            editor,
            EM_REPLACESEL,
            usize::from(undoable),
            wide.as_ptr() as isize,
        );
    }
}

/// `SFF_SELECTION`: stream only the selection.
const SFF_SELECTION: usize = 0x8000;

/// RTF of the current selection.
pub fn selection_rtf(editor: HWND) -> Option<Vec<u8>> {
    stream_out(editor, SF_RTF | SFF_SELECTION)
}

/// Replaces the selection with RTF (keeps formatting, pictures, fields).
pub fn insert_rtf(editor: HWND, rtf: &[u8]) -> bool {
    stream_in(editor, rtf, SF_RTF | SFF_SELECTION)
}

/// Number of pictures in the document. Rich Edit reports a picture as a plain
/// space in its text, so they are counted in the RTF instead.
pub fn picture_count(editor: HWND) -> usize {
    stream_out_rtf(editor)
        .map(|rtf| count_pictures(&rtf))
        .unwrap_or(0)
}

/// Number of `{\pict` groups in an RTF byte string.
pub fn count_pictures(rtf: &[u8]) -> usize {
    rtf.windows(5).filter(|window| window == b"\\pict").count()
}

/// Current selection as `(from, to)` character offsets.
pub fn selection(editor: HWND) -> (i32, i32) {
    let mut range = CharRange::default();
    // SAFETY: `range` is a valid `CHARRANGE` for the call.
    unsafe {
        SendMessageW(
            editor,
            EM_EXGETSEL,
            0,
            &mut range as *mut CharRange as isize,
        );
    }
    (range.cp_min, range.cp_max)
}

/// Restores a selection/caret position.
pub fn set_selection(editor: HWND, from: i32, to: i32) {
    let range = CharRange {
        cp_min: from,
        cp_max: to,
    };
    // SAFETY: `range` is a valid `CHARRANGE` for the call.
    unsafe {
        SendMessageW(editor, EM_EXSETSEL, 0, &range as *const CharRange as isize);
    }
}

/// Scrolls the caret into view.
pub fn scroll_caret(editor: HWND) {
    // SAFETY: standard edit message on our control.
    unsafe {
        SendMessageW(editor, 0x00B7, 0, 0); // EM_SCROLLCARET
    }
}

/// First visible line (used for the session file).
pub fn first_visible_line(editor: HWND) -> u32 {
    // SAFETY: standard edit message on our control.
    let line = unsafe { SendMessageW(editor, EM_GETFIRSTVISIBLELINE, 0, 0) };
    line.max(0) as u32
}

/// Scrolls so that `line` becomes the first visible line.
pub fn scroll_to_line(editor: HWND, line: u32) {
    let current = first_visible_line(editor) as i32;
    let delta = line as i32 - current;
    if delta == 0 {
        return;
    }
    // SAFETY: standard edit message on our control.
    unsafe {
        SendMessageW(editor, EM_LINESCROLL, 0, delta as isize);
    }
}

/// Clears the modified flag (a streamed-in document is not dirty).
pub fn set_modified(editor: HWND, modified: bool) {
    // SAFETY: standard edit message on our control.
    unsafe {
        SendMessageW(editor, EM_SETMODIFY, modified as usize, 0);
    }
}

pub fn line_col(editor: HWND) -> (i32, i32) {
    let (caret, _) = selection(editor);
    // SAFETY: `editor` is a live Rich Edit control.
    unsafe {
        let line = SendMessageW(editor, EM_EXLINEFROMCHAR, 0, caret as isize) as i32;
        let line = line.max(0);
        let line_start = SendMessageW(editor, EM_LINEINDEX_MSG, line as usize, 0) as i32;
        let column = caret - line_start.max(0) + 1;
        (line + 1, column.max(1))
    }
}

/// Runs an Edit-menu command against the active editor.
pub fn command(app: &mut App, id: u16) {
    let Some(editor) = app.active_editor() else {
        status::set_message(app, "No note open");
        return;
    };

    // SAFETY: `editor` is a live Rich Edit control owned by this process.
    unsafe {
        match id {
            menu::IDM_EDIT_UNDO => {
                // A `- ` list conversion is undone as one step (back to the
                // typed text) instead of Rich Edit's separate edits.
                if !crate::wnd::editkeys::undo_marker_conversion(editor) {
                    SendMessageW(editor, EM_UNDO, 0, 0);
                }
            }
            menu::IDM_EDIT_REDO => {
                SendMessageW(editor, EM_REDO, 0, 0);
            }
            menu::IDM_EDIT_CUT => {
                SendMessageW(editor, WM_CUT, 0, 0);
            }
            menu::IDM_EDIT_COPY => {
                SendMessageW(editor, WM_COPY, 0, 0);
            }
            menu::IDM_EDIT_PASTE => {
                // Goes through the subclass, which also handles pictures.
                SendMessageW(editor, WM_PASTE, 0, 0);
            }
            menu::IDM_EDIT_PASTE_PLAIN => {
                if let Some(text) = crate::wnd::image::clipboard_text(app.hwnd) {
                    replace_selection(
                        editor,
                        &text.replace("\r\n", "\r").replace('\n', "\r"),
                        true,
                    );
                }
            }
            menu::IDM_EDIT_SELECT_ALL => {
                let range = CharRange {
                    cp_min: 0,
                    cp_max: -1,
                };
                SendMessageW(editor, EM_EXSETSEL, 0, &range as *const CharRange as isize);
            }
            _ => {}
        }
        SetFocus(editor);
    }

    status::update_caret(app);
    if let Some(index) = app.active {
        app.refresh_tab_ui(index);
    }
}

/// Edit > Duplicate Line (Ctrl+D): copies the current paragraph, formatting
/// included, below itself. With a selection, duplicates the selection.
pub fn duplicate_line(editor: HWND) {
    let (from, to) = selection(editor);
    if from != to {
        if let Some(rtf) = selection_rtf(editor) {
            set_selection(editor, to, to);
            insert_rtf(editor, &rtf);
            set_selection(editor, to, to + (to - from));
        }
        return;
    }
    let start = paragraph_start(editor, from);
    let end = paragraph_end(editor, from);
    let rtf = if end > start {
        set_selection(editor, start, end);
        selection_rtf(editor)
    } else {
        None
    };
    set_selection(editor, end, end);
    replace_selection(editor, "\r", true);
    if let Some(rtf) = rtf {
        insert_rtf(editor, &rtf);
    }
    let caret = end + 1 + (from - start);
    set_selection(editor, caret, caret);
}

const EM_SETTARGETDEVICE: u32 = WM_USER + 72;
const EM_GETZOOM: u32 = WM_USER + 224;
const EM_SETZOOM: u32 = WM_USER + 225;

/// View > Word Wrap. Off means long lines scroll horizontally (handy for code).
pub fn set_word_wrap(editor: HWND, wrap: bool) {
    // SAFETY: plain message to our Rich Edit (no target DC).
    unsafe {
        SendMessageW(editor, EM_SETTARGETDEVICE, 0, isize::from(!wrap));
    }
}

/// Current zoom as a percentage (100 = normal).
pub fn zoom_percent(editor: HWND) -> i32 {
    let (mut numerator, mut denominator) = (0i32, 0i32);
    // SAFETY: both out parameters live on the stack.
    unsafe {
        SendMessageW(
            editor,
            EM_GETZOOM,
            &mut numerator as *mut i32 as usize,
            &mut denominator as *mut i32 as isize,
        );
    }
    if numerator <= 0 || denominator <= 0 {
        100
    } else {
        numerator * 100 / denominator
    }
}

pub fn set_zoom_percent(editor: HWND, percent: i32) {
    let percent = percent.clamp(25, 500);
    // SAFETY: plain message to our Rich Edit; 0/0 resets the zoom.
    unsafe {
        if percent == 100 {
            SendMessageW(editor, EM_SETZOOM, 0, 0);
        } else {
            SendMessageW(editor, EM_SETZOOM, percent as usize, 100);
        }
    }
}

/// Toggles bold/italic/underline/strikethrough on the current selection (or on
/// the insertion point when there is no selection).
pub fn toggle_effect(app: &mut App, id: u16) {
    let (mask, effect) = match id {
        menu::IDM_FORMAT_BOLD => (CFM_BOLD, CFE_BOLD),
        menu::IDM_FORMAT_ITALIC => (CFM_ITALIC, CFE_ITALIC),
        menu::IDM_FORMAT_UNDERLINE => (CFM_UNDERLINE, CFE_UNDERLINE),
        menu::IDM_FORMAT_STRIKEOUT => (CFM_STRIKEOUT, CFE_STRIKEOUT),
        _ => return,
    };

    let Some(editor) = app.active_editor() else {
        status::set_message(app, "No note open");
        return;
    };

    let mut format = CharFormat2W {
        cb_size: size_of::<CharFormat2W>() as u32,
        dw_mask: mask,
        ..CharFormat2W::default()
    };

    // SAFETY: `format` is stack allocated; the control is our child window.
    unsafe {
        SendMessageW(
            editor,
            EM_GETCHARFORMAT,
            SCF_SELECTION,
            &mut format as *mut CharFormat2W as isize,
        );
        if format.dw_effects & effect != 0 {
            format.dw_effects &= !effect;
        } else {
            format.dw_effects |= effect;
        }
        format.dw_mask = mask;
        SendMessageW(
            editor,
            EM_SETCHARFORMAT,
            SCF_SELECTION,
            &format as *const CharFormat2W as isize,
        );
        SetFocus(editor);
    }

    app.mark_dirty(editor);
    crate::wnd::toolbar::sync_format(app);
}
