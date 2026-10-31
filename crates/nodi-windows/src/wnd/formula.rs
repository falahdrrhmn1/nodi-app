//! Insert > Formula: LaTeX typed in a dialog is laid out by `nodi-core`,
//! drawn with GDI in Cambria Math (ships with Windows) and inserted as a PNG
//! that carries its LaTeX source, so it can be edited again later.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr;

use nodi_core::math::{self, Draw, Layout, Metrics, Style, TextMetrics};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, CreateFontW, CreateSolidBrush, DeleteDC, DeleteObject,
    ExtCreatePen, FillRect, GdiFlush, GetGlyphOutlineW, GetTextExtentPoint32W, Polyline,
    SelectObject, SetBkMode, SetTextAlign, SetTextColor, TextOutW, ANTIALIASED_QUALITY, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DIB_RGB_COLORS, FIXED, FW_BOLD,
    FW_NORMAL, GGO_METRICS, GLYPHMETRICS, HDC, HFONT, HGDIOBJ, LOGBRUSH, MAT2, OUT_TT_PRECIS,
    PS_ENDCAP_ROUND, PS_GEOMETRIC, PS_JOIN_ROUND, PS_SOLID, TA_BASELINE, TA_LEFT, TRANSPARENT,
    VARIABLE_PITCH,
};

use crate::app::App;
use crate::win;
use crate::wnd::form::{Field, Form};
use crate::wnd::{dialogs, editor, image};

/// Pixels per em at 96 DPI (12 pt, a little larger than body text).
const EM_PX: f32 = 16.0;
/// Rendered at twice the display resolution so it stays crisp at 150–200%.
const SCALE: f32 = 2.0;
const FONT: &str = "Cambria Math";

struct GdiMetrics {
    dc: HDC,
    fonts: RefCell<HashMap<(i32, u8), HFONT>>,
}

impl GdiMetrics {
    fn new(dc: HDC) -> Self {
        Self {
            dc,
            fonts: RefCell::new(HashMap::new()),
        }
    }

    fn font(&self, size: f32, style: Style) -> HFONT {
        let pixels = size.round().max(1.0) as i32;
        let key = (pixels, style as u8);
        if let Some(font) = self.fonts.borrow().get(&key) {
            return *font;
        }
        let face = win::wide(FONT);
        // SAFETY: plain font creation; the handle is freed in `Drop`.
        let font = unsafe {
            CreateFontW(
                -pixels,
                0,
                0,
                0,
                if style == Style::Bold {
                    FW_BOLD as i32
                } else {
                    FW_NORMAL as i32
                },
                u32::from(style == Style::Italic),
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_TT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                ANTIALIASED_QUALITY as u32,
                VARIABLE_PITCH as u32,
                face.as_ptr(),
            )
        };
        self.fonts.borrow_mut().insert(key, font);
        font
    }
}

impl Drop for GdiMetrics {
    fn drop(&mut self) {
        for font in self.fonts.borrow().values() {
            // SAFETY: fonts we created; none is selected any more (the DC is
            // reset before the metrics are dropped).
            unsafe {
                DeleteObject(*font as HGDIOBJ);
            }
        }
    }
}

impl Metrics for GdiMetrics {
    fn measure(&self, text: &str, size: f32, style: Style) -> TextMetrics {
        let font = self.font(size, style);
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut extent = SIZE { cx: 0, cy: 0 };
        let identity = MAT2 {
            eM11: windows_sys::Win32::Graphics::Gdi::FIXED { fract: 0, value: 1 },
            eM12: FIXED { fract: 0, value: 0 },
            eM21: FIXED { fract: 0, value: 0 },
            eM22: FIXED { fract: 0, value: 1 },
        };
        let mut ascent = 0i32;
        let mut descent = 0i32;
        // SAFETY: the DC and font are ours; buffers outlive the calls.
        unsafe {
            SelectObject(self.dc, font as HGDIOBJ);
            GetTextExtentPoint32W(self.dc, wide.as_ptr(), wide.len() as i32, &mut extent);
            for unit in &wide {
                if char::from_u32(u32::from(*unit)).is_some_and(char::is_whitespace) {
                    continue;
                }
                let mut glyph: GLYPHMETRICS = std::mem::zeroed();
                let result = GetGlyphOutlineW(
                    self.dc,
                    u32::from(*unit),
                    GGO_METRICS,
                    &mut glyph,
                    0,
                    ptr::null_mut(),
                    &identity,
                );
                if result == u32::MAX {
                    continue;
                }
                ascent = ascent.max(glyph.gmptGlyphOrigin.y);
                descent = descent.max(glyph.gmBlackBoxY as i32 - glyph.gmptGlyphOrigin.y);
            }
        }
        TextMetrics {
            width: extent.cx as f32,
            ascent: ascent.max(0) as f32,
            descent: descent.max(0) as f32,
        }
    }
}

