//! Toolbars. The top bar holds the file/edit commands; the format bar below it
//! is Word-like: a style combobox (Normal, Heading 1–4), a font-name combobox,
//! a font-size combobox, text/highlight colour palettes, alignment, lists and
//! the insert commands. Tooltips name the shortcut of every button.

use std::collections::BTreeSet;
use std::mem::size_of;
use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, EnumFontFamiliesExW, GetDC, GetTextExtentPoint32W, ReleaseDC, SelectObject,
    DEFAULT_CHARSET, LOGFONTW, RASTER_FONTTYPE, TEXTMETRICW,
};
use windows_sys::Win32::UI::Controls::{
    BTNS_AUTOSIZE, BTNS_BUTTON, BTNS_CHECK, BTNS_DROPDOWN, BTNS_SEP, BTNS_SHOWTEXT, CCS_NODIVIDER,
    CCS_NOPARENTALIGN, CCS_NORESIZE, IDB_STD_SMALL_COLOR, I_IMAGENONE, NMTTDISPINFOW, STD_COPY,
    STD_CUT, STD_FILENEW, STD_FILESAVE, STD_FIND, STD_PASTE, STD_REDOW, STD_UNDO, TBBUTTON,
    TBSTATE_CHECKED, TBSTATE_ENABLED, TBSTYLE_EX_DRAWDDARROWS, TBSTYLE_EX_MIXEDBUTTONS,
    TBSTYLE_FLAT, TBSTYLE_LIST, TBSTYLE_TOOLTIPS, TB_ADDBUTTONSW, TB_ADDSTRINGW, TB_AUTOSIZE,
    TB_BUTTONSTRUCTSIZE, TB_GETRECT, TB_LOADIMAGES, TB_SETEXTENDEDSTYLE, TB_SETSTATE,
    TOOLBARCLASSNAMEW,
};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DestroyMenu, GetCursorPos, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, SendMessageW, SetWindowPos, TrackPopupMenu, HMENU,
    MF_SEPARATOR, MF_STRING, SWP_NOACTIVATE, SWP_NOZORDER, TPM_LEFTALIGN, TPM_RETURNCMD,
    TPM_TOPALIGN, WS_CHILD, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::app::App;
use crate::win;
use crate::wnd::menu::*;
use crate::wnd::{editor, format};

/// `HINST_COMMCTRL`, absent from windows-sys.
const HINST_COMMCTRL: isize = -1;

/// `TTN_GETDISPINFOW` (`TTN_FIRST - 10`).
pub const TTN_GETDISPINFOW: u32 = 0u32.wrapping_sub(530);

/// The separator that reserves room for the three comboboxes.
const COMBO_SEP_ID: u16 = 1999;
/// Width reserved for the combos, in 96-DPI pixels (style + font + size).
const COMBO_FONT_WIDTH: i32 = 118;
const COMBO_SIZE_WIDTH: i32 = 50;
const COMBO_GAP: i32 = 6;
const COMBO_EDGE: i32 = 2;

/// Internal palette command ids (never reach `menu::on_command`).
const PALETTE_BASE: u16 = 3200;
const PALETTE_MORE: u16 = 3300;
const PALETTE_NONE: u16 = 3301;

/// Control ids of the three format comboboxes. They are children of the format
/// bar, so their `WM_COMMAND` is forwarded to the frame by a subclass shim.
pub const STYLE_COMBO_ID: u16 = 1200;
pub const FONT_COMBO_ID: u16 = 1201;
pub const SIZE_COMBO_ID: u16 = 1202;

/// Colours offered by the text-colour palette.
const TEXT_COLOURS: [(&str, u32); 10] = [
    ("Black", 0x0000_0000),
    ("Dark grey", 0x0044_4444),
    ("Red", 0x0025_2525),
    ("Orange", 0x002A_6BEC),
    ("Yellow", 0x0000_C8F0),
    ("Green", 0x0034_8A2D),
    ("Teal", 0x0090_7F2D),
    ("Blue", 0x00C0_5A2D),
    ("Purple", 0x00A0_3D80),
    ("Pink", 0x0060_5DE8),
];

/// Colours offered by the highlight palette.
const HIGHLIGHT_COLOURS: [(&str, u32); 10] = [
    ("Yellow", 0x0070_EEFF),
    ("Green", 0x0070_EEC8),
    ("Cyan", 0x00F0_ECE0),
    ("Blue", 0x00FF_D3C8),
    ("Pink", 0x00E8_C8FF),
    ("Red", 0x00C8_C8FF),
    ("Orange", 0x0060_D0FF),
    ("Purple", 0x00F0_C8E0),
    ("Grey", 0x00E0_E0E0),
    ("White", 0x00FF_FFFF),
];

