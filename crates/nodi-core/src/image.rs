//! Image helpers that need no OS: format sniffing (for sizing an inserted
//! picture) and turning a clipboard DIB into a BMP file.

/// Basic facts about an encoded image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Bmp,
}

/// Reads the pixel size from a PNG, JPEG, GIF or BMP header.
pub fn sniff(bytes: &[u8]) -> Option<ImageInfo> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 && &bytes[12..16] == b"IHDR" {
        return Some(ImageInfo {
            width: be32(&bytes[16..20]),
            height: be32(&bytes[20..24]),
            format: ImageFormat::Png,
        });
    }
    if bytes.starts_with(b"GIF8") && bytes.len() >= 10 {
        return Some(ImageInfo {
            width: u32::from(le16(&bytes[6..8])),
            height: u32::from(le16(&bytes[8..10])),
            format: ImageFormat::Gif,
        });
    }
    if bytes.starts_with(b"BM") && bytes.len() >= 26 {
        let width = le32(&bytes[18..22]) as i32;
        let height = le32(&bytes[22..26]) as i32;
        return Some(ImageInfo {
            width: width.unsigned_abs(),
            height: height.unsigned_abs(),
            format: ImageFormat::Bmp,
        });
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return sniff_jpeg(bytes);
    }
    None
}

/// Walks the JPEG markers up to the first start-of-frame segment.
fn sniff_jpeg(bytes: &[u8]) -> Option<ImageInfo> {
    let mut pos = 2usize;
    while pos + 4 <= bytes.len() {
        if bytes[pos] != 0xFF {
            pos += 1;
            continue;
        }
        let marker = bytes[pos + 1];
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        // Markers without a length field.
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            pos += 2;
            continue;
        }
        let length = usize::from(be16(&bytes[pos + 2..pos + 4]));
        let is_sof = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof {
            if pos + 9 > bytes.len() {
                return None;
            }
            return Some(ImageInfo {
                height: u32::from(be16(&bytes[pos + 5..pos + 7])),
                width: u32::from(be16(&bytes[pos + 7..pos + 9])),
                format: ImageFormat::Jpeg,
            });
        }
        if length < 2 {
            return None;
        }
        pos += 2 + length;
    }
    None
}

/// Wraps a packed DIB (`CF_DIB` clipboard data: BITMAPINFOHEADER + colour
/// table + bits) in a BITMAPFILEHEADER so it becomes a regular `.bmp`.
pub fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    if dib.len() < 40 {
        return None;
    }
    let header_size = le32(&dib[0..4]) as usize;
    if !(40..=dib.len()).contains(&header_size) {
        return None;
    }
    let bit_count = u32::from(le16(&dib[14..16]));
    let compression = le32(&dib[16..20]);
    let colors_used = le32(&dib[32..36]) as usize;
    const BI_BITFIELDS: u32 = 3;
    let masks = if compression == BI_BITFIELDS && header_size == 40 {
        12
    } else {
        0
    };
    let palette = if colors_used > 0 {
        colors_used
    } else if bit_count <= 8 {
        1usize << bit_count
    } else {
        0
    };
    let offset = 14 + header_size + masks + palette * 4;
    let file_size = 14 + dib.len();
    if offset > file_size {
        return None;
    }
    let mut out = Vec::with_capacity(file_size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

/// Display size in HIMETRIC (0.01 mm) for a picture of `width`×`height`
/// pixels at 96 DPI, scaled down to fit `max_width_px` while keeping the
/// aspect ratio.
pub fn display_size_himetric(width: u32, height: u32, max_width_px: u32) -> (i32, i32) {
    let (mut w, mut h) = (width.max(1) as u64, height.max(1) as u64);
    let max = u64::from(max_width_px.max(16));
    if w > max {
        h = (h * max).div_ceil(w);
        w = max;
    }
    let to_himetric = |px: u64| ((px * 2540) / 96) as i32;
    (to_himetric(w), to_himetric(h.max(1)))
}

/// Size in pixels after trimming `left`/`top`/`right`/`bottom` pixels from
/// a `width`×`height` picture. The result never drops below 1×1.
pub fn cropped_size(
    width: u32,
    height: u32,
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
) -> (u32, u32) {
    let width = width.max(1);
    let height = height.max(1);
    let take = |a: u32, b: u32, total: u32| a.saturating_add(b).min(total.saturating_sub(1));
    (
        width - take(left, right, width),
        height - take(top, bottom, height),
    )
}

fn be16(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}
fn le16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// True for the horizontal-rule picture Insert > Separator uses: at most 8 px
/// high and at least 50 times as wide as high. A real picture that thin is a
/// rule in all but name, so it exports as `---` too.
pub fn is_rule_picture(bytes: &[u8]) -> bool {
    sniff(bytes)
        .is_some_and(|info| info.height > 0 && info.height <= 8 && info.width >= info.height * 50)
}

/// Size of the page-break picture (`----` + Enter, Insert > Page Break).
pub const PAGE_BREAK_SIZE: (u32, u32) = (600, 40);

/// Size the page-break picture had before it became a solid band; notes
/// written with it still carry a page break.
const OLD_PAGE_BREAK_SIZE: (u32, u32) = (600, 24);

/// The page-break picture: a solid grey band across the note with a lighter
/// one-pixel edge, like the gap between two pages in Word. The rows above and
/// below the band are transparent, so only the band itself separates the
/// pages. A grey + alpha PNG, recognised later by its exact size
/// ([`is_page_picture`]).
pub fn page_break_png() -> Vec<u8> {
    page_break_png_coloured((0x52, 0x52, 0x52))
}

/// [`page_break_png`] in the given band colour (RGB). The app passes the
/// line-number background so the gap looks like the gutter crossing the page.
/// The 1 px edge is a little lighter on dark colours and a little darker on
/// light ones, so it still reads as the edge of a page.
pub fn page_break_png_coloured(band: (u8, u8, u8)) -> Vec<u8> {
    let (width, height) = PAGE_BREAK_SIZE;
    let luma = 0.2126 * f32::from(band.0) + 0.7152 * f32::from(band.1) + 0.0722 * f32::from(band.2);
    let shift = |value: u8| {
        if luma < 128.0 {
            value.saturating_add(0x30)
        } else {
            value.saturating_sub(0x18)
        }
    };
    let edge = (shift(band.0), shift(band.1), shift(band.2));
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let (colour, alpha) = match y {
            // Transparent gap above the band.
            0..=7 => ((0xFF, 0xFF, 0xFF), 0),
            // Lighter page edge above the gap.
            8 => (edge, 0xFF),
            // The solid gap between the two pages.
            9..=31 => (band, 0xFF),
            // Lighter page edge below the gap.
            32 => (edge, 0xFF),
            // Transparent gap below the band.
            _ => ((0xFF, 0xFF, 0xFF), 0),
        };
        for _ in 0..width {
            pixels.push(colour.0);
            pixels.push(colour.1);
            pixels.push(colour.2);
            pixels.push(alpha);
        }
    }
    crate::math::encode_png_rgba(width, height, &pixels, "")
}