/// A rendered formula ready for `EM_INSERTIMAGE`.
pub struct Rendered {
    pub png: Vec<u8>,
    /// Display size and ascent in HIMETRIC (0.01 mm).
    pub width: i32,
    pub height: i32,
    pub ascent: i32,
}

/// Parses, lays out and renders `latex` in the default (black) colour.
#[allow(dead_code)]
pub fn render(latex: &str) -> Result<Rendered, String> {
    render_coloured(latex, 0)
}

/// Parses, lays out and renders `latex` in `colour` (COLORREF `0x00bbggrr`).
pub fn render_coloured(latex: &str, colour: u32) -> Result<Rendered, String> {
    let tree = math::parse(latex).map_err(|error| error.to_string())?;
    // SAFETY: a private memory DC; every GDI object created here is released
    // before returning.
    unsafe {
        let dc = CreateCompatibleDC(ptr::null_mut());
        if dc.is_null() {
            return Err("Windows could not create a drawing surface.".to_owned());
        }
        let metrics = GdiMetrics::new(dc);
        let laid = math::layout(&tree, &metrics, EM_PX * SCALE);
        let result = draw(dc, &metrics, &laid, latex, colour);
        drop(metrics);
        DeleteDC(dc);
        result
    }
}

unsafe fn draw(
    dc: HDC,
    metrics: &GdiMetrics,
    laid: &Layout,
    latex: &str,
    colour: u32,
) -> Result<Rendered, String> {
    let pad = (3.0 * SCALE).ceil();
    let width = (laid.width + pad * 2.0).ceil().max(1.0) as i32;
    let height = (laid.ascent + laid.descent + pad * 2.0).ceil().max(1.0) as i32;
    if width > 8000 || height > 4000 {
        return Err("The formula is too large to draw.".to_owned());
    }

    let mut info: BITMAPINFO = std::mem::zeroed();
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: width,
        biHeight: -height, // top-down rows
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..std::mem::zeroed()
    };
    let mut bits: *mut core::ffi::c_void = ptr::null_mut();
    let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0);
    if bitmap.is_null() || bits.is_null() {
        return Err("Not enough memory to draw the formula.".to_owned());
    }
    let previous_bitmap = SelectObject(dc, bitmap as HGDIOBJ);

    let pixel_count = (width * height) as usize;
    std::ptr::write_bytes(bits as *mut u8, 0xFF, pixel_count * 4); // white
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, 0);
    SetTextAlign(dc, TA_BASELINE | TA_LEFT);
    let black = CreateSolidBrush(0);

    let origin_x = pad;
    let baseline = pad + laid.ascent;
    for item in &laid.items {
        match &item.draw {
            Draw::Text { text, size, style } => {
                let font = metrics.font(*size, *style);
                SelectObject(dc, font as HGDIOBJ);
                let wide: Vec<u16> = text.encode_utf16().collect();
                TextOutW(
                    dc,
                    (origin_x + item.x).round() as i32,
                    (baseline + item.y).round() as i32,
                    wide.as_ptr(),
                    wide.len() as i32,
                );
            }
            Draw::Rule { width, height } => {
                let left = (origin_x + item.x).round() as i32;
                let top = (baseline + item.y).round() as i32;
                let rect = RECT {
                    left,
                    top,
                    right: left + width.round().max(1.0) as i32,
                    bottom: top + height.round().max(1.0) as i32,
                };
                FillRect(dc, &rect, black);
            }
            Draw::Line { points, thickness } => {
                let brush = LOGBRUSH {
                    lbStyle: 0, // BS_SOLID
                    lbColor: 0,
                    lbHatch: 0,
                };
                let pen = ExtCreatePen(
                    (PS_GEOMETRIC | PS_SOLID | PS_ENDCAP_ROUND | PS_JOIN_ROUND) as u32,
                    thickness.round().max(1.0) as u32,
                    &brush,
                    0,
                    ptr::null(),
                );
                let old_pen = SelectObject(dc, pen as HGDIOBJ);
                let gdi_points: Vec<POINT> = points
                    .iter()
                    .map(|(x, y)| POINT {
                        x: (origin_x + x).round() as i32,
                        y: (baseline + y).round() as i32,
                    })
                    .collect();
                Polyline(dc, gdi_points.as_ptr(), gdi_points.len() as i32);
                SelectObject(dc, old_pen);
                DeleteObject(pen as HGDIOBJ);
            }
        }
    }
    GdiFlush();

    // White background + black glyphs → a coverage mask, then paint the glyphs
    // in the theme's text colour with the same alpha (transparent background).
    let pixels = std::slice::from_raw_parts(bits as *const u8, pixel_count * 4);
    let red = (colour & 0xFF) as u8;
    let green = ((colour >> 8) & 0xFF) as u8;
    let blue = ((colour >> 16) & 0xFF) as u8;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for pixel in pixels.as_chunks::<4>().0 {
        let lightness = (u32::from(pixel[0]) + u32::from(pixel[1]) + u32::from(pixel[2])) / 3;
        rgba.extend_from_slice(&[red, green, blue, 255 - lightness as u8]);
    }

    SelectObject(dc, previous_bitmap);
    DeleteObject(bitmap as HGDIOBJ);
    DeleteObject(black as HGDIOBJ);

    let png = math::encode_png_rgba(width as u32, height as u32, &rgba, latex);
    let himetric = |device: f32| ((device / SCALE) * 2540.0 / 96.0).round() as i32;
    Ok(Rendered {
        png,
        width: himetric(width as f32),
        height: himetric(height as f32),
        ascent: himetric(baseline),
    })
}