/// Tooltip text for a toolbar command.
fn tip(command: u16) -> &'static str {
    match command {
        IDM_FILE_NEW => "New note (Ctrl+N)",
        IDM_FILE_SAVE => "Save now (Ctrl+S)",
        IDM_EDIT_CUT => "Cut (Ctrl+X)",
        IDM_EDIT_COPY => "Copy (Ctrl+C)",
        IDM_EDIT_PASTE => "Paste (Ctrl+V)",
        IDM_EDIT_UNDO => "Undo (Ctrl+Z)",
        IDM_EDIT_REDO => "Redo (Ctrl+Y)",
        IDM_FORMAT_BOLD => "Bold (Ctrl+B)",
        IDM_FORMAT_ITALIC => "Italic (Ctrl+I)",
        IDM_FORMAT_UNDERLINE => "Underline (Ctrl+U)",
        IDM_FORMAT_STRIKEOUT => "Strikethrough (Alt+Shift+5)",
        IDM_FORMAT_H1 => "Heading 1 (Ctrl+Alt+1)",
        IDM_FORMAT_H2 => "Heading 2 (Ctrl+Alt+2)",
        IDM_FORMAT_BULLETS => "Bulleted list (Ctrl+Shift+8)",
        IDM_FORMAT_NUMBERS => "Numbered list (Ctrl+Shift+7)",
        IDM_TOOLBAR_COLOR => "Text colour — pick from the palette",
        IDM_TOOLBAR_HIGHLIGHT => "Highlight — pick from the palette (Ctrl+Shift+H = yellow)",
        IDM_FORMAT_LEFT => "Align left (Ctrl+L)",
        IDM_FORMAT_CENTER => "Align center (Ctrl+E)",
        IDM_FORMAT_RIGHT => "Align right (Ctrl+R)",
        IDM_FORMAT_JUSTIFY => "Justify (Ctrl+J)",
        IDM_VIEW_WORD_WRAP => "Word wrap (View > Word Wrap)",
        IDM_INSERT_IMAGE => "Insert image (Ctrl+Shift+I) — or paste a screenshot",
        IDM_INSERT_TABLE => "Insert table (Ctrl+Shift+T)",
        IDM_INSERT_LINK => "Insert link (Ctrl+K)",
        IDM_INSERT_CODE_BLOCK => "Code block (Ctrl+Shift+C)",
        IDM_INSERT_FORMULA => "Formula (Alt+=)",
        IDM_SEARCH_FIND => "Find in note (Ctrl+F)",
        _ => "",
    }
}

/// Fills an `NMTTDISPINFOW` for a toolbar button.
///
/// # Safety
/// `lparam` must be the `NMTTDISPINFOW` pointer of a `TTN_GETDISPINFOW`.
pub unsafe fn tooltip(lparam: isize) {
    let info = &mut *(lparam as *mut NMTTDISPINFOW);
    let text = tip(info.hdr.idFrom as u16);
    info.szText = [0; 80];
    for (slot, unit) in info.szText.iter_mut().zip(text.encode_utf16().take(79)) {
        *slot = unit;
    }
    info.lpszText = info.szText.as_mut_ptr();
}

// --------------------------------------------------------------- construction

struct Buttons {
    list: Vec<TBBUTTON>,
    /// First index of the bar's text strings in the toolbar string pool.
    first_text: i32,
}

impl Buttons {
    fn icon(&mut self, bitmap: u32, command: u16) {
        self.list.push(TBBUTTON {
            iBitmap: bitmap as i32,
            idCommand: command as i32,
            fsState: TBSTATE_ENABLED as u8,
            fsStyle: (BTNS_BUTTON | BTNS_AUTOSIZE) as u8,
            bReserved: [0; 6],
            dwData: 0,
            iString: 0,
        });
    }

    fn text(&mut self, index: i32, command: u16, style: u32, checked: bool) {
        let state = TBSTATE_ENABLED | if checked { TBSTATE_CHECKED } else { 0 };
        self.list.push(TBBUTTON {
            iBitmap: I_IMAGENONE,
            idCommand: command as i32,
            fsState: state as u8,
            fsStyle: (BTNS_BUTTON | BTNS_AUTOSIZE | BTNS_SHOWTEXT | style) as u8,
            bReserved: [0; 6],
            dwData: 0,
            iString: (self.first_text + index) as isize,
        });
    }

    /// A separator; `width` sets the reserved space in pixels.
    fn separator(&mut self, id: u16, width: i32) {
        self.list.push(TBBUTTON {
            iBitmap: width,
            idCommand: id as i32,
            fsState: 0,
            fsStyle: BTNS_SEP as u8,
            bReserved: [0; 6],
            dwData: 0,
            iString: 0,
        });
    }
}

