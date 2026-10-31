//! Minimal PNG decoder for note icons.
//!
//! Windows' `LoadImage` only understands `.ico`, so a `.png` chosen in
//! "Change Icon... > From file..." is decoded here (non-interlaced, 8-bit
//! greyscale/RGB/palette/alpha) and handed to GDI as 32-bit BGRA.
//!
//! This is deliberately small: it is not meant to decode note pictures (Rich
//! Edit keeps those), only 16–32 px icon files.

/// A decoded image: `width * height * 4` bytes, RGBA order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Rgba {
    /// The pixel at `(x, y)` as `(red, green, blue, alpha)`.
    pub fn pixel(&self, x: u32, y: u32) -> Option<(u8, u8, u8, u8)> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let at = ((y * self.width + x) * 4) as usize;
        Some((
            self.pixels[at],
            self.pixels[at + 1],
            self.pixels[at + 2],
            self.pixels[at + 3],
        ))
    }
}

/// Decodes a PNG. Returns a readable error for unsupported variants.
pub fn decode(bytes: &[u8]) -> Result<Rgba, String> {
    if bytes.len() < 8 || bytes[..8] != [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
        return Err("not a PNG file".to_owned());
    }

    let mut width = 0u32;
    let mut height = 0u32;
    let mut depth = 0u8;
    let mut colour_type = 0u8;
    let mut interlace = 0u8;
    let mut palette: Vec<(u8, u8, u8)> = Vec::new();
    let mut transparency: Vec<u8> = Vec::new();
    let mut compressed: Vec<u8> = Vec::new();
    let mut seen_header = false;

    let mut pos = 8usize;
    while pos + 8 <= bytes.len() {
        let length =
            u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]])
                as usize;
        let kind = &bytes[pos + 4..pos + 8];
        let start = pos + 8;
        let end = start.checked_add(length).ok_or("PNG chunk overflows")?;
        if end > bytes.len() {
            return Err("truncated PNG".to_owned());
        }
        let data = &bytes[start..end];
        match kind {
            b"IHDR" => {
                if data.len() < 13 {
                    return Err("short IHDR".to_owned());
                }
                width = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
                height = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
                depth = data[8];
                colour_type = data[9];
                interlace = data[12];
                seen_header = true;
            }
            b"PLTE" => {
                palette = data
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|rgb| (rgb[0], rgb[1], rgb[2]))
                    .collect();
            }
            b"tRNS" => transparency = data.to_vec(),
            b"IDAT" => compressed.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        pos = end + 4; // skip the CRC
    }

    if !seen_header {
        return Err("PNG has no IHDR".to_owned());
    }
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(format!("unsupported PNG size {width}x{height}"));
    }
    if u64::from(width) * u64::from(height) > 16_000_000 {
        return Err(format!("PNG is too large to decode ({width}x{height})"));
    }
    if depth != 8 {
        return Err(format!("unsupported PNG bit depth {depth} (8 is required)"));
    }
    if interlace != 0 {
        return Err("interlaced PNG is not supported".to_owned());
    }
    let channels = match colour_type {
        0 => 1, // greyscale
        2 => 3, // RGB
        3 => 1, // palette index
        4 => 2, // greyscale + alpha
        6 => 4, // RGBA
        other => return Err(format!("unsupported PNG colour type {other}")),
    };
    if colour_type == 3 && palette.is_empty() {
        return Err("palette PNG without PLTE".to_owned());
    }

    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&compressed)
        .map_err(|err| format!("PNG data is corrupt: {err:?}"))?;
    let stride = width as usize * channels;
    let expected = (stride + 1) * height as usize;
    if raw.len() < expected {
        return Err("PNG image data is truncated".to_owned());
    }

    let mut lines: Vec<Vec<u8>> = Vec::with_capacity(height as usize);
    let mut previous = vec![0u8; stride];
    let mut offset = 0usize;
    for _ in 0..height {
        let filter = raw[offset];
        let line = &raw[offset + 1..offset + 1 + stride];
        let mut current = line.to_vec();
        unfilter(filter, &mut current, &previous, channels)?;
        lines.push(current.clone());
        previous = current;
        offset += stride + 1;
    }

    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for line in &lines {
        for x in 0..width as usize {
            let (red, green, blue, alpha) = match colour_type {
                0 => {
                    let value = line[x];
                    (value, value, value, 255)
                }
                2 => {
                    let at = x * 3;
                    (line[at], line[at + 1], line[at + 2], 255)
                }
                3 => {
                    let index = line[x] as usize;
                    let (red, green, blue) = palette
                        .get(index)
                        .copied()
                        .ok_or("palette index out of range")?;
                    let alpha = transparency.get(index).copied().unwrap_or(255);
                    (red, green, blue, alpha)
                }
                4 => {
                    let at = x * 2;
                    (line[at], line[at], line[at], line[at + 1])
                }
                _ => {
                    let at = x * 4;
                    (line[at], line[at + 1], line[at + 2], line[at + 3])
                }
            };
            pixels.extend_from_slice(&[red, green, blue, alpha]);
        }
    }

    Ok(Rgba {
        width,
        height,
        pixels,
    })
}

