//! Light / Dark / Custom palettes and their application to the native shell.
//!
//! The notes themselves never change: dark mode only changes how automatic and
//! pure-black text is *displayed*, by pointing those colour runs at two
//! near-white colour-table entries ([`nodi_core::rtf::apply_theme_colors`]).
//! Every save folds those sentinels back ([`nodi_core::rtf::fold_theme_colors`]),
//! so `notes/*.rtf` keeps the colours the user chose.
//!
//! Every part of the window follows the theme: title bar (DWM), menu bar
//! (drawn here through the `WM_UAHDRAWMENU*` messages Windows sends to let
//! applications paint it, as Notepad++ does), pop-up menus (uxtheme's dark
//! mode switch), toolbars, tree, tabs, line numbers, editor and status bar.
//! Themes are full colour sets ([`ThemeSpec`]): presets, plus the user's own
//! saved themes.

use std::cell::Cell;
use std::mem::size_of;
use std::ptr;

use nodi_core::rtf::{apply_theme_colors, fold_theme_colors};
use nodi_core::storage::{parse_hex_color, Settings, Theme, ThemeSpec};
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, FillRect, InvalidateRect, RestoreDC,
    SaveDC, SelectObject, SetBkMode, SetTextColor, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, HBRUSH, HDC, HGDIOBJ, TRANSPARENT,
};
use windows_sys::Win32::UI::Controls::{
    SetWindowTheme, CDDS_ITEMPOSTPAINT, CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDIS_CHECKED, CDIS_HOT,
    CDIS_SELECTED, CDRF_DODEFAULT, CDRF_NEWFONT, CDRF_NOTIFYITEMDRAW, CDRF_NOTIFYPOSTPAINT, NMHDR,
    NMTBCUSTOMDRAW, NM_CUSTOMDRAW, TBCDRF_NOBACKGROUND, TBCDRF_NOEDGES, TBCDRF_USECDCOLORS,
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
    pub tree_text: u32,
    pub tab_bg: u32,
    pub tab_active: u32,
    pub accent: u32,
    pub border: u32,
    /// Line-number strip.
    pub gutter_bg: u32,
    pub gutter_text: u32,
    /// Lines / grid / dots behind the text (View > Background).
    pub pattern: u32,
    /// Colour-table entry that replaces automatic text while displayed.
    pub auto_sentinel: Option<(u8, u8, u8)>,
    /// Colour-table entry that replaces explicit black while displayed.
    pub black_sentinel: Option<(u8, u8, u8)>,
}

/// `COLORREF` from components.
pub const fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    red as u32 | (green as u32) << 8 | (blue as u32) << 16
}

fn rgb_tuple(colour: u32) -> (u8, u8, u8) {
    (
        (colour & 0xFF) as u8,
        ((colour >> 8) & 0xFF) as u8,
        ((colour >> 16) & 0xFF) as u8,
    )
}

thread_local! {
    /// Line-number background of the active theme, used for new page gaps.
    static PAGE_BAND: Cell<u32> = const { Cell::new(0x0052_5252) };
}

/// Remembers the line-number colour for page breaks inserted from now on.
pub fn set_page_band(colour: u32) {
    PAGE_BAND.with(|band| band.set(colour));
}

