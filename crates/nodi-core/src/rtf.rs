//! Small, targeted RTF helpers. Nodi stores notes as the RTF that Rich Edit
//! writes; these functions inspect or patch well-known pieces of it (pictures)
//! without a full RTF parser.

/// Display size of the first `\pict` in twips (`\picwgoal`, `\pichgoal`),
/// with `\picscalex`/`\picscaley` (percent) applied when present.
pub fn picture_goal(rtf: &str) -> Option<(i32, i32)> {
    let pict = pict_group(rtf)?;
    let width = control_value(pict, "\\picwgoal")?;
    let height = control_value(pict, "\\pichgoal")?;
    let scale_x = control_value(pict, "\\picscalex").unwrap_or(100);
    let scale_y = control_value(pict, "\\picscaley").unwrap_or(100);
    Some((width * scale_x / 100, height * scale_y / 100))
}

/// Returns `rtf` with the first picture resized to `width`×`height` twips.
/// Any `\picscalex`/`\picscaley` is dropped so the goal size is authoritative.
pub fn set_picture_goal(rtf: &str, width: i32, height: i32) -> Option<String> {
    let start = rtf.find("{\\pict")?;
    let end = start + group_len(&rtf[start..])?;
    let pict = &rtf[start..end];
    let mut patched = String::with_capacity(pict.len() + 32);
    let mut rest = pict;
    while let Some(index) = rest.find('\\') {
        patched.push_str(&rest[..index]);
        let word_end = control_word_end(&rest[index..]);
        let word = &rest[index..index + word_end];
        let name: String = word
            .chars()
            .take_while(|ch| *ch == '\\' || ch.is_ascii_alphabetic())
            .collect();
        // Keep the delimiting space so following hex data cannot merge with
        // the new number.
        let space = if word.ends_with(' ') { " " } else { "" };
        match name.as_str() {
            "\\picwgoal" => patched.push_str(&format!("\\picwgoal{width}{space}")),
            "\\pichgoal" => patched.push_str(&format!("\\pichgoal{height}{space}")),
            "\\picscalex" | "\\picscaley" => patched.push_str(space),
            _ => patched.push_str(word),
        }
        rest = &rest[index + word_end..];
    }
    patched.push_str(rest);
    Some(format!("{}{}{}", &rtf[..start], patched, &rtf[end..]))
}

/// The encoded bytes (PNG/JPEG/…) of the first picture, decoded from hex.
pub fn picture_blob(rtf: &str) -> Option<Vec<u8>> {
    let pict = pict_group(rtf)?;
    // The data follows the last control word of the group (skipping nested
    // groups such as `{\*\picprop ...}`).
    let mut depth = 0i32;
    let mut data_start = None;
    let bytes = pict.as_bytes();
    let mut index = 1; // skip the opening brace
    while index < bytes.len() {
        match bytes[index] {
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            b'\\' if depth == 0 => {
                index += control_word_end(&pict[index..]);
                data_start = Some(index);
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    let data = &pict[data_start?..];
    let hex: Vec<u8> = data
        .bytes()
        .take_while(|byte| *byte != b'}')
        .filter(u8::is_ascii_hexdigit)
        .collect();
    if hex.len() < 2 {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_chunks::<2>().0 {
        let text = std::str::from_utf8(pair).ok()?;
        out.push(u8::from_str_radix(text, 16).ok()?);
    }
    Some(out)
}

/// True when the RTF contains a picture.
pub fn has_picture(rtf: &str) -> bool {
    rtf.contains("{\\pict")
}

/// The first complete `{\pict ...}` group.
pub fn picture_group(rtf: &str) -> Option<&str> {
    pict_group(rtf)
}

fn pict_group(rtf: &str) -> Option<&str> {
    let start = rtf.find("{\\pict")?;
    let len = group_len(&rtf[start..])?;
    Some(&rtf[start..start + len])
}

/// Length of the balanced `{...}` group at the start of `text`.
fn group_len(text: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                index += 2; // skip escaped character / first letter of a word
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Length of the control word at the start of `text` (`\word-123 ` including
/// one delimiting space), or 2 for a control symbol such as `\*`.
fn control_word_end(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut index = 1;
    if index < bytes.len() && !bytes[index].is_ascii_alphabetic() {
        return (index + 1).min(bytes.len());
    }
    while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
        index += 1;
    }
    if index < bytes.len() && (bytes[index] == b'-' || bytes[index].is_ascii_digit()) {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
    }
    if index < bytes.len() && bytes[index] == b' ' {
        index += 1;
    }
    index
}

/// Numeric parameter of the first `name` control word in `text`.
fn control_value(text: &str, name: &str) -> Option<i32> {
    let mut search = 0;
    while let Some(found) = text[search..].find(name) {
        let after = search + found + name.len();
        let digits: String = text[after..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit() || *ch == '-')
            .collect();
        // Reject longer words that merely start with `name`.
        let next = text[after..].chars().next();
        if !digits.is_empty() {
            return digits.parse().ok();
        }
        if next.is_some_and(|ch| ch.is_ascii_alphabetic()) {
            search = after;
            continue;
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "{\\rtf1\\ansi\\pard\\f0\\fs22{\\pict{\\*\\picprop{\\sp{\\sn wzDescription}{\\sv image}}}\\pngblip\\picw6350\\pich3175\\picwgoal3600\\pichgoal1800 \r\n89504e47\r\n0d0a1a0a\r\n}\\par\r\n}";

    #[test]
    fn reads_goal_size() {
        assert_eq!(picture_goal(SAMPLE), Some((3600, 1800)));
        let scaled = SAMPLE.replace("\\pngblip", "\\pngblip\\picscalex50\\picscaley50");
        assert_eq!(picture_goal(&scaled), Some((1800, 900)));
    }

    #[test]
    fn rewrites_goal_size_and_drops_scale() {
        let scaled = SAMPLE.replace("\\pngblip", "\\pngblip\\picscalex50\\picscaley50");
        let patched = set_picture_goal(&scaled, 1200, 600).unwrap();
        assert_eq!(picture_goal(&patched), Some((1200, 600)));
        assert!(!patched.contains("picscale"));
        assert!(patched.contains("\\picw6350"));
        assert_eq!(picture_blob(&patched), picture_blob(SAMPLE));
    }

    #[test]
    fn extracts_blob() {
        assert_eq!(
            picture_blob(SAMPLE).unwrap(),
            vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]
        );
    }

    #[test]
    fn no_picture_is_none() {
        assert!(picture_goal("{\\rtf1 hello\\par}").is_none());
        assert!(!has_picture("{\\rtf1 hello}"));
    }
}