fn unfilter(filter: u8, line: &mut [u8], previous: &[u8], channels: usize) -> Result<(), String> {
    let len = line.len();
    match filter {
        0 => {}
        1 => {
            for index in channels..len {
                line[index] = line[index].wrapping_add(line[index - channels]);
            }
        }
        2 => {
            for index in 0..len {
                line[index] = line[index].wrapping_add(previous[index]);
            }
        }
        3 => {
            for index in 0..len {
                let left = if index >= channels {
                    u16::from(line[index - channels])
                } else {
                    0
                };
                let above = u16::from(previous[index]);
                line[index] = line[index].wrapping_add(((left + above) / 2) as u8);
            }
        }
        4 => {
            for index in 0..len {
                let left = i32::from(if index >= channels {
                    line[index - channels]
                } else {
                    0
                });
                let above = i32::from(previous[index]);
                let upper_left = i32::from(if index >= channels {
                    previous[index - channels]
                } else {
                    0
                });
                let estimate = left + above - upper_left;
                let distance_left = (estimate - left).abs();
                let distance_above = (estimate - above).abs();
                let distance_upper_left = (estimate - upper_left).abs();
                let predictor =
                    if distance_left <= distance_above && distance_left <= distance_upper_left {
                        left
                    } else if distance_above <= distance_upper_left {
                        above
                    } else {
                        upper_left
                    };
                line[index] = line[index].wrapping_add(predictor as u8);
            }
        }
        other => return Err(format!("unsupported PNG filter {other}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a PNG with one filter-0 scanline, so the test does not depend on
    /// an external encoder.
    fn encode(width: u32, height: u32, colour_type: u8, rows: &[Vec<u8>]) -> Vec<u8> {
        let mut raw = Vec::new();
        for row in rows {
            raw.push(0);
            raw.extend_from_slice(row);
        }
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        let mut chunk = |kind: &[u8], data: &[u8]| {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            out.extend_from_slice(&[0, 0, 0, 0]); // CRC is not checked
        };
        let mut header = Vec::new();
        header.extend_from_slice(&width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[8, colour_type, 0, 0, 0]);
        chunk(b"IHDR", &header);
        chunk(b"IDAT", &compressed);
        chunk(b"IEND", &[]);
        out
    }

    #[test]
    fn decodes_rgba() {
        let png = encode(2, 1, 6, &[vec![255, 0, 0, 255, 0, 255, 0, 128]]);
        let image = decode(&png).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.pixel(0, 0), Some((255, 0, 0, 255)));
        assert_eq!(image.pixel(1, 0), Some((0, 255, 0, 128)));
    }

    #[test]
    fn decodes_grey_and_rejects_junk() {
        let png = encode(1, 1, 0, &[vec![77]]);
        let image = decode(&png).unwrap();
        assert_eq!(image.pixel(0, 0), Some((77, 77, 77, 255)));
        assert!(decode(b"not a png").is_err());
    }
}
