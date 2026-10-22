//! Pictures inside a note: insert from a file or from the clipboard.
//!
//! Images are inserted with `EM_INSERTIMAGE` (Rich Edit 8, Windows 8+) and
//! live inside the note's RTF, so they are saved, backed up and restored with
//! the text itself; no path to the original file is kept.

use std::ffi::c_void;
use std::mem::size_of;

use nodi_core::{image, rtf};
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC, LOGPIXELSX};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW,
};
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
    OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::SHCreateMemStream;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, SendMessageW};

use crate::win;
use crate::wnd::editor;
use crate::wnd::form::{Field, Form};

const EM_INSERTIMAGE: u32 = 0x0400 + 314;
/// `TA_BASELINE`: the picture sits on the text baseline.
const TA_BASELINE: i32 = 24;
const CF_UNICODETEXT: u32 = 13;
const CF_DIB: u32 = 8;

/// `RICHEDIT_IMAGE_PARAMETERS` (richedit.h, `pack(4)`).
#[repr(C, packed(4))]
struct ImageParameters {
    x_width: i32,
    y_height: i32,
    ascent: i32,
    kind: i32,
    alternate_text: *const u16,
    stream: *mut c_void,
}

const _: () = assert!(size_of::<ImageParameters>() == 32);

/// Inserts an encoded PNG/JPEG/GIF/BMP at the selection, scaled down to fit
/// the editor width.
pub fn insert_bytes(editor: HWND, bytes: &[u8]) -> Result<(), String> {
    let info = image::sniff(bytes).ok_or("This file is not a PNG, JPEG, GIF or BMP image.")?;
    let (width, height) =
        image::display_size_himetric(info.width, info.height, max_width_px(editor));
    insert_sized(editor, bytes, width, height, height, "image")
}

/// Inserts an encoded picture shown at `width`×`height` HIMETRIC, with its
/// top `ascent` HIMETRIC above the text baseline.
pub fn insert_sized(
    editor: HWND,
    bytes: &[u8],
    width: i32,
    height: i32,
    ascent: i32,
    alt: &str,
) -> Result<(), String> {
    if bytes.len() > u32::MAX as usize {
        return Err("The image is too large.".to_owned());
    }
    let (from, _) = editor::selection(editor);

    // SAFETY: `bytes` is valid for its length; the stream copies it.
    let stream = unsafe { SHCreateMemStream(bytes.as_ptr(), bytes.len() as u32) };
    if stream.is_null() {
        return Err("Not enough memory for the image.".to_owned());
    }
    let alt = win::wide(alt);
    let parameters = ImageParameters {
        x_width: width,
        y_height: height,
        ascent,
        kind: TA_BASELINE,
        alternate_text: alt.as_ptr(),
        stream,
    };
    // SAFETY: `parameters`, `alt` and the stream outlive the synchronous call.
    let result = unsafe {
        SendMessageW(
            editor,
            EM_INSERTIMAGE,
            0,
            &parameters as *const ImageParameters as isize,
        )
    };
    // SAFETY: we own one reference to the stream; Rich Edit took its own.
    unsafe { release(stream) };
    if result == 0 {
        // Rich Edit leaves the caret in front of the new picture; typing
        // should continue after it.
        editor::set_selection(editor, from + 1, from + 1);
        Ok(())
    } else {
        Err(format!(
            "Rich Edit could not insert the image (HRESULT 0x{:08X}).",
            result as u32
        ))
    }
}

/// Selects the picture under/next to the caret and returns its RTF. A
/// picture is one character, so it is either the selection itself or the
/// character after (or before) the caret.
pub fn pick_picture(editor: HWND) -> Option<String> {
    let (from, to) = editor::selection(editor);
    let candidates: &[(i32, i32)] = if to - from == 1 {
        &[(from, to)]
    } else if from == to {
        &[(from, from + 1), (from - 1, from)]
    } else {
        &[]
    };
    for &(start, end) in candidates {
        if start < 0 {
            continue;
        }
        editor::set_selection(editor, start, end);
        if let Some(rtf) = editor::selection_rtf(editor) {
            let text = String::from_utf8_lossy(&rtf).into_owned();
            if rtf::has_picture(&text) {
                return Some(text);
            }
        }
    }
    editor::set_selection(editor, from, to);
    None
}

/// Natural size of the picture in pixels (from its encoded header).
fn natural_size(picture_rtf: &str) -> Option<(u32, u32)> {
    let blob = rtf::picture_blob(picture_rtf)?;
    let info = image::sniff(&blob)?;
    Some((info.width, info.height))
}

