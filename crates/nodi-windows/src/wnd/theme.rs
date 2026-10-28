//! Light / Dark / Custom palettes and their application to the native shell.
//!
//! The notes themselves never change: dark mode only changes how automatic and
//! pure-black text is *displayed*, by pointing those colour runs at two
//! near-white colour-table entries ([`nodi_core::rtf::apply_theme_colors`]).
//! Every save folds those sentinels back ([`nodi_core::rtf::fold_theme_colors`]),
//! so `notes/*.rtf` keeps the colours the user chose.
//!
//! Menus and message boxes stay in the system theme: Windows has no supported
//! API for dark menus (only the undocumented `SetPreferredAppMode`), and a
//! half-dark menu is worse than a consistently light one. Everything else —
//! frame, editor, tree, tabs, toolbars, status bar, splitter and title bar — is
//! themed here.

use std::mem::size_of;
use std::ptr;

use nodi_core::rtf::{apply_theme_colors, fold_theme_colors};
use nodi_core::storage::{parse_hex_color, Settings, Theme};
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, FillRect, InvalidateRect, SelectObject, HBRUSH, HDC, HGDIOBJ,
};
use windows_sys::Win32::UI::Controls::{
    SetWindowTheme, CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDIS_CHECKED, CDRF_DODEFAULT, CDRF_NEWFONT,
    CDRF_NOTIFYITEMDRAW, NMHDR, NMTBCUSTOMDRAW, NM_CUSTOMDRAW, TBCDRF_NOBACKGROUND, TBCDRF_NOEDGES,
    TBCDRF_USECDCOLORS,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_GETFONT};

use crate::app::App;
use crate::win;
use crate::wnd::{editor, status, tabs};

/// A pair of optional RGB sentinel colours for theme colour substitution.
type SentinelPair = (Option<(u8, u8, u8)>, Option<(u8, u8, u8)>);

/// One resolved colour scheme (COLORREF values, `0x00bbggrr`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// True when the window frame/tree should be painted dark.
    pub dark: bool,
    pub window_bg: u32,
    pub window_text: u32,
    pub editor_bg: u32,
    pub editor_text: u32,
    pub tree_bg: u32,
    pub tab_bg: u32,
    pub tab_active: u32,
    pub accent: u32,
    pub border: u32,
    /// Colour-table entry that replaces automatic text while displayed.
    pub auto_sentinel: Option<(u8, u8, u8)>,
    /// Colour-table entry that replaces explicit black while displayed.
    pub black_sentinel: Option<(u8, u8, u8)>,
}

/// `COLORREF` from components.
pub const fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    red as u32 | (green as u32) << 8 | (blue as u32) << 16
}

fn darken(colour: u32, amount: u8) -> u32 {
    let channel = |shift: u32| {
        let value = ((colour >> shift) & 0xFF) as u8;
        value.saturating_sub(amount)
    };
    rgb(channel(0), channel(8), channel(16))
}

fn lighten(colour: u32, amount: u8) -> u32 {
    let channel = |shift: u32| {
        let value = ((colour >> shift) & 0xFF) as u8;
        value.saturating_add(amount)
    };
    rgb(channel(0), channel(8), channel(16))
}

fn luminance(colour: u32) -> f32 {
    let red = (colour & 0xFF) as f32 / 255.0;
    let green = ((colour >> 8) & 0xFF) as f32 / 255.0;
    let blue = ((colour >> 16) & 0xFF) as f32 / 255.0;
    0.2126 * red + 0.7152 * green + 0.0722 * blue
}

fn hex_colour(text: &str, fallback: u32) -> u32 {
    match parse_hex_color(text) {
        Some((red, green, blue)) => rgb(red, green, blue),
        None => fallback,
    }
}

/// A near-white (or near-text) colour that differs from `value` so explicit
/// black text stays distinguishable from automatic text.
fn nudge((red, green, blue): (u8, u8, u8)) -> (u8, u8, u8) {
    if red < 250 {
        (red + 1, green, blue)
    } else if green < 250 {
        (red, green + 1, blue)
    } else if blue < 250 {
        (red, green, blue + 1)
    } else {
        (red - 1, green, blue)
    }
}

