//! Formulas: LaTeX → layout → (rendered by the platform) → PNG that carries
//! its own LaTeX source in an `iTXt` chunk. Because the source travels inside
//! the picture, a formula stays editable after save/reload, copy/paste between
//! notes, and exports to Markdown as `$...$`.

pub mod layout;
pub mod parse;

pub use layout::{layout, Draw, Item, Layout, Metrics, Style, TextMetrics};
pub use parse::{parse, Node, ParseError};

/// `iTXt` keyword that marks a Nodi formula picture.
pub const LATEX_KEYWORD: &str = "nodi-latex";

/// Encodes 8-bit grey + alpha pixels (`2 * width * height` bytes, rows top to
/// bottom) as a PNG and embeds `latex` in an `iTXt` chunk.
pub fn encode_png(width: u32, height: u32, gray_alpha: &[u8], latex: &str) -> Vec<u8> {
    encode_png_with(width, height, 4, 2, gray_alpha, latex)
}

/// Encodes 8-bit RGBA pixels (`4 * width * height` bytes, rows top to bottom)
/// as a PNG. Used by pictures that must match a theme colour exactly.
pub fn encode_png_rgba(width: u32, height: u32, rgba: &[u8], latex: &str) -> Vec<u8> {
    encode_png_with(width, height, 6, 4, rgba, latex)
}

fn encode_png_with(
    width: u32,
    height: u32,
    colour_type: u8,
    channels: usize,
    pixels: &[u8],
    latex: &str,
) -> Vec<u8> {
    assert_eq!(pixels.len(), (width as usize * height as usize) * channels);
    let stride = width as usize * channels;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in pixels.chunks_exact(stride) {
        raw.push(0); // filter: none
        raw.extend_from_slice(row);
    }
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 9);

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, colour_type, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &header);

    let mut text = Vec::new();
    if !latex.is_empty() {
        text.extend_from_slice(LATEX_KEYWORD.as_bytes());
        text.extend_from_slice(&[0, 0, 0]); // separator, uncompressed, method
        text.push(0); // empty language tag
        text.push(0); // empty translated keyword
        text.extend_from_slice(latex.as_bytes());
        chunk(&mut png, b"iTXt", &text);
    }

    chunk(&mut png, b"IDAT", &compressed);
    chunk(&mut png, b"IEND", &[]);
    png
}

/// The LaTeX source stored in a Nodi formula PNG, if any.
pub fn latex_from_png(png: &[u8]) -> Option<String> {
    if !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let mut pos = 8;
    while pos + 12 <= png.len() {
        let length = u32::from_be_bytes(png[pos..pos + 4].try_into().ok()?) as usize;
        let kind = &png[pos + 4..pos + 8];
        let data_end = pos + 8 + length;
        if data_end + 4 > png.len() {
            return None;
        }
        let data = &png[pos + 8..data_end];
        if kind == b"iTXt" {
            let keyword_end = data.iter().position(|byte| *byte == 0)?;
            if &data[..keyword_end] == LATEX_KEYWORD.as_bytes() && data.len() > keyword_end + 2 {
                let compressed = data[keyword_end + 1] != 0;
                let mut rest = &data[keyword_end + 3..];
                // Skip the language tag and the translated keyword.
                for _ in 0..2 {
                    let end = rest.iter().position(|byte| *byte == 0)?;
                    rest = &rest[end + 1..];
                }
                if compressed {
                    return None;
                }
                return String::from_utf8(rest.to_vec()).ok();
            }
        }
        if kind == b"IEND" {
            break;
        }
        pos = data_end + 4;
    }
    None
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let crc = crc32(&png[start..]);
    png.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trips_latex_and_is_valid() {
        let pixels = vec![0u8, 255, 0, 0, 0, 128, 0, 64];
        let png = encode_png(2, 2, &pixels, r"\frac{a}{b} ≤ ∞");
        assert_eq!(latex_from_png(&png).as_deref(), Some(r"\frac{a}{b} ≤ ∞"));
        let info = crate::image::sniff(&png).unwrap();
        assert_eq!((info.width, info.height), (2, 2));
        // IEND chunk CRC is the well-known constant.
        assert!(png.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]));
    }

    #[test]
    fn plain_png_has_no_latex() {
        assert!(latex_from_png(b"\x89PNG\r\n\x1a\n").is_none());
        assert!(latex_from_png(b"not a png").is_none());
    }

    #[test]
    fn crc_matches_reference() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