/// True for the page-break picture: the current [`PAGE_BREAK_SIZE`], or the
/// size older notes were written with.
pub fn is_page_picture(bytes: &[u8]) -> bool {
    sniff(bytes).is_some_and(|info| {
        (info.width, info.height) == PAGE_BREAK_SIZE
            || (info.width, info.height) == OLD_PAGE_BREAK_SIZE
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_break_picture_is_recognised_and_is_no_rule() {
        let png = page_break_png();
        assert!(is_page_picture(&png));
        assert!(!is_rule_picture(&png));
        // Notes written before the band was this tall still hold one.
        assert!(is_page_picture(&png_header(600, 24)));
        assert!(!is_page_picture(&png_header(600, 25)));
    }

    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    #[test]
    fn rule_pictures_are_thin_and_wide() {
        assert!(is_rule_picture(&png_header(600, 6)));
        assert!(!is_rule_picture(&png_header(600, 20)));
        assert!(!is_rule_picture(&png_header(100, 6)));
        assert!(!is_rule_picture(b"not a picture"));
    }

    #[test]
    fn sniffs_png() {
        let info = sniff(&png_header(640, 480)).unwrap();
        assert_eq!(
            (info.width, info.height, info.format),
            (640, 480, ImageFormat::Png)
        );
    }

    #[test]
    fn sniffs_gif() {
        let bytes = b"GIF89a\x20\x03\x58\x02rest";
        let info = sniff(bytes).unwrap();
        assert_eq!((info.width, info.height), (800, 600));
    }

    #[test]
    fn sniffs_jpeg_after_app_segments() {
        let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        // SOF0: length, precision, height 0x0100, width 0x0200
        bytes.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0x00, 0x02, 0x00, 0x03]);
        let info = sniff(&bytes).unwrap();
        assert_eq!(
            (info.width, info.height, info.format),
            (512, 256, ImageFormat::Jpeg)
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(sniff(b"not an image at all").is_none());
        assert!(sniff(&[0xFF, 0xD8, 0xFF]).is_none());
    }

    #[test]
    fn dib_becomes_bmp_with_correct_offset() {
        // 2x1 24-bit DIB: 40-byte header + 8 bytes of (padded) pixels.
        let mut dib = vec![0u8; 48];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&2i32.to_le_bytes());
        dib[8..12].copy_from_slice(&1i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        let bmp = dib_to_bmp(&dib).unwrap();
        assert_eq!(&bmp[0..2], b"BM");
        assert_eq!(le32(&bmp[2..6]) as usize, bmp.len());
        assert_eq!(le32(&bmp[10..14]), 54);
        let info = sniff(&bmp).unwrap();
        assert_eq!(
            (info.width, info.height, info.format),
            (2, 1, ImageFormat::Bmp)
        );
    }

    #[test]
    fn crop_keeps_at_least_one_pixel() {
        assert_eq!(cropped_size(200, 100, 10, 5, 20, 15), (170, 80));
        assert_eq!(cropped_size(200, 100, 0, 0, 0, 0), (200, 100));
        // Over-cropping clamps to one pixel instead of underflowing.
        assert_eq!(cropped_size(200, 100, 250, 150, 250, 150), (1, 1));
        assert_eq!(cropped_size(10, 10, 6, 6, 6, 6), (1, 1));
    }

    #[test]
    fn display_size_keeps_aspect_and_fits() {
        assert_eq!(display_size_himetric(96, 48, 1000), (2540, 1270));
        let (w, h) = display_size_himetric(2000, 1000, 500);
        assert_eq!(w, 500 * 2540 / 96);
        assert_eq!(h, 250 * 2540 / 96);
    }
}