/// Creates one toolbar window with the given labels and buttons.
fn create_bar(app: &App, labels: &[&str], fill: impl FnOnce(&mut Buttons)) -> HWND {
    let style = WS_CHILD
        | WS_VISIBLE
        | TBSTYLE_FLAT
        | TBSTYLE_LIST
        | TBSTYLE_TOOLTIPS
        | (CCS_NODIVIDER | CCS_NOPARENTALIGN | CCS_NORESIZE) as u32;

    // SAFETY: constant class name and parent handles owned by `app`.
    let bar = unsafe {
        CreateWindowExW(
            0,
            TOOLBARCLASSNAMEW,
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
    if bar.is_null() {
        return bar;
    }

    // SAFETY: `bar` is a live toolbar control; every message below is a
    // documented toolbar message with stack/static arguments.
    unsafe {
        SendMessageW(bar, TB_BUTTONSTRUCTSIZE, size_of::<TBBUTTON>(), 0);
        SendMessageW(
            bar,
            TB_SETEXTENDEDSTYLE,
            0,
            (TBSTYLE_EX_MIXEDBUTTONS | TBSTYLE_EX_DRAWDDARROWS) as isize,
        );
        SendMessageW(
            bar,
            TB_LOADIMAGES,
            IDB_STD_SMALL_COLOR as usize,
            HINST_COMMCTRL,
        );

        let mut strings = Vec::new();
        for label in labels {
            strings.extend(label.encode_utf16());
            strings.push(0);
        }
        strings.push(0);
        let first_text = SendMessageW(bar, TB_ADDSTRINGW, 0, strings.as_ptr() as isize) as i32;

        let mut buttons = Buttons {
            list: Vec::new(),
            first_text,
        };
        fill(&mut buttons);
        SendMessageW(
            bar,
            TB_ADDBUTTONSW,
            buttons.list.len(),
            buttons.list.as_ptr() as isize,
        );
        SendMessageW(bar, TB_AUTOSIZE, 0, 0);
    }
    bar
}

/// Main toolbar: file/edit commands, word wrap and find.
fn create_main(app: &App) -> HWND {
    const LABELS: [&str; 1] = ["Wrap"];
    create_bar(app, &LABELS, |buttons| {
        buttons.icon(STD_FILENEW, IDM_FILE_NEW);
        buttons.icon(STD_FILESAVE, IDM_FILE_SAVE);
        buttons.separator(0, 0);
        buttons.icon(STD_CUT, IDM_EDIT_CUT);
        buttons.icon(STD_COPY, IDM_EDIT_COPY);
        buttons.icon(STD_PASTE, IDM_EDIT_PASTE);
        buttons.separator(0, 0);
        buttons.icon(STD_UNDO, IDM_EDIT_UNDO);
        buttons.icon(STD_REDOW, IDM_EDIT_REDO);
        buttons.separator(0, 0);
        buttons.text(0, IDM_VIEW_WORD_WRAP, BTNS_CHECK, app.word_wrap);
        buttons.separator(0, 0);
        buttons.icon(STD_FIND, IDM_SEARCH_FIND);
    })
}

/// Format bar: style/font/size combos, colour palettes, alignment, lists and
/// the insert commands.
fn create_format(app: &App) -> HWND {
    const LABELS: [&str; 17] = [
        "B",
        "I",
        "U",
        "S",
        "Colour",
        "Highlight",
        "Left",
        "Center",
        "Right",
        "Justify",
        "\u{2022}",
        "1.",
        "Img",
        "Table",
        "Link",
        "Code",
        "f(x)",
    ];
    create_bar(app, &LABELS, |buttons| {
        buttons.separator(COMBO_SEP_ID, app.scale(combo_area(app)));
        buttons.text(0, IDM_FORMAT_BOLD, 0, false);
        buttons.text(1, IDM_FORMAT_ITALIC, 0, false);
        buttons.text(2, IDM_FORMAT_UNDERLINE, 0, false);
        buttons.text(3, IDM_FORMAT_STRIKEOUT, 0, false);
        buttons.separator(0, 0);
        buttons.text(4, IDM_TOOLBAR_COLOR, BTNS_DROPDOWN, false);
        buttons.text(5, IDM_TOOLBAR_HIGHLIGHT, BTNS_DROPDOWN, false);
        buttons.separator(0, 0);
        buttons.text(6, IDM_FORMAT_LEFT, 0, false);
        buttons.text(7, IDM_FORMAT_CENTER, 0, false);
        buttons.text(8, IDM_FORMAT_RIGHT, 0, false);
        buttons.text(9, IDM_FORMAT_JUSTIFY, 0, false);
        buttons.separator(0, 0);
        buttons.text(10, IDM_FORMAT_BULLETS, 0, false);
        buttons.text(11, IDM_FORMAT_NUMBERS, 0, false);
        buttons.separator(0, 0);
        buttons.text(12, IDM_INSERT_IMAGE, 0, false);
        buttons.text(13, IDM_INSERT_TABLE, 0, false);
        buttons.text(14, IDM_INSERT_LINK, 0, false);
        buttons.text(15, IDM_INSERT_CODE_BLOCK, 0, false);
        buttons.text(16, IDM_INSERT_FORMULA, 0, false);
    })
}

fn create_combo(app: &App, parent: HWND, id: u16, style: u32) -> HWND {
    let class = win::wide("COMBOBOX");
    // SAFETY: constant class name, live parent, no pointer arguments.
    let combo = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            ptr::null(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | style,
            0,
            0,
            10,
            200,
            parent,
            id as isize as windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
            app.hinstance,
            ptr::null(),
        )
    };
    if !combo.is_null() && !app.ui_font.is_null() {
        // SAFETY: our own combo and font.
        unsafe {
            SendMessageW(
                combo,
                windows_sys::Win32::UI::WindowsAndMessaging::WM_SETFONT,
                app.ui_font as usize,
                1,
            );
        }
    }
    combo
}

fn combo_add(combo: HWND, text: &str) {
    let wide = win::wide(text);
    // SAFETY: `wide` outlives the call; `combo` is our control.
    unsafe {
        SendMessageW(
            combo,
            windows_sys::Win32::UI::WindowsAndMessaging::CB_ADDSTRING,
            0,
            wide.as_ptr() as isize,
        );
    }
}

fn combo_select_text(combo: HWND, text: &str) -> bool {
    let wide = win::wide(text);
    // SAFETY: `wide` outlives the call; `combo` is our control.
    let index = unsafe {
        SendMessageW(
            combo,
            windows_sys::Win32::UI::WindowsAndMessaging::CB_FINDSTRINGEXACT,
            0,
            wide.as_ptr() as isize,
        )
    };
    if index < 0 {
        return false;
    }
    // SAFETY: plain combo message.
    unsafe {
        SendMessageW(
            combo,
            windows_sys::Win32::UI::WindowsAndMessaging::CB_SETCURSEL,
            index as usize,
            0,
        );
    }
    true
}

fn create_combos(app: &mut App, bar: HWND) {
    // SAFETY: the combos are created as children of our format bar.
    let style_combo = create_combo(
        app,
        bar,
        STYLE_COMBO_ID,
        windows_sys::Win32::UI::WindowsAndMessaging::CBS_DROPDOWNLIST as u32
            | windows_sys::Win32::UI::WindowsAndMessaging::CBS_HASSTRINGS as u32,
    );
    // Font and size can be typed: the text is completed from the list and
    // applied on Enter (or when an item is picked from the open list), never
    // while typing.
    let typed = windows_sys::Win32::UI::WindowsAndMessaging::CBS_DROPDOWN as u32
        | windows_sys::Win32::UI::WindowsAndMessaging::CBS_AUTOHSCROLL as u32
        | windows_sys::Win32::UI::WindowsAndMessaging::CBS_HASSTRINGS as u32;
    let font_combo = create_combo(app, bar, FONT_COMBO_ID, typed);
    let size_combo = create_combo(app, bar, SIZE_COMBO_ID, typed);
    for combo in [font_combo, size_combo] {
        // SAFETY: the edit box is the combo's first child; the subclass
        // lives as long as the control.
        unsafe {
            let edit = windows_sys::Win32::UI::WindowsAndMessaging::GetWindow(
                combo,
                windows_sys::Win32::UI::WindowsAndMessaging::GW_CHILD,
            );
            if !edit.is_null() {
                SetWindowSubclass(edit, Some(combo_edit_proc), 2, app.hwnd as usize);
            }
        }
    }

    app.style_combo = style_combo;
    app.font_combo = font_combo;
    app.size_combo = size_combo;

    if !style_combo.is_null() {
        for label in ["Normal", "Heading 1", "Heading 2", "Heading 3", "Heading 4"] {
            combo_add(style_combo, label);
        }
        // SAFETY: plain combo message.
        unsafe {
            SendMessageW(
                style_combo,
                windows_sys::Win32::UI::WindowsAndMessaging::CB_SETCURSEL,
                0,
                0,
            );
        }
    }
    if !font_combo.is_null() {
        for face in installed_fonts(bar) {
            combo_add(font_combo, &face);
        }
        combo_select_text(font_combo, &format::body_font());
    }
    if !size_combo.is_null() {
        for points in 8..=72 {
            combo_add(size_combo, &points.to_string());
        }
        combo_select_text(size_combo, "11");
    }
}

/// Family names of every installed font (sorted, no vertical `@` faces).
fn installed_fonts(window: HWND) -> Vec<String> {
    unsafe extern "system" fn collect(
        logfont: *const LOGFONTW,
        _metrics: *const TEXTMETRICW,
        kind: u32,
        data: LPARAM,
    ) -> i32 {
        // SAFETY: `data` is the `BTreeSet` passed by `installed_fonts`.
        let names = &mut *(data as *mut BTreeSet<String>);
        // Rich Edit silently substitutes a TrueType face for old bitmap fonts
        // (Fixedsys, Terminal, System), so listing them would only mislead.
        if kind & RASTER_FONTTYPE != 0 {
            return 1;
        }
        if logfont.is_null() || names.len() > 4096 {
            return 0;
        }
        // SAFETY: the callback receives a valid LOGFONTW.
        let face = &(*logfont).lfFaceName;
        let end = face
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(face.len());
        let name = String::from_utf16_lossy(&face[..end]);
        if !name.is_empty() && !name.starts_with('@') {
            names.insert(name);
        }
        1
    }

    let mut names: BTreeSet<String> = BTreeSet::new();
    // SAFETY: the DC is released after the enumeration; `names` outlives it.
    unsafe {
        let dc = GetDC(window);
        if !dc.is_null() {
            let mut filter: LOGFONTW = std::mem::zeroed();
            filter.lfCharSet = DEFAULT_CHARSET;
            EnumFontFamiliesExW(
                dc,
                &filter,
                Some(collect),
                &mut names as *mut BTreeSet<String> as LPARAM,
                0,
            );
            ReleaseDC(window, dc);
        }
    }
    names.into_iter().collect()
}

/// Creates both toolbars.
pub fn create(app: &mut App) {
    app.toolbar = create_main(app);
    app.formatbar = create_format(app);
    if !app.formatbar.is_null() {
        create_combos(app, app.formatbar);
        // The bar's buttons and combos send `WM_COMMAND`/`WM_NOTIFY` to their
        // parent (the bar); route them to the frame, which owns the commands.
        // SAFETY: the bar is alive and the subclass outlives this call.
        unsafe {
            SetWindowSubclass(app.formatbar, Some(forward_to_parent), 1, app.hwnd as usize);
        }
    }
}

/// Subclass procedure that relays the format bar's notifications upward.
unsafe extern "system" fn forward_to_parent(
    hwnd: HWND,
    msg: u32,
    wparam: usize,
    lparam: isize,
    _id: usize,
    data: usize,
) -> isize {
    if (msg == windows_sys::Win32::UI::WindowsAndMessaging::WM_COMMAND
        || msg == windows_sys::Win32::UI::WindowsAndMessaging::WM_NOTIFY)
        && data != 0
    {
        return SendMessageW(data as HWND, msg, wparam, lparam);
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// True when `window` is one of the three format comboboxes.
pub fn is_format_combo(app: &App, window: HWND) -> bool {
    !window.is_null()
        && (window == app.style_combo || window == app.font_combo || window == app.size_combo)
}

// ------------------------------------------------------------------- geometry

/// Places the comboboxes over the separator that reserves their space.
pub fn relayout(app: &mut App) {
    if app.formatbar.is_null() || app.style_combo.is_null() {
        return;
    }
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    // SAFETY: `rect` is a valid out parameter; the bar is ours.
    let ok = unsafe {
        SendMessageW(
            app.formatbar,
            TB_GETRECT,
            COMBO_SEP_ID as usize,
            &mut rect as *mut RECT as isize,
        )
    };
    if ok == 0 {
        return;
    }
    let top = rect.top + app.scale(1);
    // The window height is the height of the dropped-down list; only the top
    // part of a closed combobox is visible.
    let height = app.scale(260);
    let style_width = app.scale(style_combo_width(app));
    let font_width = app.scale(COMBO_FONT_WIDTH);
    let size_width = app.scale(COMBO_SIZE_WIDTH);
    let gap = app.scale(COMBO_GAP);
    let place = |combo: HWND, offset: i32, width: i32| {
        if combo.is_null() {
            return;
        }
        // SAFETY: `combo` is our child of the format bar.
        unsafe {
            SetWindowPos(
                combo,
                ptr::null_mut(),
                rect.left + offset,
                top,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    };
    let style_left = app.scale(COMBO_EDGE);
    place(app.style_combo, style_left, style_width);
    let font_left = style_left + style_width + gap;
    place(app.font_combo, font_left, font_width);
    let size_left = font_left + font_width + gap;
    place(app.size_combo, size_left, size_width);
}

/// Width needed by the widest style name (`Heading 1`…`Heading 4`) plus the
/// drop-down arrow and padding, in 96-DPI pixels.
fn style_combo_width(app: &App) -> i32 {
    let labels = ["Normal", "Heading 1", "Heading 2", "Heading 3", "Heading 4"];
    let mut widest = 0i32;
    // SAFETY: the DC is released below and the font belongs to the app.
    unsafe {
        let dc = GetDC(app.formatbar);
        if !dc.is_null() {
            let previous = SelectObject(dc, app.ui_font);
            for label in labels {
                let wide: Vec<u16> = label.encode_utf16().collect();
                let mut size = SIZE { cx: 0, cy: 0 };
                if GetTextExtentPoint32W(dc, wide.as_ptr(), wide.len() as i32, &mut size) != 0 {
                    widest = widest.max(size.cx);
                }
            }
            SelectObject(dc, previous);
            ReleaseDC(app.formatbar, dc);
        }
    }
    if widest == 0 {
        widest = 60;
    }
    // The measured width is already in device pixels; convert to 96-DPI.
    let dpi = app.dpi.max(1) as i32;
    win::mul_div(widest.clamp(60, 200) + 34, 96, dpi)
}

/// Total width of the three combos and their gaps, in 96-DPI pixels.
fn combo_area(app: &App) -> i32 {
    COMBO_EDGE
        + style_combo_width(app)
        + COMBO_GAP
        + COMBO_FONT_WIDTH
        + COMBO_GAP
        + COMBO_SIZE_WIDTH
        + COMBO_EDGE
}

/// Recomputes the toolbar heights after a font/DPI change.
pub fn refresh_height(app: &mut App) {
    // SAFETY: the bars are live toolbar windows.
    unsafe {
        for (bar, minimum) in [(app.toolbar, 28), (app.formatbar, 32)] {
            if bar.is_null() {
                continue;
            }
            SendMessageW(bar, TB_AUTOSIZE, 0, 0);
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(bar, &mut rect) != 0 {
                let height = (rect.bottom - rect.top).max(app.scale(minimum));
                if bar == app.toolbar {
                    app.toolbar_height = height;
                } else {
                    app.formatbar_height = height;
                }
            }
        }
    }
    relayout(app);
}

// -------------------------------------------------------------------- syncing

fn combo_text(combo: HWND) -> String {
    if combo.is_null() {
        return String::new();
    }
    // SAFETY: `combo` is our control; the buffer is NUL terminated by the call.
    unsafe {
        let length = GetWindowTextLengthW(combo);
        let mut buffer = vec![0u16; (length + 1).max(1) as usize];
        GetWindowTextW(combo, buffer.as_mut_ptr(), buffer.len() as i32);
        let end = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..end])
    }
}

/// Mirrors the formatting at the caret in the comboboxes. Called on
/// `EN_SELCHANGE` and after every formatting command.
pub fn sync_format(app: &mut App) {
    if app.suppress_combo > 0 {
        return;
    }
    let Some(editor) = app.active_editor() else {
        return;
    };
    app.suppress_combo += 1;
    let level = format::heading_level_at(editor);
    if !app.style_combo.is_null() {
        // SAFETY: plain combo message.
        unsafe {
            SendMessageW(
                app.style_combo,
                windows_sys::Win32::UI::WindowsAndMessaging::CB_SETCURSEL,
                level.min(4),
                0,
            );
        }
    }
    let current = editor::selection_format(editor);
    if !app.font_combo.is_null() {
        let name = if current.dw_mask & editor::CFM_FACE != 0 {
            format::face_name(&current)
        } else {
            format::body_font()
        };
        if !combo_select_text(app.font_combo, &name) {
            combo_add(app.font_combo, &name);
            combo_select_text(app.font_combo, &name);
        }
    }
    if !app.size_combo.is_null() {
        let points = if current.dw_mask & editor::CFM_SIZE != 0 {
            (current.y_height + 10) / 20
        } else {
            11
        };
        let points = points.clamp(8, 72);
        // SAFETY: plain combo message.
        unsafe {
            SendMessageW(
                app.size_combo,
                windows_sys::Win32::UI::WindowsAndMessaging::CB_SETCURSEL,
                (points - 8) as usize,
                0,
            );
        }
    }
    app.suppress_combo -= 1;
}

/// Mirrors `View > Word Wrap` in the toolbar button's checked state.
pub fn sync_word_wrap(app: &mut App) {
    if app.toolbar.is_null() {
        return;
    }
    let state = TBSTATE_ENABLED | if app.word_wrap { TBSTATE_CHECKED } else { 0 };
    // SAFETY: plain toolbar message on our own bar.
    unsafe {
        SendMessageW(
            app.toolbar,
            TB_SETSTATE,
            IDM_VIEW_WORD_WRAP as usize,
            state as isize,
        );
    }
}

/// Text currently shown by the style combobox (test hook and debugging).
pub fn style_text(app: &mut App) -> String {
    sync_format(app);
    combo_text(app.style_combo)
}

/// Font name currently shown by the font combobox.
pub fn font_text(app: &mut App) -> String {
    sync_format(app);
    combo_text(app.font_combo)
}

/// Font size currently shown by the size combobox.
pub fn size_text(app: &mut App) -> String {
    sync_format(app);
    combo_text(app.size_combo)
}

/// Applies style `level` (0 = Normal, 1..=4 = Heading).
pub fn apply_style(app: &mut App, level: usize) {
    if crate::wnd::editor::refuse_if_locked(app) {
        return;
    }
    let Some(editor) = app.active_editor() else {
        return;
    };
    format::heading(editor, level);
    app.mark_dirty(editor);
    // SAFETY: focusing our own editor.
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(editor);
    }
    sync_format(app);
}

fn apply_face(app: &mut App, name: &str) {
    if crate::wnd::editor::refuse_if_locked(app) {
        return;
    }
    if name.is_empty() {
        return;
    }
    let Some(editor) = app.active_editor() else {
        return;
    };
    let mut format = editor::CharFormat2W {
        dw_mask: editor::CFM_FACE,
        ..editor::CharFormat2W::default()
    };
    for (slot, unit) in format
        .sz_face_name
        .iter_mut()
        .zip(name.encode_utf16().take(31))
    {
        *slot = unit;
    }
    editor::set_selection_format(editor, &format);
    app.mark_dirty(editor);
    // SAFETY: focusing our own editor.
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(editor);
    }
    sync_format(app);
}

fn apply_size(app: &mut App, points: i32) {
    if crate::wnd::editor::refuse_if_locked(app) {
        return;
    }
    let Some(editor) = app.active_editor() else {
        return;
    };
    let format = editor::CharFormat2W {
        dw_mask: editor::CFM_SIZE,
        y_height: points.clamp(1, 400) * 20,
        ..editor::CharFormat2W::default()
    };
    editor::set_selection_format(editor, &format);
    app.mark_dirty(editor);
    // SAFETY: focusing our own editor.
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(editor);
    }
    sync_format(app);
}