/// Replaces the selected picture (from [`pick_picture`]) with the same
/// picture shown at `width`×`height` twips, and keeps it selected.
fn apply_size(editor: HWND, picture_rtf: &str, width: i32, height: i32) -> Result<(), String> {
    let patched = rtf::set_picture_goal(picture_rtf, width.max(30), height.max(30))
        .ok_or("The picture data could not be read.")?;
    let (from, _) = editor::selection(editor);
    if !editor::insert_rtf(editor, patched.as_bytes()) {
        return Err("Rich Edit refused the resized picture.".to_owned());
    }
    editor::set_selection(editor, from, from + 1);
    Ok(())
}

/// Image > Size presets: percent of the picture's natural size.
pub fn resize_percent(editor: HWND, percent: i32) -> Result<(), String> {
    let picture = pick_picture(editor).ok_or("Select a picture first.")?;
    let (width, height) = natural_size(&picture).ok_or("Unknown picture format.")?;
    let twips = |px: u32| (px as i64 * 15 * percent as i64 / 100) as i32;
    apply_size(editor, &picture, twips(width), twips(height))
}

/// Image > Fit to Width: as wide as the editor, aspect ratio kept.
pub fn fit_width(editor: HWND) -> Result<(), String> {
    let picture = pick_picture(editor).ok_or("Select a picture first.")?;
    let (width, height) = rtf::picture_goal(&picture).ok_or("Unknown picture size.")?;
    let target = crate::wnd::format::full_width_twips(editor);
    let new_height = (height as i64 * target as i64 / width.max(1) as i64) as i32;
    apply_size(editor, &picture, target, new_height)
}

/// Image > Size...: width/height in pixels or percent, with "Lock aspect
/// ratio" on by default.
pub fn resize_dialog(editor: HWND, owner: HWND) -> Result<(), String> {
    let picture = pick_picture(editor).ok_or("Select a picture first (click it).")?;
    let (goal_w, goal_h) = rtf::picture_goal(&picture).ok_or("Unknown picture size.")?;
    let natural = natural_size(&picture);
    let note = match natural {
        Some((w, h)) => format!(
            "Original size: {w} × {h} px. Current: {} × {} px.\n\
             Leave both fields of a unit empty to keep the current size.",
            goal_w / 15,
            goal_h / 15
        ),
        None => "Leave both fields of a unit empty to keep the current size.".to_owned(),
    };
    let original = natural.unwrap_or(((goal_w / 15).max(1) as u32, (goal_h / 15).max(1) as u32));
    let form = Form::new("Image Size")
        .note(&note)
        .field(Field::new("Width (px):", (goal_w / 15).to_string()))
        .field(Field::new("Height (px):", (goal_h / 15).to_string()))
        .field(Field::new("Width (%):", ""))
        .field(Field::new("Height (%):", ""))
        .checkbox("Lock aspect ratio", true)
        .extra(
            "Original size",
            Box::new(move |_, values| {
                values[0] = original.0.to_string();
                values[1] = original.1.to_string();
                values[2].clear();
                values[3].clear();
            }),
        )
        .extra(
            "Half size",
            Box::new(move |_, values| {
                values[2] = "50".to_owned();
                values[3] = "50".to_owned();
            }),
        );
    let Some(values) = form.show(owner) else {
        return Ok(());
    };
    let parse = |text: &str| text.trim().parse::<i32>().ok().filter(|value| *value > 0);
    let locked = values.get(4).is_none_or(|value| value == "1");
    // The aspect ratio follows the original pixels, falling back to the
    // current display size for formats without a usable header.
    let aspect_w = original.0.max(1) as i64;
    let aspect_h = original.1.max(1) as i64;
    let derive_height = |width: i64| (width * aspect_h / aspect_w).max(1) as i32;
    let derive_width = |height: i64| (height * aspect_w / aspect_h).max(1) as i32;

    let (percent_w, percent_h) = (parse(&values[2]), parse(&values[3]));
    let (mut width, mut height) = if percent_w.is_some() || percent_h.is_some() {
        let percent = percent_w.or(percent_h).unwrap_or(100) as i64;
        if locked {
            let width = (aspect_w * percent / 100).max(1) as i32;
            (width, derive_height(i64::from(width)))
        } else {
            let width = percent_w
                .map(|value| (aspect_w * i64::from(value) / 100).max(1) as i32)
                .unwrap_or(goal_w / 15);
            let height = percent_h
                .map(|value| (aspect_h * i64::from(value) / 100).max(1) as i32)
                .unwrap_or(goal_h / 15);
            (width, height)
        }
    } else {
        match (parse(&values[0]), parse(&values[1])) {
            (Some(width), Some(height)) => {
                if locked {
                    (width, derive_height(i64::from(width)))
                } else {
                    (width, height)
                }
            }
            (Some(width), None) if locked => (width, derive_height(i64::from(width))),
            (Some(width), None) => (width, goal_h / 15),
            (None, Some(height)) if locked => (derive_width(i64::from(height)), height),
            (None, Some(height)) => (goal_w / 15, height),
            (None, None) => {
                return Err("Enter a width or a height in pixels or percent.".to_owned())
            }
        }
    };
    if width <= 0 || height <= 0 {
        return Err("The new size must be positive.".to_owned());
    }
    // The aspect-lock path already derives the other side; honour it for the
    // unlocked path by keeping zero values out of the RTF.
    width = width.max(1);
    height = height.max(1);
    apply_size(editor, &picture, width * 15, height * 15)
}