impl Palette {
    pub fn from_settings(settings: &Settings) -> Self {
        match settings.theme {
            Theme::Light => Self {
                dark: false,
                window_bg: rgb(240, 240, 240),
                window_text: rgb(31, 31, 31),
                editor_bg: rgb(255, 255, 255),
                editor_text: rgb(31, 31, 31),
                tree_bg: rgb(255, 255, 255),
                tab_bg: rgb(240, 240, 240),
                tab_active: rgb(255, 255, 255),
                accent: rgb(204, 232, 255),
                border: rgb(200, 200, 200),
                auto_sentinel: None,
                black_sentinel: None,
            },
            Theme::Dark => Self {
                dark: true,
                window_bg: rgb(32, 32, 32),
                window_text: rgb(230, 230, 230),
                editor_bg: rgb(30, 30, 30),
                editor_text: rgb(230, 230, 230),
                tree_bg: rgb(37, 37, 38),
                tab_bg: rgb(43, 43, 43),
                tab_active: rgb(30, 30, 30),
                accent: rgb(43, 87, 154),
                border: rgb(70, 70, 70),
                auto_sentinel: Some((230, 230, 230)),
                black_sentinel: Some((231, 231, 231)),
            },
            Theme::Custom => {
                let custom = &settings.custom;
                let background = hex_colour(&custom.background, 0x00FF_FFFF);
                let text = hex_colour(&custom.text, 0x001F_1F1F);
                let accent = hex_colour(&custom.accent, 0x00FF_E8CC);
                let dark = luminance(background) < 0.5;
                let text_rgb = (
                    (text & 0xFF) as u8,
                    ((text >> 8) & 0xFF) as u8,
                    ((text >> 16) & 0xFF) as u8,
                );
                Self {
                    dark,
                    window_bg: if dark {
                        lighten(background, 10)
                    } else {
                        darken(background, 12)
                    },
                    window_text: text,
                    editor_bg: background,
                    editor_text: text,
                    tree_bg: background,
                    tab_bg: if dark {
                        lighten(background, 12)
                    } else {
                        darken(background, 8)
                    },
                    tab_active: background,
                    accent,
                    border: if dark {
                        lighten(background, 34)
                    } else {
                        darken(background, 40)
                    },
                    auto_sentinel: Some(text_rgb),
                    black_sentinel: Some(nudge(text_rgb)),
                }
            }
        }
    }

    /// The two colour-table entries used to render dark themes.
    fn sentinels(&self) -> SentinelPair {
        (self.auto_sentinel, self.black_sentinel)
    }

    /// Hands the editors this palette's display colours for automatic and
    /// black text (see `editor::set_display_colours`).
    pub fn install_display_colours(&self) {
        let colours = match self.sentinels() {
            (Some(auto), Some(black)) => {
                Some((rgb(auto.0, auto.1, auto.2), rgb(black.0, black.1, black.2)))
            }
            _ => None,
        };
        editor::set_display_colours(colours);
    }

    pub fn brush(&self) -> HBRUSH {
        // SAFETY: creates an owned GDI brush; the caller owns it.
        unsafe { CreateSolidBrush(self.window_bg) }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::from_settings(&Settings::default())
    }
}

/// RTF for display in `palette` (adds the theme sentinels when needed).
pub fn theme_bytes(palette: &Palette, rtf: &[u8]) -> Vec<u8> {
    match (palette.auto_sentinel, palette.black_sentinel) {
        (Some(auto), Some(black)) => {
            let text = String::from_utf8_lossy(rtf);
            apply_theme_colors(&text, auto, black).into_bytes()
        }
        _ => rtf.to_vec(),
    }
}