/// `WM_COMMAND` code the typed combos' edit boxes send for Enter.
pub const COMBO_COMMIT: u16 = 0x0700;
/// `WM_COMMAND` code for Escape in a typed combo.
pub const COMBO_CANCEL: u16 = 0x0701;

/// Keys of the font/size combos' edit boxes: Enter applies, Escape goes back
/// to the note, typed letters are completed from the list.
unsafe extern "system" fn combo_edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: usize,
    lparam: isize,
    _id: usize,
    frame: usize,
) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetDlgCtrlID, GetParent, PostMessageW, WM_CHAR, WM_COMMAND, WM_KEYDOWN,
    };
    let combo = GetParent(hwnd);
    match msg {
        WM_KEYDOWN if wparam == 0x0D || wparam == 0x1B => {
            let code = if wparam == 0x0D {
                COMBO_COMMIT
            } else {
                COMBO_CANCEL
            };
            let id = GetDlgCtrlID(combo) as usize & 0xFFFF;
            PostMessageW(
                frame as HWND,
                WM_COMMAND,
                id | (usize::from(code) << 16),
                combo as isize,
            );
            return 0;
        }
        // No beep for the Enter / Escape characters.
        WM_CHAR if wparam == 0x0D || wparam == 0x1B => return 0,
        WM_CHAR if wparam >= 0x20 && wparam != 0x7F => {
            let result = DefSubclassProc(hwnd, msg, wparam, lparam);
            complete_typed(combo, hwnd);
            return result;
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// Completes the typed prefix with the first matching list item and selects
/// the completed part, so typing on replaces it.
unsafe fn complete_typed(combo: HWND, edit: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowTextW, CB_FINDSTRING, CB_GETLBTEXT, CB_GETLBTEXTLEN,
    };
    let typed = combo_text(edit);
    let mut from = 0u32;
    let mut to = 0u32;
    SendMessageW(
        edit,
        EM_GETSEL_EDIT,
        &mut from as *mut u32 as usize,
        &mut to as *mut u32 as isize,
    );
    let typed_units = typed.encode_utf16().count();
    if typed.is_empty() || to as usize != typed_units {
        return;
    }
    let wide = win::wide(&typed);
    let index = SendMessageW(combo, CB_FINDSTRING, usize::MAX, wide.as_ptr() as isize);
    if index < 0 {
        return;
    }
    let length = SendMessageW(combo, CB_GETLBTEXTLEN, index as usize, 0);
    if length <= 0 {
        return;
    }
    let mut buffer = vec![0u16; length as usize + 1];
    SendMessageW(
        combo,
        CB_GETLBTEXT,
        index as usize,
        buffer.as_mut_ptr() as isize,
    );
    buffer.truncate(length as usize);
    if buffer.len() <= typed_units {
        return;
    }
    // Keep what was typed (its case), complete the rest from the item.
    let mut completed: Vec<u16> = typed.encode_utf16().collect();
    completed.extend_from_slice(&buffer[typed_units..]);
    completed.push(0);
    SetWindowTextW(edit, completed.as_ptr());
    SendMessageW(edit, EM_SETSEL_EDIT, typed_units, -1);
}

const EM_GETSEL_EDIT: u32 = 0x00B0;
const EM_SETSEL_EDIT: u32 = 0x00B1;

/// The list item matching `text`: exactly (ignoring case) or, failing that,
/// the first item starting with it.
fn combo_match(combo: HWND, text: &str) -> Option<String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{CB_FINDSTRING, CB_FINDSTRINGEXACT};
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let wide = win::wide(text);
    // SAFETY: plain combo queries with a NUL-terminated string.
    let index = unsafe {
        let exact = SendMessageW(
            combo,
            CB_FINDSTRINGEXACT,
            usize::MAX,
            wide.as_ptr() as isize,
        );
        if exact >= 0 {
            exact
        } else {
            SendMessageW(combo, CB_FINDSTRING, usize::MAX, wide.as_ptr() as isize)
        }
    };
    combo_item(combo, index)
}

