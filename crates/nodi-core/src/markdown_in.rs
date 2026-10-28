//! Markdown → Rich Edit RTF (the subset Nodi itself produces), used when
//! pasting Markdown from ChatGPT, a web page or a text editor.
//!
//! Only formatting Rich Edit can show is generated: headings (sizes from
//! `format::HEADING_SIZES`), nested bullets/numbers, bold/italic/strike/
//! inline code, links (`HYPERLINK` fields), fenced code (a one-cell Consolas
//! table, the same shape `Insert > Code Block` uses), GFM tables and the
//! `---` separator. Images become a link to their address; nothing is ever
//! fetched from the network.

use crate::links;

/// Body font, mirroring `wnd::format::BODY_FONT`.
pub const BODY_FONT: &str = "Segoe UI";
/// Code font, mirroring `wnd::format::CODE_FONT`.
pub const CODE_FONT: &str = "Consolas";
const BODY_SIZE: i32 = 22;
const CODE_SIZE: i32 = 20;
/// RTF half-point size per heading level; 400/320/280/250 twips.
const HEADING_SIZES: [i32; 4] = [40, 32, 28, 25];
/// One list/indent step in twips.
const INDENT: i32 = 360;

// ---------------------------------------------------------------- inline model

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Run {
    text: String,
    style: Style,
    link: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum Block {
    Heading(usize, Vec<Run>),
    Paragraph(Vec<Run>),
    List {
        numbered: bool,
        level: usize,
        runs: Vec<Run>,
    },
    Quote(Vec<Run>),
    Code {
        language: String,
        text: String,
    },
    Table(Vec<Vec<Vec<Run>>>),
    Rule,
}

/// Converts Markdown to an RTF document. `width_twips` is the editor's text
/// width, used for code blocks and tables.
pub fn markdown_to_rtf(markdown: &str, width_twips: i32) -> String {
    let blocks = parse_blocks(markdown);
    let width = width_twips.clamp(1440, 40_000);
    let mut out = header();
    for block in &blocks {
        push_block(&mut out, block, width);
    }
    out.push('}');
    out
}

/// Plain text that carries at least two Markdown signals (or a code fence) is
/// treated as Markdown when pasted. Ordinary prose never reaches this.
pub fn looks_like_markdown(text: &str) -> bool {
    let mut signals = 0;
    for line in text.lines().take(500) {
        let line = line.trim_start();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("```") || line.starts_with("~~~") {
            return true;
        }
        if heading(line).is_some() || list_marker(line).is_some() || line.starts_with('>') {
            signals += 1;
        }
        for marker in ["**", "](", "~~", "`"] {
            if line.contains(marker) {
                signals += 1;
            }
        }
        if line.matches('|').count() >= 2 && line.ends_with('|') {
            signals += 1;
        }
        if signals >= 2 {
            return true;
        }
    }
    signals >= 2
}

// ------------------------------------------------------------------ block parse

fn parse_blocks(markdown: &str) -> Vec<Block> {
    let normalized = markdown.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut blocks = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim();
        if trimmed.is_empty() {
            index += 1;
            continue;
        }
        if let Some((fence, language)) = fence_start(trimmed) {
            let mut code = Vec::new();
            index += 1;
            while index < lines.len() {
                let candidate = lines[index].trim_start();
                if candidate.starts_with(fence) {
                    index += 1;
                    break;
                }
                code.push(lines[index]);
                index += 1;
            }
            blocks.push(Block::Code {
                language,
                text: code.join("\n"),
            });
            continue;
        }
        if let Some(level) = heading(trimmed) {
            blocks.push(Block::Heading(level, parse_inline(trimmed[level..].trim())));
            index += 1;
            continue;
        }
        if is_rule(trimmed) {
            blocks.push(Block::Rule);
            index += 1;
            continue;
        }
        if is_table_row(line) && index + 1 < lines.len() && is_table_delimiter(lines[index + 1]) {
            let mut rows: Vec<Vec<Vec<Run>>> = Vec::new();
            rows.push(table_cells(line));
            index += 2; // header + delimiter
            while index < lines.len() && is_table_row(lines[index]) {
                if !is_table_delimiter(lines[index]) {
                    rows.push(table_cells(lines[index]));
                }
                index += 1;
            }
            blocks.push(Block::Table(rows));
            continue;
        }
        if let Some((numbered, level, rest)) = list_marker(line) {
            let mut content = rest.to_owned();
            index += 1;
            while index < lines.len() {
                let next = lines[index];
                if next.trim().is_empty() {
                    break;
                }
                let indented = next.len() - next.trim_start().len();
                if indented == 0 && (list_marker(next).is_some() || is_rule(next.trim())) {
                    break;
                }
                if indented > 0 && list_marker(next).is_some() {
                    // A deeper list item is a block of its own.
                    break;
                }
                if indented == 0 && heading(next.trim()).is_some() {
                    break;
                }
                if indented == 0 && !content.ends_with(' ') {
                    // A lazy continuation line of the same item.
                    content.push(' ');
                    content.push_str(next.trim());
                    index += 1;
                    continue;
                }
                if indented > 0 {
                    content.push(' ');
                    content.push_str(next.trim());
                    index += 1;
                    continue;
                }
                break;
            }
            blocks.push(Block::List {
                numbered,
                level,
                runs: parse_inline(content.trim()),
            });
            continue;
        }
        if trimmed.starts_with('>') {
            let mut quote = Vec::new();
            while index < lines.len() {
                let next = lines[index].trim_start();
                let Some(rest) = next.strip_prefix('>') else {
                    break;
                };
                quote.push(rest.trim_start());
                index += 1;
            }
            blocks.push(Block::Quote(parse_inline(&quote.join(" "))));
            continue;
        }
        // Ordinary paragraph: soft line breaks join with a space.
        let mut text = trimmed.to_owned();
        index += 1;
        while index < lines.len() {
            let next = lines[index];
            let trimmed_next = next.trim();
            if trimmed_next.is_empty()
                || fence_start(trimmed_next).is_some()
                || heading(trimmed_next).is_some()
                || is_rule(trimmed_next)
                || list_marker(next).is_some()
                || next.trim_start().starts_with('>')
                || (is_table_row(next)
                    && index + 1 < lines.len()
                    && is_table_delimiter(lines[index + 1]))
            {
                break;
            }
            text.push(' ');
            text.push_str(trimmed_next);
            index += 1;
        }
        blocks.push(Block::Paragraph(parse_inline(&text)));
    }
    blocks
}

