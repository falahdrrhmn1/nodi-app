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

/// Crop amounts (left, top, right, bottom) of the first `\pict` in twips.
/// Missing `\piccrop*` words read as 0, the RTF default.
pub fn picture_crop(rtf: &str) -> Option<(i32, i32, i32, i32)> {
    let pict = pict_group(rtf)?;
    let value = |name| control_value(pict, name).unwrap_or(0).max(0);
    Some((
        value("\\piccropl"),
        value("\\piccropt"),
        value("\\piccropr"),
        value("\\piccropb"),
    ))
}

/// Returns `rtf` with the first picture cropped by `left`/`top`/`right`/
/// `bottom` twips (`\piccropl`/`\piccropt`/`\piccropr`/`\piccropb`).
pub fn set_picture_crop(rtf: &str, left: i32, top: i32, right: i32, bottom: i32) -> Option<String> {
    let start = rtf.find("{\\pict")?;
    let end = start + group_len(&rtf[start..])?;
    let pict = &rtf[start..end];
    let mut patched = String::with_capacity(pict.len() + 64);
    patched.push_str("{\\pict");
    patched.push_str(&format!(
        "\\piccropl{} \\piccropt{} \\piccropr{} \\piccropb{} ",
        left.max(0),
        top.max(0),
        right.max(0),
        bottom.max(0)
    ));
    let mut rest = &pict["{\\pict".len()..];
    while let Some(index) = rest.find('\\') {
        patched.push_str(&rest[..index]);
        let word_end = control_word_end(&rest[index..]);
        let word = &rest[index..index + word_end];
        let name: String = word
            .chars()
            .take_while(|ch| *ch == '\\' || ch.is_ascii_alphabetic())
            .collect();
        let space = if word.ends_with(' ') { " " } else { "" };
        // Existing crop values are replaced by the four words added above.
        if name.starts_with("\\piccrop") {
            patched.push_str(space);
        } else {
            patched.push_str(word);
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

/// Folds theme-rendered colours back to their storage meaning before saving.
///
/// Dark mode displays automatic text as `auto_rgb` (near-white) and pure black
/// text as `black_rgb` (a second, one-step-different near-white) so black text
/// stays readable. This function rewrites every `\cfN` that refers to those two
/// colour-table entries: `auto_rgb` becomes `\cf0` (automatic) and `black_rgb`
/// becomes the table entry for black (added when missing). Other colours are
/// untouched, so the RTF stored on disk keeps the note's real colours.
pub fn fold_theme_colors(rtf: &str, auto_rgb: (u8, u8, u8), black_rgb: (u8, u8, u8)) -> String {
    if auto_rgb == black_rgb {
        return rtf.to_owned();
    }
    let Some(start) = rtf.find("{\\colortbl") else {
        return rtf.to_owned();
    };
    let Some(len) = group_len(&rtf[start..]) else {
        return rtf.to_owned();
    };
    let end = start + len;
    let header_len = "{\\colortbl".len();
    let body = &rtf[start + header_len..end - 1];

    // Entry 0 is the implicit "auto" slot before the first semicolon.
    let mut entries: Vec<Option<(u8, u8, u8)>> = vec![None];
    let mut segments: Vec<&str> = body.split(';').skip(1).collect();
    // The text after the final semicolon is not an entry.
    if segments
        .last()
        .is_some_and(|segment| segment.trim().is_empty())
    {
        segments.pop();
    }
    for segment in segments {
        let rgb = (
            control_value(segment, "\\red").unwrap_or(0).clamp(0, 255) as u8,
            control_value(segment, "\\green").unwrap_or(0).clamp(0, 255) as u8,
            control_value(segment, "\\blue").unwrap_or(0).clamp(0, 255) as u8,
        );
        let has_colour =
            segment.contains("\\red") || segment.contains("\\green") || segment.contains("\\blue");
        entries.push(has_colour.then_some(rgb));
    }

    let auto_index = entries
        .iter()
        .position(|entry| *entry == Some(auto_rgb))
        .map(|index| index as i32);
    let Some(auto_index) = auto_index else {
        return rtf.to_owned();
    };

    let mut table = rtf[start..end].to_owned();
    // The theme rendered black text through a second sentinel entry; fold it
    // back to the note's real black entry (appended when the note has none).
    let rendered_black = entries
        .iter()
        .position(|entry| *entry == Some(black_rgb))
        .map(|index| index as i32);
    let black_index = match entries
        .iter()
        .position(|entry| *entry == Some((0, 0, 0)))
        .map(|index| index as i32)
    {
        Some(index) => index,
        None => {
            let index = entries.len() as i32;
            let at = table.len() - 1;
            table.insert_str(at, "\\red0\\green0\\blue0;");
            index
        }
    };

    let mut mappings = vec![(auto_index, 0)];
    if let Some(rendered) = rendered_black {
        if rendered != black_index {
            mappings.push((rendered, black_index));
        }
    }
    let replaced = rewrite_color_refs(&rtf[..start], &mappings);
    let rest = rewrite_color_refs(&rtf[end..], &mappings);
    format!("{replaced}{table}{rest}")
}

/// Applies display-only theme colours before streaming RTF into an editor.
///
/// Automatic text (`\cf0`) is pointed at an `auto_rgb` colour-table entry and
/// every explicit pure-black entry at a `black_rgb` entry, so a dark theme can
/// render otherwise-black text light without changing the RTF on disk:
/// [`fold_theme_colors`] reverses both before the next save.
///
/// Text that never names a colour is automatic too, so an explicit `\cf0` is
/// added where the document body starts and after every `\plain` (and a
/// colour table when the RTF has none).
pub fn apply_theme_colors(rtf: &str, auto_rgb: (u8, u8, u8), black_rgb: (u8, u8, u8)) -> String {
    // Re-applying to an already themed document must first undo the previous
    // pass, otherwise the sentinel entries would be folded into each other.
    let folded = fold_theme_colors(rtf, auto_rgb, black_rgb);
    if auto_rgb == black_rgb || !folded.starts_with("{\\rtf") {
        return folded;
    }
    let folded = with_explicit_auto_colour(&folded);
    let Some(start) = folded.find("{\\colortbl") else {
        return folded;
    };
    let Some(len) = group_len(&folded[start..]) else {
        return folded;
    };
    let end = start + len;
    let header_len = "{\\colortbl".len();
    let body = &folded[start + header_len..end - 1];

    let mut entries = colour_entries(body);
    let mut table = folded[start..end].to_owned();
    let auto_index = ensure_colour(&mut entries, &mut table, auto_rgb);
    let black_index = ensure_colour(&mut entries, &mut table, black_rgb);

    let mut mappings = vec![(0, auto_index)];
    for (index, entry) in entries.iter().enumerate() {
        if *entry == Some((0, 0, 0)) {
            mappings.push((index as i32, black_index));
        }
    }
    let replaced = rewrite_color_refs(&folded[..start], &mappings);
    let rest = rewrite_color_refs(&folded[end..], &mappings);
    format!("{replaced}{table}{rest}")
}

/// Parses the inside of a `{\colortbl ...}` group. Index 0 is the implicit
/// automatic slot before the first semicolon; `None` marks it and any entry
/// without an explicit colour.
fn colour_entries(body: &str) -> Vec<Option<(u8, u8, u8)>> {
    let mut entries: Vec<Option<(u8, u8, u8)>> = vec![None];
    let mut segments: Vec<&str> = body.split(';').skip(1).collect();
    if segments
        .last()
        .is_some_and(|segment| segment.trim().is_empty())
    {
        segments.pop();
    }
    for segment in segments {
        let rgb = (
            control_value(segment, "\\red").unwrap_or(0).clamp(0, 255) as u8,
            control_value(segment, "\\green").unwrap_or(0).clamp(0, 255) as u8,
            control_value(segment, "\\blue").unwrap_or(0).clamp(0, 255) as u8,
        );
        let has_colour =
            segment.contains("\\red") || segment.contains("\\green") || segment.contains("\\blue");
        entries.push(has_colour.then_some(rgb));
    }
    entries
}

/// Returns the index of `rgb` in the table, appending it when missing.
fn ensure_colour(
    entries: &mut Vec<Option<(u8, u8, u8)>>,
    table: &mut String,
    (red, green, blue): (u8, u8, u8),
) -> i32 {
    if let Some(index) = entries
        .iter()
        .position(|entry| *entry == Some((red, green, blue)))
    {
        return index as i32;
    }
    let index = entries.len() as i32;
    entries.push(Some((red, green, blue)));
    let at = table.len().saturating_sub(1);
    table.insert_str(at, &format!("\\red{red}\\green{green}\\blue{blue};"));
    index
}

/// Replaces `\cfN` for the given index pairs (control-word aware, so `\cf1`
/// never eats the prefix of `\cf12`). Other colour references are left alone.
/// Adds a colour table when there is none, and an explicit `\cf0` at the start
/// of the body and after every `\plain` that is not followed by a `\cf`
/// already (so applying twice adds nothing).
fn with_explicit_auto_colour(rtf: &str) -> String {
    let mut text = rtf.to_owned();
    if !text.contains("{\\colortbl") {
        let at = text
            .find("{\\fonttbl")
            .and_then(|start| group_len(&text[start..]).map(|len| start + len))
            .unwrap_or("{\\rtf1".len().min(text.len()));
        text.insert_str(at, "{\\colortbl ;}");
    }
    let Some(table) = text.find("{\\colortbl") else {
        return text;
    };
    let Some(len) = group_len(&text[table..]) else {
        return text;
    };
    let body = table + len;
    if !text[body..].starts_with("\\cf0") {
        text.insert_str(body, "\\cf0 ");
    }

    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 64);
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 1 < bytes.len()
            && bytes[index + 1].is_ascii()
            && !bytes[index + 1].is_ascii_alphabetic()
        {
            // `\\`, `\{`, `\}`, `\'xx`, ...: an escape, never a control word.
            out.push_str(&text[index..index + 2]);
            index += 2;
            continue;
        }
        if text[index..].starts_with("\\plain") {
            let after = index + "\\plain".len();
            let word_ends = bytes.get(after).is_none_or(|b| !b.is_ascii_alphanumeric());
            if word_ends {
                out.push_str("\\plain");
                index = after;
                let rest = text[index..].trim_start_matches(' ');
                if !rest.starts_with("\\cf") {
                    out.push_str("\\cf0");
                }
                continue;
            }
        }
        let ch = text[index..].chars().next().unwrap_or(' ');
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

fn rewrite_color_refs(text: &str, mappings: &[(i32, i32)]) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && index + 1 < bytes.len()
            && bytes[index + 1].is_ascii()
            && !bytes[index + 1].is_ascii_alphabetic()
        {
            // An escape such as `\\` (literal backslash): skip both bytes so
            // text like `\\cf1` is never read as a colour reference.
            out.push_str(&text[index..index + 2]);
            index += 2;
            continue;
        }
        if bytes[index] == b'\\' && text[index..].starts_with("\\cf") {
            let digits_start = index + 3;
            let mut digits_end = digits_start;
            while digits_end < bytes.len() && bytes[digits_end].is_ascii_digit() {
                digits_end += 1;
            }
            if digits_end > digits_start {
                if let Ok(value) = text[digits_start..digits_end].parse::<i32>() {
                    if let Some((_, to)) = mappings.iter().find(|(from, _)| *from == value) {
                        out.push_str("\\cf");
                        out.push_str(&to.to_string());
                        index = digits_end;
                        continue;
                    }
                }
            }
        }
        let ch = text[index..].chars().next().unwrap_or(' ');
        out.push(ch);
        index += ch.len_utf8();
    }
    out
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
        // A control symbol: `\` plus one character (which may be multi-byte
        // in pasted text), so the slice end stays on a char boundary.
        let symbol = text[index..].chars().next().map_or(1, char::len_utf8);
        return index + symbol;
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
    fn rewrites_crop_amounts() {
        let patched = set_picture_crop(SAMPLE, 120, 60, 30, 15).unwrap();
        assert_eq!(picture_crop(&patched), Some((120, 60, 30, 15)));
        assert_eq!(picture_goal(&patched), picture_goal(SAMPLE));
        assert_eq!(picture_blob(&patched), picture_blob(SAMPLE));
        // Patching again replaces the values instead of stacking them.
        let again = set_picture_crop(&patched, 1, 2, 3, 4).unwrap();
        assert_eq!(picture_crop(&again), Some((1, 2, 3, 4)));
        assert_eq!(again.matches("\\piccropl").count(), 1);
    }

    #[test]
    fn missing_crop_is_zero_and_negative_is_clamped() {
        assert_eq!(picture_crop(SAMPLE), Some((0, 0, 0, 0)));
        let patched = set_picture_crop(SAMPLE, -5, -6, -7, -8).unwrap();
        assert_eq!(picture_crop(&patched), Some((0, 0, 0, 0)));
    }

    #[test]
    fn extracts_blob() {
        assert_eq!(
            picture_blob(SAMPLE).unwrap(),
            vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]
        );
    }

    #[test]
    fn non_ascii_control_symbol_does_not_panic() {
        let rtf = "{\\rtf1{\\pict\\é\\picwgoal10\\pichgoal20 00}}";
        assert_eq!(picture_goal(rtf), Some((10, 20)));
        assert!(set_picture_goal(rtf, 30, 40).is_some());
    }

    #[test]
    fn no_picture_is_none() {
        assert!(picture_goal("{\\rtf1 hello\\par}").is_none());
        assert!(!has_picture("{\\rtf1 hello}"));
    }

    #[test]
    fn folds_theme_auto_colour_to_automatic() {
        let rtf = "{\\rtf1\\ansi{\\colortbl ;\\red230\\green230\\blue230;\\red255\\green0\\blue0;}\\cf1 auto\\cf2 red\\cf0 plain}";
        let folded = fold_theme_colors(rtf, (230, 230, 230), (0, 0, 0));
        assert!(folded.contains("\\cf0 auto"));
        assert!(folded.contains("\\cf2 red"));
        // Black did not exist in the table: it is appended for a later save.
        assert!(folded.contains("\\red0\\green0\\blue0;"));
    }

    #[test]
    fn folds_theme_black_to_the_existing_black_entry() {
        let rtf = "{\\rtf1{\\colortbl ;\\red230\\green230\\blue230;\\red0\\green0\\blue0;}\\cf1 auto\\cf2 black\\cf1 again}";
        let folded = fold_theme_colors(rtf, (230, 230, 230), (0, 0, 0));
        assert_eq!(folded.matches("\\cf0 auto").count(), 1);
        assert!(folded.contains("\\cf0 again"));
        assert!(folded.contains("\\cf2 black"));
        assert_eq!(folded.matches("\\red0\\green0\\blue0;").count(), 1);
    }

    #[test]
    fn fold_ignores_other_colours_and_missing_tables() {
        let plain = "{\\rtf1\\cf1 nothing}";
        assert_eq!(fold_theme_colors(plain, (230, 230, 230), (0, 0, 0)), plain);
        let rtf =
            "{\\rtf1{\\colortbl ;\\red230\\green230\\blue230;\\red255\\green0\\blue0;}\\cf1 auto\\cf10 ten\\cf2 red}";
        let folded = fold_theme_colors(rtf, (230, 230, 230), (0, 0, 0));
        assert!(folded.contains("\\cf0 auto"));
        assert!(folded.contains("\\cf10 ten"));
        assert!(folded.contains("\\cf2 red"));
    }

    #[test]
    fn theme_colours_round_trip_through_fold() {
        let rtf = "{\\rtf1{\\colortbl ;\\red0\\green0\\blue0;\\red255\\green0\\blue0;}\\cf0 auto\\cf1 black\\cf2 red}";
        let themed = apply_theme_colors(rtf, (230, 230, 230), (231, 231, 231));
        assert!(themed.contains("\\red230\\green230\\blue230;"));
        assert!(themed.contains("\\red231\\green231\\blue231;"));
        // cf0 and cf1 now point at the sentinels; the explicit red survives.
        assert!(themed.contains("\\cf3 auto"));
        assert!(themed.contains("\\cf4 black"));
        assert!(themed.contains("\\cf2 red"));

        let folded = fold_theme_colors(&themed, (230, 230, 230), (231, 231, 231));
        assert!(folded.contains("\\cf0 auto"));
        assert!(folded.contains("\\cf1 black"));
        assert!(folded.contains("\\cf2 red"));
    }

    #[test]
    fn theme_colours_are_idempotent() {
        let rtf = "{\\rtf1{\\colortbl ;\\red0\\green0\\blue0;}\\cf0 auto\\cf1 black}";
        let once = apply_theme_colors(rtf, (230, 230, 230), (231, 231, 231));
        let twice = apply_theme_colors(&once, (230, 230, 230), (231, 231, 231));
        assert_eq!(once, twice);
    }

    const AUTO: (u8, u8, u8) = (230, 230, 230);
    const BLACK: (u8, u8, u8) = (231, 231, 231);

    #[test]
    fn theme_colours_add_a_table_when_there_is_none() {
        let rtf = r"{\rtf1{\fonttbl{\f0 Segoe UI;}}\pard plain text\par}";
        let themed = apply_theme_colors(rtf, AUTO, BLACK);
        assert!(themed.contains(r"\red230\green230\blue230;"), "{themed}");
        assert!(themed.contains(r"}\cf1 \pard plain text"), "{themed}");
        let folded = fold_theme_colors(&themed, AUTO, BLACK);
        assert!(folded.contains(r"\cf0 \pard plain text"), "{folded}");
    }

    #[test]
    fn text_without_a_colour_and_after_plain_is_themed() {
        let rtf = r"{\rtf1{\colortbl ;\red255\green0\blue0;}\pard body \cf1 red\plain after\plain\cf1 red2}";
        let themed = apply_theme_colors(rtf, AUTO, BLACK);
        // Entry 2 is the auto sentinel: the body start and the first `\plain`
        // point at it; the explicit red stays red.
        assert!(themed.contains(r"}\cf2 \pard body"), "{themed}");
        assert!(themed.contains(r"\plain\cf2 after"), "{themed}");
        assert!(themed.contains(r"\plain\cf1 red2"), "{themed}");
        // The first fold may append a real black entry; after that it is stable.
        let again = apply_theme_colors(&themed, AUTO, BLACK);
        assert_eq!(apply_theme_colors(&again, AUTO, BLACK), again);
        assert!(!again.contains(r"\cf0\cf"), "{again}");
    }

    #[test]
    fn escaped_backslashes_are_not_colour_references() {
        let rtf = r"{\rtf1{\colortbl ;\red0\green0\blue0;}\pard C:\\cf1\\x \cf1 hitam}";
        let themed = apply_theme_colors(rtf, AUTO, BLACK);
        assert!(themed.contains(r"C:\\cf1\\x"), "{themed}");
        let folded = fold_theme_colors(&themed, AUTO, BLACK);
        assert!(folded.contains(r"C:\\cf1\\x \cf1 hitam"), "{folded}");
    }

    #[test]
    fn non_ascii_after_a_backslash_does_not_panic() {
        let rtf = "{\\rtf1{\\colortbl ;}\\pard a\\é b\\plain\u{00e9}}";
        let themed = apply_theme_colors(rtf, AUTO, BLACK);
        assert!(themed.contains('é'));
    }
}