/// Text of list item `index` of `combo`.
fn combo_item(combo: HWND, index: isize) -> Option<String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{CB_GETLBTEXT, CB_GETLBTEXTLEN};
    if index < 0 {
        return None;
    }
    // SAFETY: plain combo queries; the buffer is sized from the length.
    unsafe {
        let length = SendMessageW(combo, CB_GETLBTEXTLEN, index as usize, 0).max(0) as usize;
        let mut buffer = vec![0u16; length + 1];
        SendMessageW(
            combo,
            CB_GETLBTEXT,
            index as usize,
            buffer.as_mut_ptr() as isize,
        );
        buffer.truncate(length);
        Some(String::from_utf16_lossy(&buffer))
    }
}

/// Font/size combo: an item was picked from the open list (`CBN_SELENDOK`),
/// Enter was pressed in its edit box ([`COMBO_COMMIT`]) or Escape
/// ([`COMBO_CANCEL`]).
pub fn on_combo_commit(app: &mut App, combo: HWND, code: u16) {
    if app.suppress_combo > 0 {
        return;
    }
    let text = if code == COMBO_COMMIT {
        combo_text(combo)
    } else {
        // SAFETY: plain combo query.
        let index = unsafe {
            SendMessageW(
                combo,
                windows_sys::Win32::UI::WindowsAndMessaging::CB_GETCURSEL,
                0,
                0,
            )
        };
        combo_item(combo, index).unwrap_or_default()
    };
    if code != COMBO_CANCEL {
        if combo == app.font_combo {
            if let Some(face) = combo_match(combo, &text) {
                apply_face(app, &face);
                return;
            }
            let message = format!(
                "\"{}\" cannot be used (not installed, or an old bitmap font such as Fixedsys) — try Consolas or Lucida Console",
                text.trim()
            );
            crate::wnd::status::set_message(app, &message);
        } else if combo == app.size_combo {
            match text.trim().parse::<f32>() {
                Ok(points) if (1.0..=400.0).contains(&points) => {
                    apply_size(app, points.round() as i32);
                    return;
                }
                _ => crate::wnd::status::set_message(app, "Type a font size in points, e.g. 12"),
            }
        }
    }
    sync_format(app);
    if let Some(editor) = app.active_editor() {
        // SAFETY: focusing our own editor.
        unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(editor);
        }
    }
}