/// Image > Crop...: trims pixels from each side with RTF `\piccrop*`.
pub fn crop_dialog(editor: HWND, owner: HWND) -> Result<(), String> {
    let picture = pick_picture(editor).ok_or("Select a picture first (click it).")?;
    let (goal_w, goal_h) = rtf::picture_goal(&picture).ok_or("Unknown picture size.")?;
    let (natural_w, natural_h) = natural_size(&picture)
        .map(|(w, h)| (w.max(1), h.max(1)))
        .unwrap_or(((goal_w / 15).max(1) as u32, (goal_h / 15).max(1) as u32));
    let crop = rtf::picture_crop(&picture).unwrap_or((0, 0, 0, 0));
    let note = format!(
        "Trim pixels from each side of the {natural_w} × {natural_h} px picture.\n\
         The display size stays the same; use Image > Size to change it."
    );
    let form = Form::new("Crop Image")
        .note(&note)
        .field(Field::new("Left (px):", (crop.0 / 15).to_string()))
        .field(Field::new("Top (px):", (crop.1 / 15).to_string()))
        .field(Field::new("Right (px):", (crop.2 / 15).to_string()))
        .field(Field::new("Bottom (px):", (crop.3 / 15).to_string()))
        .extra(
            "Reset crop",
            Box::new(|_, values| {
                for value in values.iter_mut() {
                    *value = "0".to_owned();
                }
            }),
        );
    let Some(values) = form.show(owner) else {
        return Ok(());
    };
    let parse = |text: &str| -> Option<i32> {
        let text = text.trim();
        if text.is_empty() {
            Some(0)
        } else {
            text.parse::<i32>().ok().filter(|value| *value >= 0)
        }
    };
    let parse_all = || -> Option<(i32, i32, i32, i32)> {
        Some((
            parse(&values[0])?,
            parse(&values[1])?,
            parse(&values[2])?,
            parse(&values[3])?,
        ))
    };
    let (left, top, right, bottom) =
        parse_all().ok_or("Enter whole, non-negative pixel amounts.")?;
    if left as u32 + right as u32 >= natural_w || top as u32 + bottom as u32 >= natural_h {
        return Err(format!(
            "That crop would remove the whole picture ({natural_w} × {natural_h} px)."
        ));
    }
    let patched = rtf::set_picture_crop(&picture, left * 15, top * 15, right * 15, bottom * 15)
        .ok_or("The picture data could not be read.")?;
    let (from, _) = editor::selection(editor);
    if !editor::insert_rtf(editor, patched.as_bytes()) {
        return Err("Rich Edit refused the cropped picture.".to_owned());
    }
    editor::set_selection(editor, from, from + 1);
    Ok(())
}