/// RTF as it must be stored on disk (sentinels folded back to auto/black).
pub fn fold_bytes(palette: &Palette, rtf: &[u8]) -> Vec<u8> {
    match (palette.auto_sentinel, palette.black_sentinel) {
        (Some(auto), Some(black)) => {
            let text = String::from_utf8_lossy(rtf);
            fold_theme_colors(&text, auto, black).into_bytes()
        }
        _ => rtf.to_vec(),
    }
}

/// Reloads one editor from its stored RTF (`stream_out_rtf` already folded the
/// old palette's display colours) in `new`'s display colours, keeping the
/// caret, selection and first visible line.
fn retint_editor(editor: HWND, stored: &[u8], new: &Palette) {
    let themed = theme_bytes(new, stored);
    let (from, to) = editor::selection(editor);
    let line = editor::first_visible_line(editor);
    editor::set_change_events(editor, false);
    editor::load_rtf(editor, &themed);
    editor::set_selection(editor, from, to);
    editor::set_change_events(editor, true);
    editor::scroll_to_line(editor, line);
}

/// Paints `rect` in `hdc` with `colour`.
pub unsafe fn fill(hdc: HDC, rect: &windows_sys::Win32::Foundation::RECT, colour: u32) {
    let brush = CreateSolidBrush(colour);
    if !brush.is_null() {
        FillRect(hdc, rect, brush);
        DeleteObject(brush);
    }
}

/// Applies the current `app.settings.theme` everywhere, immediately.
pub fn apply(app: &mut App) {
    let old = app.palette;
    let new = Palette::from_settings(&app.settings);
    if old.sentinels() != new.sentinels() {
        // Read every editor while the old display colours are installed, so
        // they fold back correctly, then reload in the new ones.
        let stored: Vec<(HWND, Vec<u8>)> = app
            .tabs
            .iter()
            .filter_map(|tab| editor::stream_out_rtf(tab.editor).map(|rtf| (tab.editor, rtf)))
            .collect();
        new.install_display_colours();
        for (window, rtf) in &stored {
            retint_editor(*window, rtf, &new);
        }
    }
    new.install_display_colours();
    app.palette = new;

    if !app.background_brush.is_null() {
        // SAFETY: we created this brush and replace it below.
        unsafe {
            DeleteObject(app.background_brush);
        }
    }
    app.background_brush = new.brush();

    // SAFETY: every handle below belongs to this process and lives as long as
    // the frame; the DWM call only reads the BOOL.
    unsafe {
        let dark: BOOL = i32::from(new.dark);
        let attribute = DWMWA_USE_IMMERSIVE_DARK_MODE as u32;
        if DwmSetWindowAttribute(
            app.hwnd,
            attribute,
            &dark as *const BOOL as *const core::ffi::c_void,
            size_of::<BOOL>() as u32,
        ) < 0
        {
            // Windows 10 before 20H1 used attribute 19.
            DwmSetWindowAttribute(
                app.hwnd,
                19,
                &dark as *const BOOL as *const core::ffi::c_void,
                size_of::<BOOL>() as u32,
            );
        }

        let dark_theme = win::wide("DarkMode_Explorer");
        let no_theme = win::wide("");
        let sub_app = if new.dark {
            dark_theme.as_ptr()
        } else {
            no_theme.as_ptr()
        };
        // The icon toolbar keeps the light look (see `on_notify`).
        for window in [app.tree_view, app.tabctrl, app.status, app.formatbar] {
            if !window.is_null() {
                SetWindowTheme(window, sub_app, ptr::null());
            }
        }
        for tab in &app.tabs {
            SetWindowTheme(tab.editor, sub_app, ptr::null());
            editor::set_background(tab.editor, new.editor_bg);
            editor::set_spell_check(tab.editor, app.settings.spell_check);
        }

        // TreeView has built-in colours for its items; the selected row keeps
        // the system highlight, which is readable in both palettes.
        if !app.tree_view.is_null() {
            let text = if new.dark {
                new.window_text
            } else {
                0xFFFF_FFFF // CLR_NONE: back to the system default
            };
            let background = if new.dark { new.tree_bg } else { 0xFFFF_FFFF };
            SendMessageW(app.tree_view, TVM_SETBKCOLOR, 0, background as isize);
            SendMessageW(app.tree_view, TVM_SETTEXTCOLOR, 0, text as isize);
        }

        // The sidebar caption is a static control without its own colours.
        if !app.sidebar_title.is_null() {
            InvalidateRect(app.sidebar_title, ptr::null(), 1);
        }
    }

    status::reapply(app);
    tabs::invalidate(app);

    // SAFETY: invalidating our own windows.
    unsafe {
        InvalidateRect(app.hwnd, ptr::null(), 1);
        if !app.tree_view.is_null() {
            InvalidateRect(app.tree_view, ptr::null(), 1);
        }
        if !app.toolbar.is_null() {
            InvalidateRect(app.toolbar, ptr::null(), 1);
        }
        if !app.formatbar.is_null() {
            InvalidateRect(app.formatbar, ptr::null(), 1);
        }
        if !app.status.is_null() {
            InvalidateRect(app.status, ptr::null(), 1);
        }
    }
}