/// A formula in the document: `start..end` covers its hidden link
/// instruction and the picture.
pub struct Found {
    pub start: i32,
    pub end: i32,
    pub latex: String,
}

/// The formula at or next to `pos`. In the raw text a formula reads
/// `HYPERLINK "nodi-math:…"` (hidden field instruction) followed by U+FFFC.
pub fn formula_at(editor: HWND, pos: i32) -> Option<Found> {
    let raw = editor::raw_text(editor);
    let pos = (pos.max(0) as usize).min(raw.len());
    let mut candidates: Vec<usize> = Vec::new();
    if pos > 0 && raw[pos - 1] == 0xFFFC {
        candidates.push(pos - 1);
    }
    if let Some(offset) = raw[pos..]
        .iter()
        .take(8192)
        .position(|unit| *unit == 0xFFFC)
    {
        candidates.push(pos + offset);
    }
    for picture in candidates {
        if let Some((start, latex)) = field_before(&raw, picture) {
            if start <= pos && pos <= picture + 1 {
                return Some(Found {
                    start: start as i32,
                    end: picture as i32 + 1,
                    latex,
                });
            }
        }
    }
    None
}

/// Start of the `HYPERLINK "nodi-math:…"` instruction that ends right before
/// `picture`, plus the decoded LaTeX.
fn field_before(raw: &[u16], picture: usize) -> Option<(usize, String)> {
    if picture == 0 || raw[picture - 1] != u16::from(b'"') {
        return None;
    }
    let window_start = picture.saturating_sub(16384);
    let window = &raw[window_start..picture - 1];
    let needle: Vec<u16> = "HYPERLINK \"".encode_utf16().collect();
    let found = window
        .windows(needle.len())
        .rposition(|candidate| candidate == needle.as_slice())?;
    let target_start = found + needle.len();
    let target = String::from_utf16_lossy(&window[target_start..]);
    if target.contains('"') {
        return None;
    }
    let latex = nodi_core::links::parse_math_link(&target)?;
    Some((window_start + found, latex))
}

/// LaTeX of the formula at the selection, if any.
pub fn selected_latex(editor: HWND) -> Option<String> {
    let (from, _) = editor::selection(editor);
    formula_at(editor, from).map(|found| found.latex)
}

/// Inserts a rendered formula at the selection (replacing it) as a picture
/// wrapped in a `nodi-math:` hyperlink that carries the LaTeX.
pub fn insert_rendered(editor: HWND, rendered: &Rendered, latex: &str) -> Result<(), String> {
    let (at, _) = editor::selection(editor);
    image::insert_sized(
        editor,
        &rendered.png,
        rendered.width,
        rendered.height,
        rendered.ascent,
        "formula",
    )?;
    // Re-read the picture as Rich Edit stored it and wrap it in the link.
    editor::set_selection(editor, at, at + 1);
    let picture = editor::selection_rtf(editor)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .and_then(|rtf| nodi_core::rtf::picture_group(&rtf).map(str::to_owned))
        .ok_or("The formula picture could not be read back.")?;
    let wrapped = format!(
        "{{\\rtf1{{\\field{{\\*\\fldinst{{HYPERLINK \"{}\"}}}}{{\\fldrslt{{{}}}}}}}}}",
        nodi_core::links::format_math_link(latex),
        picture
    );
    if !editor::insert_rtf(editor, wrapped.as_bytes()) {
        return Err("Rich Edit refused the formula.".to_owned());
    }
    // Continue typing after the formula.
    let after = formula_at(editor, at).map_or(at + 1, |found| found.end);
    editor::set_selection(editor, after, after);
    Ok(())
}