fn fence_start(line: &str) -> Option<(&'static str, String)> {
    for fence in ["```", "~~~"] {
        if let Some(rest) = line.strip_prefix(fence) {
            return Some((fence, rest.trim().to_owned()));
        }
    }
    None
}

/// `#` … `####` followed by a space (or end of line).
fn heading(line: &str) -> Option<usize> {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    match line.as_bytes().get(hashes) {
        None => Some(hashes.min(4)),
        Some(b' ') => Some(hashes.min(4)),
        _ => None,
    }
}

fn is_rule(line: &str) -> bool {
    if line.len() < 3 {
        return false;
    }
    let compact: String = line.chars().filter(|ch| !ch.is_whitespace()).collect();
    if compact.len() < 3 {
        return false;
    }
    let first = compact.chars().next().unwrap_or(' ');
    if !matches!(first, '-' | '*' | '_') {
        return false;
    }
    compact.chars().all(|ch| ch == first)
}

/// `- text`, `* text`, `+ text`, `1. text`, `1) text` with its nesting level.
fn list_marker(line: &str) -> Option<(bool, usize, &str)> {
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    let level = (indent / 2 + 1).min(3);
    let (numbered, after) = if let Some(after) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        (false, after)
    } else {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let tail = &rest[digits..];
        let after = tail
            .strip_prefix(". ")
            .or_else(|| tail.strip_prefix(") "))?;
        (true, after)
    };
    Some((numbered, level, after))
}

fn is_table_row(line: &str) -> bool {
    let line = line.trim();
    line.starts_with('|') && line.ends_with('|') && line.matches('|').count() >= 2
}

fn is_table_delimiter(line: &str) -> bool {
    let line = line.trim();
    if !line.starts_with('|') || line.matches('|').count() < 2 {
        return false;
    }
    line.chars()
        .all(|ch| matches!(ch, '|' | '-' | ':' | ' ' | '\t'))
        && line.contains('-')
}

fn table_cells(line: &str) -> Vec<Vec<Run>> {
    let inner = line.trim().trim_start_matches('|').trim_end_matches('|');
    inner
        .split('|')
        .map(|cell| parse_inline(cell.trim()))
        .collect()
}

// ----------------------------------------------------------------- inline parse