/// `CBN_SELCHANGE` from one of the format comboboxes. Only the style combo
/// applies at once; font and size wait for [`on_combo_commit`].
pub fn on_combo(app: &mut App, combo: HWND) {
    if app.suppress_combo > 0 {
        return;
    }
    if combo == app.style_combo {
        // SAFETY: plain combo message.
        let index = unsafe {
            SendMessageW(
                app.style_combo,
                windows_sys::Win32::UI::WindowsAndMessaging::CB_GETCURSEL,
                0,
                0,
            )
        };
        if index >= 0 {
            apply_style(app, index as usize);
        }
    }
}

// -------------------------------------------------------------------- palettes

/// Screen rectangle of a toolbar button (both bars are searched).
fn button_rect(app: &App, command: u16) -> Option<RECT> {
    for bar in [app.formatbar, app.toolbar] {
        if bar.is_null() {
            continue;
        }
        let mut rect: RECT = unsafe { std::mem::zeroed() };
        // SAFETY: `rect` is a valid out parameter; `bar` is ours.
        let ok = unsafe {
            SendMessageW(
                bar,
                TB_GETRECT,
                command as usize,
                &mut rect as *mut RECT as isize,
            )
        };
        if ok == 0 {
            continue;
        }
        let mut top_left = POINT {
            x: rect.left,
            y: rect.top,
        };
        let mut bottom_right = POINT {
            x: rect.right,
            y: rect.bottom,
        };
        // SAFETY: both points are valid; `bar` is alive.
        unsafe {
            ClientToScreen(bar, &mut top_left);
            ClientToScreen(bar, &mut bottom_right);
        }
        return Some(RECT {
            left: top_left.x,
            top: top_left.y,
            right: bottom_right.x,
            bottom: bottom_right.y,
        });
    }
    None
}