/// Applies a crop in pixels to the picture at the selection (test hook and
/// the dialog share this path).
pub fn crop_selection(
    editor: HWND,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) -> Result<(), String> {
    let picture = pick_picture(editor).ok_or("Select a picture first.")?;
    let (goal_w, goal_h) = rtf::picture_goal(&picture).ok_or("Unknown picture size.")?;
    let (natural_w, natural_h) = natural_size(&picture)
        .map(|(w, h)| (w.max(1), h.max(1)))
        .unwrap_or(((goal_w / 15).max(1) as u32, (goal_h / 15).max(1) as u32));
    let (left, top, right, bottom) = (
        left.max(0) as u32,
        top.max(0) as u32,
        right.max(0) as u32,
        bottom.max(0) as u32,
    );
    if left + right >= natural_w || top + bottom >= natural_h {
        return Err("Crop is larger than the picture.".to_owned());
    }
    let patched = rtf::set_picture_crop(
        &picture,
        left as i32 * 15,
        top as i32 * 15,
        right as i32 * 15,
        bottom as i32 * 15,
    )
    .ok_or("The picture data could not be read.")?;
    let (from, _) = editor::selection(editor);
    if !editor::insert_rtf(editor, patched.as_bytes()) {
        return Err("Rich Edit refused the cropped picture.".to_owned());
    }
    editor::set_selection(editor, from, from + 1);
    Ok(())
}

/// Image > Save Image As...: writes the original encoded bytes.
pub fn save_as(editor: HWND, owner: HWND) -> Result<(), String> {
    let picture = pick_picture(editor).ok_or("Select a picture first.")?;
    let blob = rtf::picture_blob(&picture).ok_or("The picture data could not be read.")?;
    let extension = match image::sniff(&blob).map(|info| info.format) {
        Some(image::ImageFormat::Jpeg) => "jpg",
        Some(image::ImageFormat::Gif) => "gif",
        Some(image::ImageFormat::Bmp) => "bmp",
        _ => "png",
    };
    let Some(path) = save_dialog(owner, extension) else {
        return Ok(());
    };
    std::fs::write(&path, blob).map_err(|err| format!("Could not write {path}:\n{err}"))
}

fn save_dialog(owner: HWND, extension: &str) -> Option<String> {
    let filter: Vec<u16> =
        format!("Image (*.{extension})\0*.{extension}\0All files (*.*)\0*.*\0\0")
            .encode_utf16()
            .collect();
    let mut buffer = vec![0u16; 4096];
    for (slot, unit) in buffer
        .iter_mut()
        .zip(format!("image.{extension}").encode_utf16())
    {
        *slot = unit;
    }
    let default_extension = win::wide(extension);
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = buffer.as_mut_ptr();
    dialog.nMaxFile = buffer.len() as u32;
    dialog.lpstrDefExt = default_extension.as_ptr();
    dialog.Flags = OFN_EXPLORER | OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    // SAFETY: every pointer refers to buffers that outlive the call.
    if unsafe { GetSaveFileNameW(&mut dialog) } == 0 {
        return None;
    }
    let length = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
    Some(String::from_utf16_lossy(&buffer[..length]))
}

/// `IUnknown::Release` through the vtable (`windows-sys` has no COM wrappers).
///
/// # Safety
/// `object` must be a live COM interface pointer we own a reference to.
pub unsafe fn release(object: *mut c_void) {
    if object.is_null() {
        return;
    }
    let vtable = *(object as *const *const usize);
    let release: unsafe extern "system" fn(*mut c_void) -> u32 =
        std::mem::transmute(*vtable.add(2));
    release(object);
}

/// Widest picture (in 96-DPI pixels) that fits the editor without scrolling.
fn max_width_px(editor: HWND) -> u32 {
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    // Rich Edit maps HIMETRIC with its DC's DPI, so measure with that.
    // SAFETY: `rect` is a valid out parameter; the DC is released at once.
    let (ok, dpi) = unsafe {
        let ok = GetClientRect(editor, &mut rect);
        let dc = GetDC(editor);
        let dpi = if dc.is_null() {
            GetDpiForWindow(editor) as i32
        } else {
            let value = GetDeviceCaps(dc, LOGPIXELSX as i32);
            ReleaseDC(editor, dc);
            value
        };
        (ok, dpi)
    };
    if ok == 0 || rect.right <= 0 {
        return 600;
    }
    let dpi = if dpi <= 0 { 96 } else { dpi };
    let px = win::mul_div(rect.right, 96, dpi) - 60;
    px.clamp(64, 4000) as u32
}