/// `NM_CUSTOMDRAW` handling for the two toolbars. Returns `None` when the
/// notification is not ours.
pub unsafe fn on_notify(app: &App, header: &NMHDR, lparam: LPARAM) -> Option<LRESULT> {
    if header.code != NM_CUSTOMDRAW
        || (header.hwndFrom != app.toolbar && header.hwndFrom != app.formatbar)
    {
        // TreeView uses TVM_SETBKCOLOR/TVM_SETTEXTCOLOR instead.
        return None;
    }
    if !app.palette.dark {
        // The light theme keeps the native toolbar look.
        return Some(CDRF_DODEFAULT as LRESULT);
    }
    let draw = &mut *(lparam as *mut NMTBCUSTOMDRAW);
    if header.hwndFrom == app.toolbar {
        // The icon toolbar uses Windows' black standard glyphs, unreadable on
        // a dark strip; it stays light like the menu bar right above it.
        if draw.nmcd.dwDrawStage == CDDS_PREPAINT && !draw.nmcd.hdc.is_null() {
            fill(draw.nmcd.hdc, &draw.nmcd.rc, rgb(240, 240, 240));
        }
        return Some(CDRF_DODEFAULT as LRESULT);
    }
    let result = match draw.nmcd.dwDrawStage {
        CDDS_PREPAINT => {
            if !draw.nmcd.hdc.is_null() {
                fill(draw.nmcd.hdc, &draw.nmcd.rc, app.palette.window_bg);
            }
            CDRF_NOTIFYITEMDRAW
        }
        CDDS_ITEMPREPAINT => {
            draw.clrText = app.palette.window_text;
            draw.clrBtnFace = app.palette.window_bg;
            draw.clrBtnHighlight = app.palette.border;
            draw.clrMark = app.palette.accent;
            // `CDRF_NEWFONT` makes the toolbar draw with the font selected in
            // the DC, which must be its own font: the DC's default font is
            // larger than the one the buttons were measured with.
            let font = SendMessageW(header.hwndFrom, WM_GETFONT, 0, 0);
            if font != 0 && !draw.nmcd.hdc.is_null() {
                SelectObject(draw.nmcd.hdc, font as HGDIOBJ);
            }
            let mut flags = CDRF_NEWFONT | TBCDRF_USECDCOLORS | TBCDRF_NOEDGES;
            if draw.nmcd.uItemState & CDIS_CHECKED != 0 && !draw.nmcd.hdc.is_null() {
                // A checked button (word wrap) is shown in the accent colour;
                // the default light-blue check fill hides light text.
                fill(draw.nmcd.hdc, &draw.nmcd.rc, app.palette.accent);
                flags |= TBCDRF_NOBACKGROUND;
            }
            flags
        }
        _ => CDRF_DODEFAULT,
    };
    Some(result as LRESULT)
}

/// Constant declared here so `mod.rs` can match the tree background helper.
pub const TVM_SETBKCOLOR: u32 = 0x1100 + 29;
pub const TVM_SETTEXTCOLOR: u32 = 0x1100 + 30;