fn parse_inline(text: &str) -> Vec<Run> {
    let chars: Vec<char> = text.chars().collect();
    let mut runs: Vec<Run> = Vec::new();
    let mut buffer = String::new();
    let mut style = Style::default();
    let mut index = 0;

    fn flush(runs: &mut Vec<Run>, buffer: &mut String, style: Style, link: Option<&str>) {
        if buffer.is_empty() {
            return;
        }
        let text = std::mem::take(buffer);
        let link = link.map(str::to_owned);
        if let Some(last) = runs.last_mut() {
            if last.style == style && last.link == link {
                last.text.push_str(&text);
                return;
            }
        }
        runs.push(Run { text, style, link });
    }

    while index < chars.len() {
        let ch = chars[index];
        match ch {
            '\\' if index + 1 < chars.len() => {
                buffer.push(chars[index + 1]);
                index += 2;
            }
            '`' => {
                let mut end = index + 1;
                while end < chars.len() && chars[end] != '`' {
                    end += 1;
                }
                if end < chars.len() {
                    flush(&mut runs, &mut buffer, style, None);
                    let code: String = chars[index + 1..end].iter().collect();
                    runs.push(Run {
                        text: code,
                        style: Style {
                            code: true,
                            ..style
                        },
                        link: None,
                    });
                    index = end + 1;
                } else {
                    buffer.push('`');
                    index += 1;
                }
            }
            '*' | '_' => {
                let doubled = index + 1 < chars.len() && chars[index + 1] == ch;
                flush(&mut runs, &mut buffer, style, None);
                if doubled {
                    style.bold = !style.bold;
                    index += 2;
                } else {
                    style.italic = !style.italic;
                    index += 1;
                }
            }
            '~' if index + 1 < chars.len() && chars[index + 1] == '~' => {
                flush(&mut runs, &mut buffer, style, None);
                style.strike = !style.strike;
                index += 2;
            }
            '!' if index + 1 < chars.len() && chars[index + 1] == '[' => {
                if let Some((end, label, target)) = link_at(&chars, index + 1) {
                    flush(&mut runs, &mut buffer, style, None);
                    let text = if label.trim().is_empty() {
                        target.clone()
                    } else {
                        label
                    };
                    runs.push(Run {
                        text,
                        style,
                        link: Some(target),
                    });
                    index = end;
                } else {
                    buffer.push('!');
                    index += 1;
                }
            }
            '[' => {
                if let Some((end, label, target)) = link_at(&chars, index) {
                    flush(&mut runs, &mut buffer, style, None);
                    for mut run in parse_inline(&label) {
                        run.link = Some(target.clone());
                        runs.push(run);
                    }
                    index = end;
                } else {
                    buffer.push('[');
                    index += 1;
                }
            }
            '<' => {
                // `<https://…>` is a link; any other tag is dropped.
                if let Some(end) = chars[index..].iter().position(|ch| *ch == '>') {
                    let inner: String = chars[index + 1..index + end].iter().collect();
                    if inner.starts_with("http://") || inner.starts_with("https://") {
                        flush(&mut runs, &mut buffer, style, None);
                        runs.push(Run {
                            text: inner.clone(),
                            style,
                            link: Some(inner),
                        });
                    }
                    index += end + 1;
                } else {
                    buffer.push('<');
                    index += 1;
                }
            }
            _ => {
                buffer.push(ch);
                index += 1;
            }
        }
    }
    flush(&mut runs, &mut buffer, style, None);
    runs
}