/// `Insert > Image...`: asks for a file and returns its bytes.
pub fn choose_file(owner: HWND) -> Option<(String, Vec<u8>)> {
    let filter: Vec<u16> =
        "Images (*.png;*.jpg;*.jpeg;*.gif;*.bmp)\0*.png;*.jpg;*.jpeg;*.gif;*.bmp\0All files (*.*)\0*.*\0\0"
            .encode_utf16()
            .collect();
    let mut buffer = vec![0u16; 4096];
    let title = win::wide("Insert Image");
    let mut dialog: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    dialog.hwndOwner = owner;
    dialog.lpstrFilter = filter.as_ptr();
    dialog.lpstrFile = buffer.as_mut_ptr();
    dialog.nMaxFile = buffer.len() as u32;
    dialog.lpstrTitle = title.as_ptr();
    dialog.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    // SAFETY: every pointer in `dialog` refers to buffers that outlive the call.
    if unsafe { GetOpenFileNameW(&mut dialog) } == 0 {
        return None;
    }
    let length = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
    let path = String::from_utf16_lossy(&buffer[..length]);
    std::fs::read(&path).ok().map(|bytes| (path, bytes))
}

/// True when the clipboard holds a picture but no text: pasting it needs
/// [`clipboard_image`], because Rich Edit itself only pastes text/RTF here.
pub fn clipboard_is_image_only() -> bool {
    // SAFETY: the format queries take no pointers.
    unsafe {
        let has_text = IsClipboardFormatAvailable(CF_UNICODETEXT) != 0
            || IsClipboardFormatAvailable(rtf_format()) != 0;
        let has_image = IsClipboardFormatAvailable(png_format()) != 0
            || IsClipboardFormatAvailable(CF_DIB) != 0;
        has_image && !has_text
    }
}

/// The clipboard picture as PNG (preferred) or BMP bytes.
pub fn clipboard_image(owner: HWND) -> Option<Vec<u8>> {
    // SAFETY: the clipboard is opened and closed in this block; global memory
    // is only read while locked.
    unsafe {
        if OpenClipboard(owner) == 0 {
            return None;
        }
        let png = png_format();
        let bytes = if IsClipboardFormatAvailable(png) != 0 {
            global_bytes(GetClipboardData(png))
        } else if IsClipboardFormatAvailable(CF_DIB) != 0 {
            global_bytes(GetClipboardData(CF_DIB)).and_then(|dib| image::dib_to_bmp(&dib))
        } else {
            None
        };
        CloseClipboard();
        bytes
    }
}

/// True when the clipboard holds Rich Text Format (Rich Edit pastes it
/// natively, keeping the source formatting).
pub fn clipboard_has_rtf() -> bool {
    // SAFETY: the format queries take no pointers.
    unsafe { IsClipboardFormatAvailable(rtf_format()) != 0 }
}

/// The clipboard's CF_HTML bytes, when the source offered HTML.
pub fn clipboard_html(owner: HWND) -> Option<Vec<u8>> {
    // SAFETY: the clipboard is opened and closed here; global memory is only
    // read while locked.
    unsafe {
        let format = html_format();
        if IsClipboardFormatAvailable(format) == 0 || OpenClipboard(owner) == 0 {
            return None;
        }
        let bytes = global_bytes(GetClipboardData(format));
        CloseClipboard();
        bytes.map(|mut bytes| {
            while bytes.last() == Some(&0) {
                bytes.pop();
            }
            bytes
        })
    }
}

/// Clipboard text only (for `Paste as Plain Text`).
pub fn clipboard_text(owner: HWND) -> Option<String> {
    // SAFETY: as in `clipboard_image`.
    unsafe {
        if OpenClipboard(owner) == 0 {
            return None;
        }
        let text = if IsClipboardFormatAvailable(CF_UNICODETEXT) != 0 {
            global_bytes(GetClipboardData(CF_UNICODETEXT)).map(|bytes| {
                let units: Vec<u16> = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| u16::from_le_bytes(*pair))
                    .take_while(|unit| *unit != 0)
                    .collect();
                String::from_utf16_lossy(&units)
            })
        } else {
            None
        };
        CloseClipboard();
        text
    }
}

unsafe fn global_bytes(handle: *mut c_void) -> Option<Vec<u8>> {
    if handle.is_null() {
        return None;
    }
    let size = GlobalSize(handle);
    let data = GlobalLock(handle) as *const u8;
    if data.is_null() {
        return None;
    }
    let bytes = std::slice::from_raw_parts(data, size).to_vec();
    GlobalUnlock(handle);
    Some(bytes)
}

fn png_format() -> u32 {
    let name = win::wide("PNG");
    // SAFETY: NUL-terminated name; registration is idempotent.
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}

fn rtf_format() -> u32 {
    let name = win::wide("Rich Text Format");
    // SAFETY: NUL-terminated name; registration is idempotent.
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}

fn html_format() -> u32 {
    let name = win::wide("HTML Format");
    // SAFETY: NUL-terminated name; registration is idempotent.
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}