/// RGB of the active theme's line-number strip.
pub fn page_band_rgb() -> (u8, u8, u8) {
    rgb_tuple(PAGE_BAND.with(Cell::get))
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

/// One theme colour list in [`ThemeSpec`] field order: note bg, note text,
/// window bg, window text, tree bg, tree text, tab bar, active tab, accent,
/// border, line numbers, background pattern.
type Colours = [&'static str; 12];

fn spec(name: &str, colours: Colours) -> ThemeSpec {
    let [editor_bg, editor_text, window_bg, window_text, tree_bg, tree_text, tab_bg, tab_active, accent, border, gutter_text, pattern] =
        colours;
    ThemeSpec {
        name: name.to_owned(),
        editor_bg: editor_bg.to_owned(),
        editor_text: editor_text.to_owned(),
        window_bg: window_bg.to_owned(),
        window_text: window_text.to_owned(),
        tree_bg: tree_bg.to_owned(),
        tree_text: tree_text.to_owned(),
        tab_bg: tab_bg.to_owned(),
        tab_active: tab_active.to_owned(),
        accent: accent.to_owned(),
        border: border.to_owned(),
        gutter_text: gutter_text.to_owned(),
        pattern: pattern.to_owned(),
        ui_font: "Segoe UI".to_owned(),
        ui_size: 9,
    }
}

/// Built-in themes: light ones first, then dark ones modelled on popular
/// editor themes (VS Code Dark+ / Dark Modern, Monokai, Dracula, One Dark,
/// Nord, Solarized, Gruvbox, Tokyo Night, Catppuccin).
pub fn presets() -> Vec<ThemeSpec> {
    vec![
        spec(
            "Light",
            [
                "#ffffff", "#1f1f1f", "#f0f0f0", "#1f1f1f", "#ffffff", "#1f1f1f", "#f0f0f0",
                "#ffffff", "#cce8ff", "#c8c8c8", "#9a9a9a", "#e3e9f0",
            ],
        ),
        spec(
            "Paper",
            [
                "#fbf8f1", "#3b3a36", "#efe9dc", "#3b3a36", "#f6f1e6", "#3b3a36", "#ece5d6",
                "#fbf8f1", "#f3d9a4", "#d8cfbd", "#a59d8c", "#ebe2cf",
            ],
        ),
        spec(
            "GitHub Light",
            [
                "#ffffff", "#1f2328", "#f6f8fa", "#1f2328", "#f6f8fa", "#1f2328", "#eaeef2",
                "#ffffff", "#ddf4ff", "#d0d7de", "#8c959f", "#e7ecf0",
            ],
        ),
        spec(
            "Solarized Light",
            [
                "#fdf6e3", "#586e75", "#eee8d5", "#586e75", "#eee8d5", "#586e75", "#e4ddc8",
                "#fdf6e3", "#d6e6ea", "#d6ceb5", "#93a1a1", "#eee6d0",
            ],
        ),
        spec(
            "Dark",
            [
                "#1e1e1e", "#d4d4d4", "#252526", "#cccccc", "#252526", "#cccccc", "#2d2d2d",
                "#1e1e1e", "#264f78", "#3c3c3c", "#858585", "#2a2a2a",
            ],
        ),
        spec(
            "Dark Modern",
            [
                "#1f1f1f", "#cccccc", "#181818", "#cccccc", "#181818", "#cccccc", "#181818",
                "#1f1f1f", "#0e639c", "#2b2b2b", "#6e7681", "#2a2a2a",
            ],
        ),
        spec(
            "Monokai",
            [
                "#272822", "#f8f8f2", "#1e1f1c", "#f8f8f2", "#1e1f1c", "#f8f8f2", "#34352f",
                "#272822", "#49483e", "#3e3d32", "#90908a", "#32332c",
            ],
        ),
        spec(
            "Dracula",
            [
                "#282a36", "#f8f8f2", "#21222c", "#f8f8f2", "#21222c", "#f8f8f2", "#191a21",
                "#282a36", "#44475a", "#44475a", "#6272a4", "#303241",
            ],
        ),
        spec(
            "One Dark",
            [
                "#282c34", "#abb2bf", "#21252b", "#abb2bf", "#21252b", "#abb2bf", "#21252b",
                "#282c34", "#3e4451", "#181a1f", "#636d83", "#2f343d",
            ],
        ),
        spec(
            "Nord",
            [
                "#2e3440", "#d8dee9", "#3b4252", "#d8dee9", "#2e3440", "#d8dee9", "#3b4252",
                "#2e3440", "#434c5e", "#4c566a", "#616e88", "#353c4a",
            ],
        ),
        spec(
            "Solarized Dark",
            [
                "#002b36", "#93a1a1", "#073642", "#93a1a1", "#073642", "#93a1a1", "#073642",
                "#002b36", "#274642", "#0e4a59", "#586e75", "#06323e",
            ],
        ),
        spec(
            "Gruvbox Dark",
            [
                "#282828", "#ebdbb2", "#1d2021", "#ebdbb2", "#1d2021", "#ebdbb2", "#3c3836",
                "#282828", "#504945", "#3c3836", "#7c6f64", "#32302f",
            ],
        ),
        spec(
            "Tokyo Night",
            [
                "#1a1b26", "#c0caf5", "#16161e", "#a9b1d6", "#16161e", "#a9b1d6", "#16161e",
                "#1a1b26", "#33467c", "#101014", "#3b4261", "#1f2030",
            ],
        ),
        spec(
            "Catppuccin Mocha",
            [
                "#1e1e2e", "#cdd6f4", "#181825", "#cdd6f4", "#181825", "#cdd6f4", "#11111b",
                "#1e1e2e", "#45475a", "#313244", "#6c7086", "#252536",
            ],
        ),
        spec(
            "High Contrast",
            [
                "#000000", "#ffffff", "#000000", "#ffffff", "#000000", "#ffffff", "#000000",
                "#1a1a1a", "#00468c", "#6fc3df", "#ffffff", "#1a1a1a",
            ],
        ),
    ]
}

/// The older three-colour custom theme (`settings.custom`) as a full theme.
fn custom_spec(custom: &nodi_core::CustomTheme) -> ThemeSpec {
    let background = hex_colour(&custom.background, 0x00FF_FFFF);
    let dark = luminance(background) < 0.5;
    let shade = |amount: u8| {
        let colour = if dark {
            lighten(background, amount)
        } else {
            darken(background, amount)
        };
        nodi_core::storage::format_hex_color((
            (colour & 0xFF) as u8,
            ((colour >> 8) & 0xFF) as u8,
            ((colour >> 16) & 0xFF) as u8,
        ))
    };
    ThemeSpec {
        name: "Custom".to_owned(),
        editor_bg: custom.background.clone(),
        editor_text: custom.text.clone(),
        window_bg: shade(10),
        window_text: custom.text.clone(),
        tree_bg: custom.background.clone(),
        tree_text: custom.text.clone(),
        tab_bg: shade(12),
        tab_active: custom.background.clone(),
        accent: custom.accent.clone(),
        border: shade(36),
        gutter_text: shade(90),
        pattern: shade(14),
        ..ThemeSpec::default()
    }
}

/// The theme `settings` selects: a saved theme of the user's, a preset, or
/// (older files) Light / Dark / the three-colour Custom theme.
pub fn active_spec(settings: &Settings) -> ThemeSpec {
    let name = if settings.theme_name.is_empty() {
        match settings.theme {
            Theme::Light => "Light".to_owned(),
            Theme::Dark => "Dark".to_owned(),
            Theme::Custom => return custom_spec(&settings.custom),
        }
    } else {
        settings.theme_name.clone()
    };
    settings
        .my_themes
        .iter()
        .find(|theme| theme.name == name)
        .cloned()
        .or_else(|| presets().into_iter().find(|theme| theme.name == name))
        .unwrap_or_default()
}

thread_local! {
    /// Face and point size of the chrome font of the active theme.
    static UI_FONT: std::cell::RefCell<(String, u32)> =
        std::cell::RefCell::new(("Segoe UI".to_owned(), 9));
}

/// Face and size (points) the tree, tabs and status bar use.
pub fn ui_font() -> (String, u32) {
    UI_FONT.with(|font| font.borrow().clone())
}

fn rgb_of(colour: u32) -> (u8, u8, u8) {
    (
        (colour & 0xFF) as u8,
        ((colour >> 8) & 0xFF) as u8,
        ((colour >> 16) & 0xFF) as u8,
    )
}

fn mix(a: u32, b: u32) -> u32 {
    let (ar, ag, ab) = rgb_of(a);
    let (br, bg, bb) = rgb_of(b);
    rgb(
        ((u16::from(ar) + u16::from(br)) / 2) as u8,
        ((u16::from(ag) + u16::from(bg)) / 2) as u8,
        ((u16::from(ab) + u16::from(bb)) / 2) as u8,
    )
}

impl Palette {
    pub fn from_settings(settings: &Settings) -> Self {
        Self::from_spec(&active_spec(settings))
    }

    pub fn from_spec(spec: &ThemeSpec) -> Self {
        let colour = |text: &str| hex_colour(text, 0);
        let editor_bg = colour(&spec.editor_bg);
        let editor_text = colour(&spec.editor_text);
        let window_bg = colour(&spec.window_bg);
        let dark = luminance(editor_bg) < 0.5;
        // Automatic (and explicitly black) text is shown in the theme's text
        // colour, except on a classic light page with near-black text.
        let plain_light = !dark && luminance(editor_text) < 0.13;
        let text_rgb = rgb_of(editor_text);
        Self {
            dark,
            window_bg,
            window_text: colour(&spec.window_text),
            editor_bg,
            editor_text,
            tree_bg: colour(&spec.tree_bg),
            tree_text: colour(&spec.tree_text),
            tab_bg: colour(&spec.tab_bg),
            tab_active: colour(&spec.tab_active),
            accent: colour(&spec.accent),
            border: colour(&spec.border),
            gutter_bg: mix(editor_bg, window_bg),
            gutter_text: colour(&spec.gutter_text),
            pattern: colour(&spec.pattern),
            auto_sentinel: (!plain_light).then_some(text_rgb),
            black_sentinel: (!plain_light).then_some(nudge(text_rgb)),
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

    /// Cell background for Rich Edit tables: white on light themes, a slightly
    /// raised shade of the page on dark ones (never a glaring white block).
    pub fn table_bg(&self) -> u32 {
        if self.dark {
            lighten(self.editor_bg, 14)
        } else {
            rgb(255, 255, 255)
        }
    }

    /// Table borders: clearly visible on both light and dark pages without
    /// becoming a bright grid.
    pub fn table_border(&self) -> u32 {
        if self.dark {
            lighten(self.editor_bg, 58)
        } else {
            rgb(160, 160, 160)
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::from_settings(&Settings::default())
    }
}

/// RTF for display in `palette` (adds the theme sentinels when needed).
pub fn theme_bytes(palette: &Palette, rtf: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(rtf);
    // The page gap is a picture, so its pixels cannot follow the theme by
    // themselves: repaint every page-break picture in the line-number colour
    // before the RTF reaches Rich Edit (also for notes written by older
    // versions or imported from Markdown).
    let gap = nodi_core::image::page_break_png_coloured(rgb_tuple(palette.gutter_bg));
    let text = nodi_core::rtf::replace_page_breaks(&text, &gap);
    // Formula pictures are grey + alpha masks: repaint them in the text colour
    // so they stay readable in dark and high-contrast themes as well.
    let text = nodi_core::rtf::replace_formula_pictures(&text, rgb_tuple(palette.editor_text));
    match (palette.auto_sentinel, palette.black_sentinel) {
        (Some(auto), Some(black)) => apply_theme_colors(&text, auto, black).into_bytes(),
        _ => text.into_bytes(),
    }
}

/// RTF as it must be stored on disk (sentinels folded back to auto/black).
pub fn fold_bytes(palette: &Palette, rtf: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(rtf);
    // Formulas are stored black and repainted on display, like automatic text.
    let text = nodi_core::rtf::replace_formula_pictures(&text, (0, 0, 0));
    match (palette.auto_sentinel, palette.black_sentinel) {
        (Some(auto), Some(black)) => fold_theme_colors(&text, auto, black).into_bytes(),
        _ => text.into_bytes(),
    }
}

/// Reloads one editor from its stored RTF (`stream_out_rtf` already folded the
/// old palette's display colours) in `new`'s display colours, keeping the
/// caret, selection and first visible line.
fn retint_editor(editor: HWND, stored: &[u8], new: &Palette, zoom: i32) {
    let themed = theme_bytes(new, stored);
    let (from, to) = editor::selection(editor);
    let line = editor::first_visible_line(editor);
    editor::set_change_events(editor, false);
    editor::load_rtf(editor, &themed);
    crate::wnd::table::retheme(editor, new.table_bg(), new.table_border());
    // `EM_STREAMIN` resets the zoom to 100%; restore the user's level so a
    // theme switch does not require Ctrl+0 (or a manual zoom) afterwards.
    editor::set_zoom_percent(editor, zoom);
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
    if old.sentinels() != new.sentinels()
        || old.gutter_bg != new.gutter_bg
        || old.table_bg() != new.table_bg()
        || old.table_border() != new.table_border()
    {
        // Read every editor while the old display colours are installed, so
        // they fold back correctly, then reload in the new ones. The page gap
        // follows the line-number colour, so it is repainted too.
        let stored: Vec<(HWND, Vec<u8>)> = app
            .tabs
            .iter()
            .filter_map(|tab| editor::stream_out_rtf(tab.editor).map(|rtf| (tab.editor, rtf)))
            .collect();
        new.install_display_colours();
        for (window, rtf) in &stored {
            retint_editor(*window, rtf, &new, app.zoom);
        }
    }
    new.install_display_colours();
    app.palette = new;
    set_page_band(new.gutter_bg);

    let spec = active_spec(&app.settings);
    let font = (spec.ui_font.clone(), spec.ui_size);
    if UI_FONT.with(|current| *current.borrow() != font) {
        UI_FONT.with(|current| *current.borrow_mut() = font);
        crate::app::recreate_fonts(app);
        crate::wnd::layout(app);
    }
    set_dark_menus(new.dark);

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
        crate::wnd::pattern::set(app.settings.background, new.pattern, new.editor_bg);
        for tab in &app.tabs {
            SetWindowTheme(tab.editor, sub_app, ptr::null());
            editor::set_background(tab.editor, new.editor_bg);
            InvalidateRect(tab.editor, ptr::null(), 1);
            editor::set_spell_check(tab.editor, app.settings.spell_check);
        }

        // TreeView has built-in colours for its items; the selected row keeps
        // the system highlight, which is readable in both palettes.
        if !app.tree_view.is_null() {
            SendMessageW(app.tree_view, TVM_SETBKCOLOR, 0, new.tree_bg as isize);
            SendMessageW(app.tree_view, TVM_SETTEXTCOLOR, 0, new.tree_text as isize);
        }
        // The menu bar is repainted in the new colours.
        windows_sys::Win32::UI::WindowsAndMessaging::DrawMenuBar(app.hwnd);

        // The sidebar caption is a static control without its own colours.
        if !app.sidebar_title.is_null() {
            InvalidateRect(app.sidebar_title, ptr::null(), 1);
        }
    }

    status::reapply(app);
    tabs::invalidate(app);
    crate::wnd::gutter::refresh(app);

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
        // The standard toolbar bitmaps were made for a light background: their
        // dark outlines vanish on a dark theme. Paint the same buttons as
        // Segoe MDL2 glyphs (and the one text button) in the theme colours.
        let result = match draw.nmcd.dwDrawStage {
            CDDS_PREPAINT => {
                if !draw.nmcd.hdc.is_null() {
                    fill(draw.nmcd.hdc, &draw.nmcd.rc, icon_strip(&app.palette));
                }
                CDRF_NOTIFYITEMDRAW
            }
            CDDS_ITEMPREPAINT => CDRF_NOTIFYPOSTPAINT,
            CDDS_ITEMPOSTPAINT => {
                let command = draw.nmcd.dwItemSpec as u16;
                if let Some(glyph) = toolbar_glyph(command) {
                    draw_toolbar_glyph(app, draw, glyph);
                } else if command == crate::wnd::menu::IDM_VIEW_WORD_WRAP {
                    draw_toolbar_text(app, draw, "Wrap");
                } else if command == 0 {
                    // Separator: a one-pixel rule in the theme's border colour.
                    let mut line = draw.nmcd.rc;
                    line.left = (line.left + line.right) / 2;
                    line.right = line.left + 1;
                    fill(draw.nmcd.hdc, &line, app.palette.border);
                }
                CDRF_DODEFAULT
            }
            _ => CDRF_DODEFAULT,
        };
        return Some(result as LRESULT);
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

/// Segoe MDL2 Assets glyph for a standard toolbar command.
fn toolbar_glyph(command: u16) -> Option<u16> {
    Some(match command {
        crate::wnd::menu::IDM_FILE_NEW => 0xE8A5,  // page
        crate::wnd::menu::IDM_FILE_SAVE => 0xE74E, // floppy
        crate::wnd::menu::IDM_EDIT_CUT => 0xE8C6,  // scissors
        crate::wnd::menu::IDM_EDIT_COPY => 0xE8C8, // two pages
        crate::wnd::menu::IDM_EDIT_PASTE => 0xE77F,
        crate::wnd::menu::IDM_EDIT_UNDO => 0xE7A7,
        crate::wnd::menu::IDM_EDIT_REDO => 0xE7A6,
        crate::wnd::menu::IDM_SEARCH_FIND => 0xE721, // magnifier
        _ => return None,
    })
}

/// Paints one toolbar button as a theme-coloured MDL2 glyph.
unsafe fn draw_toolbar_glyph(app: &App, draw: &NMTBCUSTOMDRAW, glyph: u16) {
    let hdc = draw.nmcd.hdc;
    if hdc.is_null() {
        return;
    }
    let strip = icon_strip(&app.palette);
    let hot = draw.nmcd.uItemState & (CDIS_HOT | CDIS_SELECTED) != 0;
    fill(
        hdc,
        &draw.nmcd.rc,
        if hot { lighten(strip, 26) } else { strip },
    );
    let height = (draw.nmcd.rc.bottom - draw.nmcd.rc.top - 7).clamp(10, 22);
    let face = win::wide("Segoe MDL2 Assets");
    let font = CreateFontW(-height, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr());
    if font.is_null() {
        return;
    }
    let saved = SaveDC(hdc);
    SelectObject(hdc, font as HGDIOBJ);
    SetBkMode(hdc, TRANSPARENT as i32);
    SetTextColor(
        hdc,
        if hot {
            app.palette.accent
        } else {
            app.palette.window_text
        },
    );
    let mut rect = draw.nmcd.rc;
    let text = [glyph];
    DrawTextW(
        hdc,
        text.as_ptr(),
        1,
        &mut rect,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
    );
    RestoreDC(hdc, saved);
    DeleteObject(font as HGDIOBJ);
}

/// Paints a text button (Word Wrap) with the toolbar font and theme colours.
unsafe fn draw_toolbar_text(app: &App, draw: &NMTBCUSTOMDRAW, label: &str) {
    let hdc = draw.nmcd.hdc;
    if hdc.is_null() {
        return;
    }
    let strip = icon_strip(&app.palette);
    let checked = draw.nmcd.uItemState & CDIS_CHECKED != 0;
    let hot = draw.nmcd.uItemState & (CDIS_HOT | CDIS_SELECTED) != 0;
    let background = if hot {
        lighten(strip, 26)
    } else if checked {
        lighten(strip, 18)
    } else {
        strip
    };
    fill(hdc, &draw.nmcd.rc, background);
    if checked {
        // A toggle reads better as a bar in the accent colour than as a
        // full accent fill (the accent can be a light colour in dark themes).
        let mut bar = draw.nmcd.rc;
        bar.top = bar.bottom - 2;
        fill(hdc, &bar, app.palette.accent);
    }
    let font = SendMessageW(draw.nmcd.hdr.hwndFrom, WM_GETFONT, 0, 0);
    let saved = SaveDC(hdc);
    if font != 0 {
        SelectObject(hdc, font as HGDIOBJ);
    }
    SetBkMode(hdc, TRANSPARENT as i32);
    SetTextColor(hdc, app.palette.window_text);
    let mut rect = draw.nmcd.rc;
    let text = win::wide(label);
    DrawTextW(
        hdc,
        text.as_ptr(),
        -1,
        &mut rect,
        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
    );
    RestoreDC(hdc, saved);
}

/// Background of the icon toolbar in a dark theme.
fn icon_strip(palette: &Palette) -> u32 {
    lighten(palette.window_bg, 38)
}

/// Dark pop-up menus through uxtheme's `SetPreferredAppMode` (ordinal 135)
/// and `FlushMenuThemes` (136), the switch Windows' own apps use. Missing on
/// old Windows versions: then the menus simply stay light.
fn set_dark_menus(dark: bool) {
    type SetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
    type FlushMenuThemes = unsafe extern "system" fn();
    let name = win::wide("uxtheme.dll");
    // SAFETY: uxtheme is a system DLL; the ordinals have these signatures on
    // every Windows 10/11 build that exports them, and are checked for null.
    unsafe {
        let module = windows_sys::Win32::System::LibraryLoader::LoadLibraryW(name.as_ptr());
        if module.is_null() {
            return;
        }
        let set = windows_sys::Win32::System::LibraryLoader::GetProcAddress(module, 135 as _);
        let flush = windows_sys::Win32::System::LibraryLoader::GetProcAddress(module, 136 as _);
        if let (Some(set), Some(flush)) = (set, flush) {
            let set: SetPreferredAppMode = std::mem::transmute(set);
            let flush: FlushMenuThemes = std::mem::transmute(flush);
            // 2 = ForceDark, 3 = ForceLight.
            set(if dark { 2 } else { 3 });
            flush();
        }
    }
}

// ------------------------------------------------------------- dark menu bar

/// The default light theme keeps Windows' own menu bar; every other theme
/// paints it in its window colours.
fn paints_menu_bar(palette: &Palette) -> bool {
    palette.dark || palette.window_bg != rgb(240, 240, 240)
}

/// `WM_UAHDRAWMENU`: Windows asks the window to paint the menu bar strip.
pub const WM_UAHDRAWMENU: u32 = 0x0091;
/// `WM_UAHDRAWMENUITEM`: one menu bar item.
pub const WM_UAHDRAWMENUITEM: u32 = 0x0092;

#[repr(C)]
struct UahMenu {
    menu: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
    hdc: HDC,
    flags: u32,
}

#[repr(C)]
struct UahDrawMenuItem {
    dis: windows_sys::Win32::UI::Controls::DRAWITEMSTRUCT,
    um: UahMenu,
    position: i32,
}

/// Paints the menu bar (dark themes only). Returns `None` to let Windows
/// draw it (light themes).
pub unsafe fn on_uah(app: &App, msg: u32, lparam: LPARAM) -> Option<LRESULT> {
    use windows_sys::Win32::Graphics::Gdi::{
        DrawTextW, SetBkMode, SetTextColor, DT_CENTER, DT_HIDEPREFIX, DT_SINGLELINE, DT_VCENTER,
        TRANSPARENT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetMenuBarInfo, GetMenuStringW, GetWindowRect, MENUBARINFO, MF_BYPOSITION, OBJID_MENU,
    };
    if !paints_menu_bar(&app.palette) {
        return None;
    }
    match msg {
        WM_UAHDRAWMENU => {
            let menu = &*(lparam as *const UahMenu);
            let mut info: MENUBARINFO = std::mem::zeroed();
            info.cbSize = size_of::<MENUBARINFO>() as u32;
            if GetMenuBarInfo(app.hwnd, OBJID_MENU, 0, &mut info) == 0 {
                return None;
            }
            let mut window = windows_sys::Win32::Foundation::RECT::default();
            GetWindowRect(app.hwnd, &mut window);
            let mut bar = info.rcBar;
            bar.left -= window.left;
            bar.right -= window.left;
            bar.top -= window.top;
            bar.bottom -= window.top;
            // One pixel more: Windows draws a light line under the bar.
            bar.bottom += 1;
            fill(menu.hdc, &bar, app.palette.window_bg);
            Some(0)
        }
        WM_UAHDRAWMENUITEM => {
            let item = &*(lparam as *const UahDrawMenuItem);
            let mut text = [0u16; 128];
            let length = GetMenuStringW(
                item.um.menu,
                item.position as u32,
                text.as_mut_ptr(),
                text.len() as i32,
                MF_BYPOSITION,
            );
            const ODS_SELECTED: u32 = 0x0001;
            const ODS_GRAYED: u32 = 0x0002;
            const ODS_HOTLIGHT: u32 = 0x0040;
            const ODS_NOACCEL: u32 = 0x0100;
            let state = item.dis.itemState;
            let hot = state & (ODS_SELECTED | ODS_HOTLIGHT) != 0;
            let background = if hot && app.palette.dark {
                lighten(app.palette.window_bg, 28)
            } else if hot {
                darken(app.palette.window_bg, 22)
            } else {
                app.palette.window_bg
            };
            fill(item.dis.hDC, &item.dis.rcItem, background);
            SetBkMode(item.dis.hDC, TRANSPARENT as i32);
            let colour = if state & ODS_GRAYED != 0 {
                app.palette.gutter_text
            } else {
                app.palette.window_text
            };
            SetTextColor(item.dis.hDC, colour);
            let mut rect = item.dis.rcItem;
            let mut flags = DT_CENTER | DT_SINGLELINE | DT_VCENTER;
            if state & ODS_NOACCEL != 0 {
                flags |= DT_HIDEPREFIX;
            }
            DrawTextW(item.dis.hDC, text.as_ptr(), length.max(0), &mut rect, flags);
            Some(0)
        }
        _ => None,
    }
}

/// After the non-client area is painted: covers the light line Windows
/// draws between a dark menu bar and the client area.
pub unsafe fn after_ncpaint(app: &App) {
    use windows_sys::Win32::Graphics::Gdi::{GetWindowDC, ReleaseDC};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetMenuBarInfo, GetWindowRect, MENUBARINFO, OBJID_MENU,
    };
    if !paints_menu_bar(&app.palette) {
        return;
    }
    let mut info: MENUBARINFO = std::mem::zeroed();
    info.cbSize = size_of::<MENUBARINFO>() as u32;
    if GetMenuBarInfo(app.hwnd, OBJID_MENU, 0, &mut info) == 0 {
        return;
    }
    let mut window = windows_sys::Win32::Foundation::RECT::default();
    GetWindowRect(app.hwnd, &mut window);
    let line = windows_sys::Win32::Foundation::RECT {
        left: info.rcBar.left - window.left,
        right: info.rcBar.right - window.left,
        top: info.rcBar.bottom - window.top,
        bottom: info.rcBar.bottom - window.top + 1,
    };
    let dc = GetWindowDC(app.hwnd);
    if !dc.is_null() {
        fill(dc, &line, app.palette.window_bg);
        ReleaseDC(app.hwnd, dc);
    }
}