/// Parses `[label](target)` starting at `chars[start] == '['`. Returns the
/// index after `)`, the label and the target.
fn link_at(chars: &[char], start: usize) -> Option<(usize, String, String)> {
    let mut index = start + 1;
    let mut depth = 1;
    while index < chars.len() {
        match chars[index] {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        index += 1;
    }
    if index >= chars.len() || chars.get(index + 1) != Some(&'(') {
        return None;
    }
    let label: String = chars[start + 1..index].iter().collect();
    let mut end = index + 2;
    let mut parens = 1;
    while end < chars.len() {
        match chars[end] {
            '(' => parens += 1,
            ')' => {
                parens -= 1;
                if parens == 0 {
                    break;
                }
            }
            _ => {}
        }
        end += 1;
    }
    if end >= chars.len() {
        return None;
    }
    let target: String = chars[index + 2..end].iter().collect();
    let target = target.trim().trim_matches('<').trim_matches('>').to_owned();
    Some((end + 1, label, target))
}

// ------------------------------------------------------------------- RTF output

fn header() -> String {
    let mut colours = String::from("{\\colortbl ;");
    // 1..=5: light block, 6..=11: output block (same as format.rs).
    for (red, green, blue) in [
        (246, 248, 250),
        (208, 215, 222),
        (235, 238, 241),
        (110, 118, 129),
        (0, 102, 204),
        (30, 30, 30),
        (60, 60, 60),
        (45, 45, 45),
        (200, 200, 200),
        (220, 220, 220),
        (110, 170, 255),
        (31, 31, 31),
    ] {
        colours.push_str(&format!("\\red{red}\\green{green}\\blue{blue};"));
    }
    colours.push('}');
    format!(
        "{{\\rtf1\\ansi\\ansicpg1252\\deff0\\uc1\
         {{\\fonttbl{{\\f0\\fnil\\fcharset0 {BODY_FONT};}}{{\\f1\\fmodern\\fcharset0 {CODE_FONT};}}\
         {{\\f2\\fnil\\fcharset2 Symbol;}}}}{colours}\\viewkind4"
    )
}

fn push_block(out: &mut String, block: &Block, width: i32) {
    match block {
        Block::Heading(level, runs) => {
            let size = HEADING_SIZES[level.saturating_sub(1).min(3)];
            out.push_str(&format!("\\pard\\sa120\\sb120\\f0\\fs{size}\\b "));
            push_runs(out, runs, true, size);
            out.push_str(&format!("\\b0\\fs{BODY_SIZE}\\par\n"));
        }
        Block::Paragraph(runs) => {
            out.push_str(&format!("\\pard\\sa0\\sb0\\f0\\fs{BODY_SIZE} "));
            push_runs(out, runs, false, BODY_SIZE);
            out.push_str("\\par\n");
        }
        Block::Quote(runs) => {
            out.push_str("\\pard\\sa0\\sb0\\li360\\ri360\\i ");
            push_runs(out, runs, false, BODY_SIZE);
            out.push_str("\\i0\\par\n");
        }
        Block::List {
            numbered,
            level,
            runs,
        } => {
            let indent = level.saturating_sub(1) as i32 * INDENT + INDENT;
            // Shape Rich Edit itself writes: a `\pntext` marker plus a
            // `{\*\pn ...}` list definition, then the hanging indent. Without
            // the `\pn` group Rich Edit treats the marker as literal text.
            let (marker, definition) = if *numbered {
                (
                    "{\\pntext\\f0 1.\\tab}",
                    "{\\*\\pn\\pnlvlbody\\pnf0\\pnindent360\\pnstart1\\pndec{\\pntxta.}}",
                )
            } else {
                (
                    "{\\pntext\\f2\\'B7\\tab}",
                    "{\\*\\pn\\pnlvlblt\\pnf2\\pnindent360{\\pntxtb\\'B7}}",
                )
            };
            out.push_str(&format!(
                "\\pard{marker}{definition}\\fi-{INDENT}\\li{indent}\\f0\\fs{BODY_SIZE} "
            ));
            push_runs(out, runs, false, BODY_SIZE);
            out.push_str("\\par\n");
        }
        Block::Code { language, text } => push_code(out, language, text, width),
        Block::Table(rows) => push_table(out, rows, width),
        Block::Rule => push_rule(out, width),
    }
}

fn push_runs(out: &mut String, runs: &[Run], bold: bool, base_size: i32) {
    for run in runs {
        let mut body = String::new();
        if run.style.code {
            body.push_str(&format!("\\f1\\fs{CODE_SIZE} "));
        } else {
            body.push_str(&format!("\\f0\\fs{base_size} "));
        }
        if run.style.bold || bold {
            body.push_str("\\b ");
        }
        if run.style.italic {
            body.push_str("\\i ");
        }
        if run.style.strike {
            body.push_str("\\strike ");
        }
        escape(&mut body, &run.text);
        body.push_str(&format!("\\b0\\i0\\strike0\\f0\\fs{base_size} "));
        match &run.link {
            Some(target) => {
                // Rich Edit drops HYPERLINK fields whose target is a bare local
                // path, so store those as file URLs; the display text keeps the
                // path the author wrote.
                let target = match links::path_file_url(target) {
                    // A target written by our own Markdown export has percent
                    // escapes (`C:\Program%20Files`); decode those first.
                    Some(_) => {
                        let decoded = links::percent_decode_text(target);
                        links::path_file_url(&decoded).unwrap_or(decoded)
                    }
                    None => target.clone(),
                };
                let target = target.replace('"', "%22").replace('\\', "\\\\");
                out.push_str("{\\field{\\*\\fldinst{HYPERLINK \"");
                out.push_str(&target);
                out.push_str("\"}}{\\fldrslt{\\ul ");
                out.push_str(&body);
                out.push_str("}}}");
            }
            None => out.push_str(&body),
        }
    }
}

fn push_code(out: &mut String, language: &str, code: &str, width: i32) {
    let label = match language.trim() {
        "" => "code",
        other => other,
    };
    let border = "\\brdrs\\brdrw10\\brdrcf2";
    // Header row: language label on the left, Copy link at the right. The
    // colour is scoped to the label group so it cannot leak into the code row
    // or into whatever follows the block (that used to turn table cells grey).
    out.push_str(&format!(
        "\\trowd\\trgaph108\\trleft0\\clcbpat3\\clbrdrt{border}\\clbrdrl{border}\
         \\clbrdrb{border}\\clbrdrr{border}\\cellx{width}\
         \\pard\\intbl\\tqr\\tx{}\\f1\\fs16{{\\cf4 ",
        (width - 200).max(0)
    ));
    escape(out, label);
    out.push_str("}\\cf0\\tab{\\field{\\*\\fldinst{HYPERLINK \"");
    out.push_str(&links::format_copy_link());
    out.push_str("\"}}{\\fldrslt{\\ul\\cf5 Copy}}}\\cell\\row");

    // Code row: an explicit dark colour (light theme) so it never inherits the
    // grey label colour from the row above.
    let body = code.trim_end_matches(['\r', '\n']);
    let mut cell = String::new();
    for (index, line) in body.split('\n').enumerate() {
        if index > 0 {
            cell.push_str("\\par ");
        }
        escape(&mut cell, line);
    }
    let mut tabs = String::new();
    for step in 1..=12 {
        tabs.push_str(&format!("\\tx{}", 480 * step));
    }
    out.push_str(&format!(
        "\\trowd\\trgaph108\\trleft0\\clcbpat1\\clbrdrt{border}\\clbrdrl{border}\
         \\clbrdrb{border}\\clbrdrr{border}\\cellx{width}\
         \\pard\\intbl\\f1\\fs20\\cf12{tabs} {cell}\\cell\\row\n\
         \\pard\\f0\\fs{BODY_SIZE}\\cf0\\par\n"
    ));
}

fn push_table(out: &mut String, rows: &[Vec<Vec<Run>>], width: i32) {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let column = width / columns as i32;
    let empty: Vec<Run> = Vec::new();
    for row in rows.iter() {
        let mut cells: Vec<&Vec<Run>> = row.iter().collect();
        while cells.len() < columns {
            cells.push(&empty);
        }
        out.push_str("\\trowd\\trgaph108\\trleft0");
        for index in 1..=columns {
            out.push_str(&format!(
                "\\clbrdrt\\brdrs\\brdrw10\\clbrdrl\\brdrs\\brdrw10\
                 \\clbrdrb\\brdrs\\brdrw10\\clbrdrr\\brdrs\\brdrw10\\cellx{}",
                column * index as i32
            ));
        }
        out.push_str(&format!("\\pard\\intbl\\f0\\fs{BODY_SIZE}\\cf0 "));
        for cell in cells.iter() {
            push_runs(out, cell, false, BODY_SIZE);
            out.push_str("\\cell ");
        }
        out.push_str("\\row\n");
    }
    out.push_str(&format!("\\pard\\f0\\fs{BODY_SIZE}\\cf0\\par\n"));
}

fn push_rule(out: &mut String, width: i32) {
    const WIDTH: u32 = 600;
    const HEIGHT: u32 = 6;
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 2) as usize);
    for _ in 0..WIDTH * HEIGHT {
        pixels.push(0x88);
        pixels.push(0xFF);
    }
    let png = crate::math::encode_png(WIDTH, HEIGHT, &pixels, "");
    // A plain picture: inside a link field Rich Edit would underline it, and
    // the Markdown export recognises the rule by its shape.
    out.push_str(&format!(
        "\\pard\\sa0\\sb0{{\\pict\\pngblip\\picw{WIDTH}\\pich{HEIGHT}\\picwgoal{width}\\pichgoal60 "
    ));
    for byte in png {
        out.push_str(&format!("{byte:02x}"));
    }
    out.push_str("}\\par\n");
}

