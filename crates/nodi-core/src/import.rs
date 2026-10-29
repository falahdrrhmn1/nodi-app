//! Import of outside notes (File > Import Files / Import Folder, files dropped
//! on the Notes Explorer). Originals are only read; every file becomes a new
//! note in Nodi's own library, a folder becomes a folder node.
//!
//! `.md` / `.markdown` go through [`markdown_in`] (local PNG/JPEG/GIF/BMP
//! pictures next to the file are embedded; nothing is ever fetched from the
//! network), `.txt` is plain text (UTF-8/UTF-16 with or without BOM, else
//! Windows-1252), `.rtf` is kept as it is.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::image;
use crate::markdown_in;

/// File types that can be imported.
pub const EXTENSIONS: [&str; 4] = ["md", "markdown", "txt", "rtf"];

/// Largest file imported (bigger ones are skipped and reported).
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

/// One node to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Planned {
    Note {
        title: String,
        path: PathBuf,
    },
    Folder {
        title: String,
        children: Vec<Planned>,
    },
}

/// What [`plan`] found.
#[derive(Debug, Default)]
pub struct Plan {
    pub items: Vec<Planned>,
    /// `(path, reason)` of everything left out.
    pub skipped: Vec<(PathBuf, String)>,
}

impl Plan {
    /// Number of notes the plan creates.
    pub fn note_count(&self) -> usize {
        fn count(items: &[Planned]) -> usize {
            items
                .iter()
                .map(|item| match item {
                    Planned::Note { .. } => 1,
                    Planned::Folder { children, .. } => count(children),
                })
                .sum()
        }
        count(&self.items)
    }
}

/// True for a file Nodi can import.
pub fn is_importable(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            EXTENSIONS
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
}

/// Plans the import of `paths` (files and folders, e.g. a multi-selection or
/// a drop). Folders are walked recursively in name order; empty folders and
/// unknown file types are left out (unknown types are listed in `skipped`).
pub fn plan(paths: &[PathBuf]) -> Plan {
    let mut result = Plan::default();
    for path in paths {
        if let Some(item) = plan_one(path, &mut result.skipped, 0) {
            result.items.push(item);
        }
    }
    result
}

fn plan_one(path: &Path, skipped: &mut Vec<(PathBuf, String)>, depth: usize) -> Option<Planned> {
    let title = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) => {
            skipped.push((path.to_path_buf(), err.to_string()));
            return None;
        }
    };
    if meta.file_type().is_symlink() {
        skipped.push((path.to_path_buf(), "link not followed".to_owned()));
        return None;
    }
    if meta.is_dir() {
        if depth > 32 {
            skipped.push((path.to_path_buf(), "folder nested too deeply".to_owned()));
            return None;
        }
        let mut entries: Vec<PathBuf> = match fs::read_dir(path) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect(),
            Err(err) => {
                skipped.push((path.to_path_buf(), err.to_string()));
                return None;
            }
        };
        entries.sort_by_key(|entry| entry.to_string_lossy().to_lowercase());
        let children: Vec<Planned> = entries
            .iter()
            .filter(|entry| !is_hidden(entry))
            .filter_map(|entry| {
                if entry.is_file() && !is_importable(entry) {
                    skipped.push((entry.clone(), "not a .md, .txt or .rtf file".to_owned()));
                    return None;
                }
                plan_one(entry, skipped, depth + 1)
            })
            .collect();
        let title = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or(title);
        return (!children.is_empty()).then_some(Planned::Folder { title, children });
    }
    if !is_importable(path) {
        skipped.push((
            path.to_path_buf(),
            "not a .md, .txt or .rtf file".to_owned(),
        ));
        return None;
    }
    if meta.len() > MAX_FILE_BYTES {
        skipped.push((path.to_path_buf(), "larger than 50 MB".to_owned()));
        return None;
    }
    Some(Planned::Note {
        title,
        path: path.to_path_buf(),
    })
}

/// Dot files and Nodi's own mirror bookkeeping are not offered as notes.
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

/// Reads one file and converts it to `(rtf, plain text)` for a new note.
pub fn convert_file(path: &Path, width_twips: i32) -> io::Result<(Vec<u8>, String)> {
    let bytes = fs::read(path)?;
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "rtf" => {
            if !bytes.starts_with(b"{\\rtf") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "not an RTF file",
                ));
            }
            // Search sidecar: the text as the Markdown export sees it.
            let text = crate::markdown::rtf_to_markdown(&String::from_utf8_lossy(&bytes), "files")
                .markdown;
            Ok((bytes, text))
        }
        "md" | "markdown" => {
            let text = decode_text(&bytes);
            let base = path.parent().unwrap_or(Path::new("."));
            Ok((
                markdown_with_pictures(&text, base, width_twips).into_bytes(),
                text,
            ))
        }
        _ => {
            let text = decode_text(&bytes);
            Ok((text_to_rtf(&text).into_bytes(), text))
        }
    }
}