fn show_palette_at(app: &App, highlight: bool, colours: &[(&str, u32)]) -> Option<u16> {
    // SAFETY: we create and destroy the popup in this function.
    let menu: HMENU = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return None;
    }
    // SAFETY: `menu` is ours; labels are static strings.
    unsafe {
        for (index, (label, _)) in colours.iter().enumerate() {
            AppendMenuW(
                menu,
                MF_STRING,
                (PALETTE_BASE + index as u16) as usize,
                win::wide(label).as_ptr(),
            );
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(
            menu,
            MF_STRING,
            PALETTE_MORE as usize,
            win::wide("More...").as_ptr(),
        );
        AppendMenuW(
            menu,
            MF_STRING,
            PALETTE_NONE as usize,
            win::wide("None").as_ptr(),
        );
    }

    let command_id = if highlight {
        IDM_TOOLBAR_HIGHLIGHT
    } else {
        IDM_TOOLBAR_COLOR
    };
    let point = button_rect(app, command_id)
        .map(|rect| POINT {
            x: rect.left,
            y: rect.bottom + 2,
        })
        .unwrap_or_else(|| {
            let mut cursor = POINT { x: 0, y: 0 };
            // SAFETY: `cursor` is a valid out parameter.
            unsafe {
                GetCursorPos(&mut cursor);
            }
            cursor
        });
    // SAFETY: `menu` is our popup; `hwnd` is our frame window.
    let command = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN,
            point.x,
            point.y,
            0,
            app.hwnd,
            ptr::null(),
        )
    } as u16;
    // SAFETY: the popup is no longer needed.
    unsafe {
        DestroyMenu(menu);
    }
    if command == 0 {
        None
    } else {
        Some(command)
    }
}