/// Replaces every formula placeholder a Markdown/HTML paste left behind
/// (`markdown_in::FORMULA_START` … `FORMULA_END`) with a rendered formula.
/// LaTeX that cannot be rendered stays as inline-code text. Returns how many
/// were rendered and the LaTeX of those kept as text.
pub fn render_pasted(editor: HWND, colour: u32) -> (usize, Vec<String>) {
    let start_mark = nodi_core::markdown_in::FORMULA_START as u16;
    let end_mark = nodi_core::markdown_in::FORMULA_END as u16;
    let mut rendered_count = 0;
    let mut kept = Vec::new();
    let mut caret = editor::selection(editor).1;
    loop {
        let raw = editor::raw_text(editor);
        let Some(start) = raw.iter().position(|unit| *unit == start_mark) else {
            break;
        };
        let before = editor::text_length(editor);
        let Some(length) = raw[start..].iter().position(|unit| *unit == end_mark) else {
            // An unterminated marker: drop it so the loop ends.
            editor::set_selection(editor, start as i32, start as i32 + 1);
            editor::replace_selection(editor, "", true);
            caret += editor::text_length(editor) - before;
            continue;
        };
        let end = start + length;
        let latex = String::from_utf16_lossy(&raw[start + 1..end]);
        let (from, to) = (start as i32, end as i32 + 1);
        editor::set_selection(editor, from, to);
        let done = render_coloured(&latex, colour)
            .and_then(|picture| insert_rendered(editor, &picture, &latex))
            .is_ok();
        if done {
            rendered_count += 1;
        } else {
            // Rendering failed before anything changed: put the LaTeX back
            // as code text in place of the placeholder.
            editor::set_selection(editor, from, to);
            editor::replace_selection(editor, &latex, true);
            let length = latex.encode_utf16().count() as i32;
            editor::set_selection(editor, from, from + length);
            crate::wnd::format::toggle_inline_code(editor);
            kept.push(latex);
        }
        caret += editor::text_length(editor) - before;
    }
    editor::set_selection(editor, caret, caret);
    (rendered_count, kept)
}

/// Insert > Formula... (Alt+=): new formula, or edit the selected one.
pub fn insert_or_edit(app: &mut App) {
    if crate::wnd::editor::refuse_if_locked(app) {
        return;
    }
    let Some(editor) = app.active_editor() else {
        return;
    };
    let (from, to) = editor::selection(editor);
    let found = formula_at(editor, from);
    let existing = found.as_ref().map(|found| found.latex.clone());
    let colour = app.palette.editor_text;
    let mut latex = existing.clone().unwrap_or_default();
    loop {
        let title = if existing.is_some() {
            "Edit Formula"
        } else {
            "Insert Formula"
        };
        let Some(values) = Form::new(title)
            .note(
                "Type LaTeX, e.g.  E = mc^2    \\frac{a}{b}    \\sqrt{x}    x_i^2\n\
                 \\sum_{i=1}^{n} x_i    \\int_0^\\infty e^{-x}\\,dx    \\hat{y}    \\alpha \\leq \\beta\n\
                 \\begin{pmatrix} a & b \\\\ c & d \\end{pmatrix}    \\begin{cases} 1 & x>0 \\\\ 0 & \\text{else} \\end{cases}",
            )
            .field(Field::multiline("LaTeX:", latex.clone(), 5))
            .show(app.hwnd)
        else {
            return;
        };
        latex = values[0].trim().replace("\r\n", "\n");
        if latex.is_empty() {
            return;
        }
        match render_coloured(&latex, colour) {
            Ok(rendered) => {
                // Editing replaces the whole old formula (link + picture).
                match &found {
                    Some(old) => editor::set_selection(editor, old.start, old.end),
                    None => editor::set_selection(editor, from, to),
                }
                match insert_rendered(editor, &rendered, &latex) {
                    Ok(()) => app.mark_dirty(editor),
                    Err(message) => dialogs::error(app.hwnd, &message),
                }
                return;
            }
            Err(message) => {
                dialogs::error(
                    app.hwnd,
                    &format!("The formula could not be read:\n{message}"),
                );
            }
        }
    }
}