fn escape(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\t' => out.push_str("\\tab "),
            '\n' | '\r' => out.push_str("\\line "),
            '\u{00A0}' => out.push_str("\\~"),
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::rtf_to_markdown;

    fn convert(markdown: &str) -> String {
        let rtf = markdown_to_rtf(markdown, 9000);
        assert!(rtf.starts_with("{\\rtf1"));
        assert!(rtf.ends_with('}'));
        // No braces may be unbalanced: Rich Edit would reject the stream.
        let mut depth = 0i32;
        let mut escaped = false;
        for ch in rtf.chars() {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            assert!(depth >= 0, "unbalanced RTF");
        }
        assert_eq!(depth, 0, "unbalanced RTF: {rtf}");
        rtf
    }

    fn round_trip(markdown: &str) -> String {
        rtf_to_markdown(&convert(markdown), "files").markdown
    }

    #[test]
    fn headings_use_the_editor_sizes() {
        let rtf = convert("# Satu\n\n### Tiga");
        assert!(rtf.contains("\\fs40\\b "), "{rtf}");
        assert!(rtf.contains("\\fs28\\b "), "{rtf}");
        let markdown = round_trip("# Satu\n\n### Tiga");
        assert!(markdown.contains("# Satu"), "{markdown}");
        assert!(markdown.contains("### Tiga"), "{markdown}");
    }

    #[test]
    fn emphasis_and_code_spans() {
        let markdown = round_trip("Plain **bold**, *italic*, ~~gone~~ and `let x = 1;`.");
        assert!(markdown.contains("**bold**"), "{markdown}");
        assert!(markdown.contains("*italic*"), "{markdown}");
        assert!(markdown.contains("~~gone~~"), "{markdown}");
        assert!(markdown.contains("`let x = 1;`"), "{markdown}");
    }

    #[test]
    fn lists_nest_and_number() {
        let markdown = round_trip("- satu\n  - anak\n- dua\n\n1. pertama\n2. kedua");
        assert!(markdown.contains("- satu"), "{markdown}");
        assert!(markdown.contains("  - anak"), "{markdown}");
        assert!(markdown.contains("1. pertama"), "{markdown}");
        assert!(markdown.contains("1. kedua"), "{markdown}");
    }

    #[test]
    fn fenced_code_becomes_a_code_table() {
        let markdown = round_trip("Text\n\n```rust\nfn main() {\n\tx();\n}\n```\n\nEnd");
        assert!(markdown.contains("```"), "{markdown}");
        assert!(markdown.contains("fn main() {"), "{markdown}");
        assert!(markdown.contains("\tx();"), "{markdown}");
        assert!(!markdown.contains("End```"), "{markdown}");
    }

    #[test]
    fn gfm_tables_round_trip() {
        let markdown = round_trip("| Nama | Nilai |\n| --- | --- |\n| A | 92% |");
        assert!(markdown.contains("| Nama | Nilai |"), "{markdown}");
        assert!(markdown.contains("| --- | --- |"), "{markdown}");
        assert!(markdown.contains("| A | 92% |"), "{markdown}");
    }

    #[test]
    fn links_images_and_rules() {
        let rtf = convert(
            "[Docs](https://example.com/a b) and ![Logo](https://example.com/x.png)\n\n---",
        );
        assert!(
            rtf.contains("HYPERLINK \"https://example.com/a b\""),
            "{rtf}"
        );
        assert!(
            rtf.contains("HYPERLINK \"https://example.com/x.png\""),
            "{rtf}"
        );
        assert!(rtf.contains(r"{\pict\pngblip\picw600\pich6"), "{rtf}");
        assert!(!rtf.contains("nodi-hr:"), "{rtf}");
        let markdown = round_trip("[Docs](https://example.com/a b)\n\n---");
        assert!(
            markdown.contains("[Docs](https://example.com/a%20b)"),
            "{markdown}"
        );
        assert!(markdown.contains("---"), "{markdown}");
    }

    #[test]
    fn escapes_and_unicode() {
        let markdown = round_trip("Harga 100% — “kutip” {kurung} \\backslash");
        assert!(markdown.contains("Harga 100%"), "{markdown}");
        assert!(markdown.contains('“'), "{markdown}");
        assert!(markdown.contains("{kurung}"), "{markdown}");
    }

    #[test]
    fn heuristic_needs_two_signals_or_a_fence() {
        assert!(!looks_like_markdown("Biasa saja, tanpa format."));
        assert!(!looks_like_markdown("- satu saja"));
        assert!(looks_like_markdown("# Judul\n\n- satu"));
        assert!(looks_like_markdown("```\ncode\n```"));
        assert!(looks_like_markdown("**a** and [b](c)"));
    }
}