/// Shows the text-colour or highlight palette (toolbar button or its arrow).
pub fn show_palette(app: &mut App, highlight: bool) {
    if crate::wnd::editor::refuse_if_locked(app) {
        return;
    }
    let Some(editor) = app.active_editor() else {
        return;
    };
    let colours = if highlight {
        &HIGHLIGHT_COLOURS[..]
    } else {
        &TEXT_COLOURS[..]
    };
    let Some(command) = show_palette_at(app, highlight, colours) else {
        // SAFETY: focusing our own editor.
        unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(editor);
        }
        return;
    };
    let index = command.wrapping_sub(PALETTE_BASE) as usize;
    let mut changed = false;
    if index < colours.len() {
        let colour = colours[index].1;
        if highlight {
            format::set_highlight(editor, Some(colour));
        } else {
            format::set_text_color(editor, Some(colour));
        }
        changed = true;
    } else if command == PALETTE_MORE {
        if highlight {
            format::choose_highlight_color(editor, app.hwnd);
        } else {
            format::choose_text_color(editor, app.hwnd);
        }
        changed = true;
    } else if command == PALETTE_NONE {
        if highlight {
            format::set_highlight(editor, None);
        } else {
            format::set_text_color(editor, None);
        }
        changed = true;
    }
    if changed {
        app.mark_dirty(editor);
    }
    // SAFETY: focusing our own editor.
    unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(editor);
    }
    sync_format(app);
}

/// `TBN_DROPDOWN` from a palette button's arrow.
///
/// # Safety
/// `lparam` must be the `NMTOOLBARW` pointer of a `TBN_DROPDOWN`.
pub unsafe fn on_dropdown(app: &mut App, lparam: isize) {
    let info = &*(lparam as *const windows_sys::Win32::UI::Controls::NMTOOLBARW);
    let command = info.tbButton.idCommand as u16;
    if command == IDM_TOOLBAR_COLOR {
        show_palette(app, false);
    } else if command == IDM_TOOLBAR_HIGHLIGHT {
        show_palette(app, true);
    }
}

/// True when `name` is an installed font (the font combo lists them all).
pub fn font_installed(app: &App, name: &str) -> bool {
    if app.font_combo.is_null() {
        return true;
    }
    let wide = win::wide(name.trim());
    // SAFETY: plain combo query with a NUL-terminated string.
    let index = unsafe {
        SendMessageW(
            app.font_combo,
            windows_sys::Win32::UI::WindowsAndMessaging::CB_FINDSTRINGEXACT,
            usize::MAX,
            wide.as_ptr() as isize,
        )
    };
    index >= 0
}

/// The first listed (drawable) font whose name starts with `name`, e.g.
/// "Fixedsys Excelsior 3.01" for "Fixedsys".
pub fn closest_font(app: &App, name: &str) -> Option<String> {
    if app.font_combo.is_null() {
        return None;
    }
    combo_match(app.font_combo, name)
}