/// Text from a file of unknown encoding: UTF-8 / UTF-16 BOMs are honoured,
/// BOM-less UTF-16 is recognised by its zero bytes, valid UTF-8 is UTF-8,
/// anything else is Windows-1252 (the usual Notepad "ANSI" on Windows).
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return String::from_utf8_lossy(rest).into_owned();
    }
    let utf16 = |rest: &[u8], little: bool| {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                if little {
                    u16::from_le_bytes(*pair)
                } else {
                    u16::from_be_bytes(*pair)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(b"\xFF\xFE") {
        return utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(b"\xFE\xFF") {
        return utf16(rest, false);
    }
    if bytes.len() >= 4 && bytes.len().is_multiple_of(2) {
        let sample = &bytes[..bytes.len().min(512)];
        let odd_zeros = sample
            .iter()
            .skip(1)
            .step_by(2)
            .filter(|b| **b == 0)
            .count();
        let even_zeros = sample.iter().step_by(2).filter(|b| **b == 0).count();
        let half = sample.len() / 2;
        if odd_zeros * 3 > half && even_zeros == 0 {
            return utf16(bytes, true);
        }
        if even_zeros * 3 > half && odd_zeros == 0 {
            return utf16(bytes, false);
        }
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes
            .iter()
            .map(|byte| crate::markdown::cp1252(*byte))
            .collect(),
    }
}

/// Plain text as an RTF document in the body font, one paragraph per line.
pub fn text_to_rtf(text: &str) -> String {
    let mut out = format!(
        "{{\\rtf1\\ansi\\ansicpg1252\\deff0\\uc1{{\\fonttbl{{\\f0\\fnil\\fcharset0 {};}}}}\\viewkind4\\pard\\f0\\fs{} ",
        markdown_in::body_font(),
        markdown_in::body_size()
    );
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    for (index, line) in normalized.split('\n').enumerate() {
        if index > 0 {
            out.push_str("\\par\n");
        }
        for ch in line.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '{' => out.push_str("\\{"),
                '}' => out.push_str("\\}"),
                '\t' => out.push_str("\\tab "),
                ch if (ch as u32) < 0x20 => {}
                ch if (ch as u32) < 0x80 => out.push(ch),
                ch => {
                    let mut units = [0u16; 2];
                    for unit in ch.encode_utf16(&mut units) {
                        out.push_str(&format!("\\u{}?", *unit as i16));
                    }
                }
            }
        }
    }
    out.push_str("\\par}");
    out
}

const PICTURE_START: char = '\u{E002}';
const PICTURE_END: char = '\u{E003}';

/// Markdown → RTF with `![alt](relative/picture.png)` pictures from `base`
/// embedded. Only relative paths inside `base` (no `..`, no drive, no URL)
/// are read; anything else stays the link `markdown_in` makes of it.
pub fn markdown_with_pictures(markdown: &str, base: &Path, width_twips: i32) -> String {
    let mut pictures: Vec<Vec<u8>> = Vec::new();
    let mut text = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(at) = rest.find("![") {
        text.push_str(&rest[..at]);
        let candidate = &rest[at..];
        let parsed = candidate.find("](").and_then(|middle| {
            let close = candidate[middle + 2..].find(')')? + middle + 2;
            Some((close, candidate[middle + 2..close].trim().to_owned()))
        });
        match parsed
            .and_then(|(close, target)| local_picture(base, &target).map(|bytes| (close, bytes)))
        {
            Some((close, bytes)) => {
                text.push(PICTURE_START);
                text.push_str(&pictures.len().to_string());
                text.push(PICTURE_END);
                pictures.push(bytes);
                rest = &candidate[close + 1..];
            }
            None => {
                text.push_str("![");
                rest = &candidate[2..];
            }
        }
    }
    text.push_str(rest);

    let mut rtf = markdown_in::markdown_to_rtf(&text, width_twips);
    for (index, bytes) in pictures.iter().enumerate() {
        let token = rtf_unicode(&format!("{PICTURE_START}{index}{PICTURE_END}"));
        rtf = rtf.replacen(&token, &picture_rtf(bytes, width_twips), 1);
    }
    rtf
}

/// How `markdown_in` writes `text` (`\uN?` for non-ASCII).
fn rtf_unicode(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if (ch as u32) < 0x80 {
            out.push(ch);
        } else {
            let mut units = [0u16; 2];
            for unit in ch.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{}?", *unit as i16));
            }
        }
    }
    out
}

fn local_picture(base: &Path, target: &str) -> Option<Vec<u8>> {
    let target = target.split_whitespace().next()?.trim_matches(['<', '>']);
    let decoded = crate::links::percent_decode_text(target);
    let relative = Path::new(&decoded);
    let safe = !decoded.contains(':')
        && relative.components().all(|part| {
            matches!(
                part,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        });
    if !safe {
        return None;
    }
    let path = base.join(relative);
    if fs::metadata(&path).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    image::sniff(&bytes).map(|_| bytes)
}

/// `{\pict ...}` for PNG/JPEG data (GIF/BMP are not RTF blip types and stay
/// out), scaled to fit `width_twips`.
fn picture_rtf(bytes: &[u8], width_twips: i32) -> String {
    let Some(info) = image::sniff(bytes) else {
        return String::new();
    };
    let blip = match info.format {
        image::ImageFormat::Png => "\\pngblip",
        image::ImageFormat::Jpeg => "\\jpegblip",
        _ => return String::new(),
    };
    // 15 twips per pixel at 96 DPI, at most the text width.
    let mut width = info.width as i64 * 15;
    let mut height = info.height as i64 * 15;
    let limit = width_twips.max(1440) as i64;
    if width > limit {
        height = height * limit / width.max(1);
        width = limit;
    }
    let mut out = format!(
        "{{\\pict{blip}\\picw{}\\pich{}\\picwgoal{width}\\pichgoal{height} ",
        info.width, info.height
    );
    for (index, byte) in bytes.iter().enumerate() {
        if index % 64 == 0 {
            out.push('\n');
        }
        out.push_str(&format!("{byte:02x}"));
    }
    out.push('}');
    out
}
