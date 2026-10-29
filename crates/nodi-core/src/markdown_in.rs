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

/// Formulas pasted as `$…$`, `$$…$$`, `\(…\)` or `\[…\]` are written as
/// `FORMULA_START` + LaTeX + `FORMULA_END` (private-use characters). The
/// editor replaces each such span with a rendered formula after the paste
/// (rendering needs Windows), or with the LaTeX as inline code when it
/// cannot be rendered — the text is never lost.
pub const FORMULA_START: char = '\u{E000}';
/// See [`FORMULA_START`].
pub const FORMULA_END: char = '\u{E001}';

/// Code font, mirroring `wnd::format::CODE_FONT`.
pub const CODE_FONT: &str = "Consolas";

/// Default body font for new and pasted text (Preferences > Default font);
/// the Windows editor reads the same value.
static BODY: std::sync::RwLock<Option<(String, i32)>> = std::sync::RwLock::new(None);

/// Sets the default body font and size (half-points, `22` = 11 pt).
pub fn set_body(font: &str, half_points: i32) {
    if let Ok(mut body) = BODY.write() {
        *body = Some((font.to_owned(), half_points.clamp(8, 400)));
    }
}

/// The default body font (`Segoe UI` unless changed in Preferences).
pub fn body_font() -> String {
    BODY.read()
        .ok()
        .and_then(|body| body.as_ref().map(|(font, _)| font.clone()))
        .unwrap_or_else(|| "Segoe UI".to_owned())
}

/// The default body size in half-points (`22` = 11 pt).
pub fn body_size() -> i32 {
    BODY.read()
        .ok()
        .and_then(|body| body.as_ref().map(|(_, size)| *size))
        .unwrap_or(22)
}
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
    /// The run's text is LaTeX.
    math: bool,
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
        /// A task-list item (`- [ ]` / `- [x]`): a checklist box, ticked or not.
        check: Option<bool>,
    },
    Quote(Vec<Run>),
    Code {
        language: String,
        text: String,
    },
    Table(Vec<Vec<Vec<Run>>>),
    Rule,
    PageBreak,
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

/// Characters that only draw a table's frame (box drawing, ASCII borders).
fn is_border_char(ch: char) -> bool {
    matches!(ch, '-' | '=' | '+' | '|' | ':' | ' ' | '\t' | '\\')
        || ('\u{2500}'..='\u{257F}').contains(&ch)
}

/// A line that is only a table border (`+----+`, `├────┼────┤`, `|---|`).
fn is_border_line(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty()
        && line.chars().all(is_border_char)
        && line
            .chars()
            .filter(|ch| matches!(ch, '-' | '=' | '─' | '━' | '═'))
            .count()
            >= 3
}

/// The cells of a table row separated by `|` or `│`, without the frame's
/// empty first/last cells; `None` when the line has no separators.
fn text_table_cells(line: &str) -> Option<Vec<String>> {
    // Terminal copies often end every row with a wrap marker.
    let line = line.trim_end().trim_end_matches('\\').trim_end();
    let separators = line
        .chars()
        .filter(|ch| matches!(ch, '|' | '│' | '┃' | '║'))
        .count();
    if separators < 2 {
        return None;
    }
    let mut cells: Vec<String> = line
        .split(['|', '│', '┃', '║'])
        .map(|cell| cell.trim().to_owned())
        .collect();
    if cells.first().is_some_and(String::is_empty) {
        cells.remove(0);
    }
    if cells.last().is_some_and(String::is_empty) {
        cells.pop();
    }
    (cells.len() >= 2).then_some(cells)
}

fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(
            ch,
            '\\' | '*' | '_' | '[' | ']' | '#' | '>' | '`' | '|' | '~' | '$'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Plain text with a table drawn in characters — terminal output such as
/// `pm2 ls`, `docker ps` or MySQL, with `|`/`│` columns and `+---+`/`├──┤`
/// borders — as Markdown with a real table, so it pastes as a Nodi table.
/// The lines before and after the table stay ordinary paragraphs. `None`
/// when the text is not such a table (Markdown tables are handled by the
/// Markdown paste already).
pub fn text_table_markdown(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let is_row = |line: &str| text_table_cells(line).is_some() && !is_border_line(line);
    // The longest run of rows and borders.
    let mut best: Option<(usize, usize)> = None;
    let mut start = None;
    for index in 0..=lines.len() {
        let inside = index < lines.len() && (is_row(lines[index]) || is_border_line(lines[index]));
        match (inside, start) {
            (true, None) => start = Some(index),
            (false, Some(first)) => {
                let rows = lines[first..index]
                    .iter()
                    .filter(|line| is_row(line))
                    .count();
                if rows >= 2 && best.is_none_or(|(a, b)| index - first > b - a) {
                    best = Some((first, index));
                }
                start = None;
            }
            _ => {}
        }
    }
    let (first, last) = best?;
    let block = &lines[first..last];
    // A Markdown table (`|---|` right after the header) needs no help.
    let markdown_table = block.len() >= 2
        && block[0].trim_start().starts_with('|')
        && block[1].trim().starts_with('|')
        && block[1].contains("--")
        && !block
            .iter()
            .any(|line| line.contains('│') || line.trim().starts_with('+'));
    if markdown_table {
        return None;
    }
    let rows: Vec<Vec<String>> = block
        .iter()
        .filter(|line| is_row(line))
        .filter_map(|line| text_table_cells(line))
        .collect();
    let columns = rows.iter().map(Vec::len).max()?;
    let mut out = String::new();
    for line in &lines[..first] {
        if !line.trim().is_empty() {
            out.push_str(&escape_markdown(line.trim_end()));
        }
        out.push_str("\n\n");
    }
    for (index, row) in rows.iter().enumerate() {
        out.push('|');
        for column in 0..columns {
            let cell = row.get(column).map(String::as_str).unwrap_or("");
            out.push(' ');
            out.push_str(&escape_markdown(cell));
            out.push_str(" |");
        }
        out.push('\n');
        if index == 0 {
            out.push('|');
            out.push_str(&"---|".repeat(columns));
            out.push('\n');
        }
    }
    for line in &lines[last..] {
        out.push('\n');
        if !line.trim().is_empty() {
            out.push_str(&escape_markdown(line.trim_end()));
            out.push('\n');
        }
    }
    Some(out)
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
        // Display math is unmistakable on its own.
        if line.starts_with("$$") || line.starts_with("\\[") || line.contains("\\(") {
            return true;
        }
        let chars: Vec<char> = line.chars().collect();
        if (0..chars.len()).any(|at| chars[at] == '$' && inline_math_at(&chars, at).is_some()) {
            signals += 1;
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
        if let Some((close, rest)) = trimmed
            .strip_prefix("$$")
            .map(|rest| ("$$", rest))
            .or_else(|| trimmed.strip_prefix("\\[").map(|rest| ("\\]", rest)))
        {
            // Display math, possibly over several lines.
            let mut latex = String::new();
            let mut current = rest.to_owned();
            index += 1;
            loop {
                if let Some(end) = current.find(close) {
                    latex.push_str(&current[..end]);
                    let after = current[end + close.len()..].trim().to_owned();
                    blocks.push(Block::Paragraph(vec![math_run(&latex)]));
                    if !after.is_empty() {
                        blocks.push(Block::Paragraph(parse_inline(&after)));
                    }
                    break;
                }
                latex.push_str(&current);
                latex.push(' ');
                if index >= lines.len() {
                    // Never closed: keep it as text.
                    blocks.push(Block::Paragraph(parse_inline(trimmed)));
                    break;
                }
                current = lines[index].to_owned();
                index += 1;
            }
            continue;
        }
        if let Some(level) = heading(trimmed) {
            blocks.push(Block::Heading(level, parse_inline(trimmed[level..].trim())));
            index += 1;
            continue;
        }
        if is_page_break(trimmed) {
            blocks.push(Block::PageBreak);
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
            let content = content.trim();
            let (check, content) = match content.get(..4) {
                Some("[ ] ") if !numbered => (Some(false), &content[4..]),
                Some("[x] " | "[X] ") if !numbered => (Some(true), &content[4..]),
                _ => (None, content),
            };
            blocks.push(Block::List {
                numbered,
                level,
                runs: parse_inline(content),
                check,
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

/// A page break: the HTML `markdown::PAGE_BREAK` writes (any spacing or
/// case), or LaTeX's `\pagebreak` / `\newpage`.
fn is_page_break(line: &str) -> bool {
    let compact: String = line
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    compact.starts_with("<divstyle=\"page-break-after:always")
        || compact == "\\pagebreak"
        || compact == "\\newpage"
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
    let level = (indent / 2 + 1).min(9);
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
        if let Some((end, latex)) = inline_math_at(&chars, index) {
            flush(&mut runs, &mut buffer, style, None);
            runs.push(math_run(&latex));
            index = end;
            continue;
        }
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

fn math_run(latex: &str) -> Run {
    Run {
        text: latex.trim().to_owned(),
        style: Style {
            math: true,
            ..Style::default()
        },
        link: None,
    }
}

/// Inline formula at `start`: `$$…$$`, `\(…\)`, `\[…\]` or `$…$`. A single
/// `$` pair must hug its content (`$x$`, not `$ 5 and $`) and must not be
/// followed by a digit, so prices stay text. Returns the index after the
/// closing delimiter and the LaTeX.
fn inline_math_at(chars: &[char], start: usize) -> Option<(usize, String)> {
    let starts_with = |at: usize, text: &str| {
        let pattern: Vec<char> = text.chars().collect();
        chars.get(at..at + pattern.len()) == Some(pattern.as_slice())
    };
    let find = |from: usize, close: &str| (from..chars.len()).find(|&at| starts_with(at, close));
    for (open, close) in [("$$", "$$"), ("\\(", "\\)"), ("\\[", "\\]")] {
        if starts_with(start, open) {
            let from = start + open.chars().count();
            let end = find(from, close)?;
            let latex: String = chars[from..end].iter().collect();
            if latex.trim().is_empty() {
                return None;
            }
            return Some((end + close.chars().count(), latex));
        }
    }
    if chars.get(start) != Some(&'$') || start > 0 && chars[start - 1] == '\\' {
        return None;
    }
    let first = *chars.get(start + 1)?;
    if first.is_whitespace() || first == '$' {
        return None;
    }
    let end = (start + 2..chars.len()).find(|&at| chars[at] == '$' && chars[at - 1] != '\\')?;
    if chars[end - 1].is_whitespace() || chars.get(end + 1).is_some_and(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((end + 1, chars[start + 1..end].iter().collect()))
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
    let body_font = body_font();
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
         {{\\fonttbl{{\\f0\\fnil\\fcharset0 {body_font};}}{{\\f1\\fmodern\\fcharset0 {CODE_FONT};}}\
         {{\\f2\\fnil\\fcharset2 Symbol;}}}}{colours}\\viewkind4"
    )
}

fn push_block(out: &mut String, block: &Block, width: i32) {
    let body_size = body_size();
    match block {
        Block::Heading(level, runs) => {
            let size = HEADING_SIZES[level.saturating_sub(1).min(3)];
            out.push_str(&format!("\\pard\\sa120\\sb120\\f0\\fs{size}\\b "));
            push_runs(out, runs, true, size);
            out.push_str(&format!("\\b0\\fs{body_size}\\par\n"));
        }
        Block::Paragraph(runs) => {
            out.push_str(&format!("\\pard\\sa0\\sb0\\f0\\fs{body_size} "));
            push_runs(out, runs, false, body_size);
            out.push_str("\\par\n");
        }
        Block::Quote(runs) => {
            out.push_str("\\pard\\sa0\\sb0\\li360\\ri360\\i ");
            push_runs(out, runs, false, body_size);
            out.push_str("\\i0\\par\n");
        }
        Block::List {
            level,
            runs,
            check: Some(ticked),
            ..
        } => {
            // A checklist item: the box is a literal glyph + tab with a
            // hanging indent, as Nodi's editor writes it.
            let indent = level.saturating_sub(1) as i32 * INDENT + INDENT;
            let glyph = if *ticked { 9745 } else { 9744 };
            out.push_str(&format!(
                "\\pard\\fi-{INDENT}\\li{indent}\\f0\\fs{body_size} \\u{glyph}?\\tab "
            ));
            push_runs(out, runs, false, body_size);
            out.push_str("\\par\n");
        }
        Block::List {
            numbered,
            level,
            runs,
            check: None,
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
                "\\pard{marker}{definition}\\fi-{INDENT}\\li{indent}\\f0\\fs{body_size} "
            ));
            push_runs(out, runs, false, body_size);
            out.push_str("\\par\n");
        }
        Block::Code { language, text } => push_code(out, language, text, width),
        Block::Table(rows) => push_table(out, rows, width),
        Block::Rule => push_rule(out, width),
        Block::PageBreak => push_page_break(out, width),
    }
}

fn push_runs(out: &mut String, runs: &[Run], bold: bool, base_size: i32) {
    for run in runs {
        if run.style.math {
            // A placeholder the editor turns into a rendered formula.
            out.push_str(&format!("\\f0\\fs{base_size} "));
            escape(out, &format!("{FORMULA_START}{}{FORMULA_END}", run.text));
            continue;
        }
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
        // Pasted content decides the target, so only known-safe schemes
        // become links (the rest stays as its text).
        match run
            .link
            .as_deref()
            .filter(|target| links::pasted_link_allowed(target))
        {
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
                    None => target.to_owned(),
                };
                let target = links::field_target(&target);
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
    let body_size = body_size();
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
         \\pard\\f0\\fs{body_size}\\cf0\\par\n"
    ));
}

fn push_table(out: &mut String, rows: &[Vec<Vec<Run>>], width: i32) {
    let body_size = body_size();
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
        out.push_str(&format!("\\pard\\intbl\\f0\\fs{body_size}\\cf0 "));
        for cell in cells.iter() {
            push_runs(out, cell, false, body_size);
            out.push_str("\\cell ");
        }
        out.push_str("\\row\n");
    }
    out.push_str(&format!("\\pard\\f0\\fs{body_size}\\cf0\\par\n"));
}

fn push_page_break(out: &mut String, width: i32) {
    let (picture_width, picture_height) = crate::image::PAGE_BREAK_SIZE;
    out.push_str(&format!(
        "\\pard\\sa0\\sb0{{\\pict\\pngblip\\picw{picture_width}\\pich{picture_height}\\picwgoal{width}\\pichgoal360 "
    ));
    for byte in crate::image::page_break_png() {
        out.push_str(&format!("{byte:02x}"));
    }
    out.push_str("}\\par\n");
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

    #[test]
    fn terminal_tables_become_markdown_tables() {
        let pm2 = "performa@host:~$ pm2 ls\n\
            ┌────┬──────────┬────────┐\\\n\
            │ id │ name     │ status │\\\n\
            ├────┼──────────┼────────┤\\\n\
            │ 14 │ event    │ online │\\\n\
            │ 13 │ runner   │ online │\\\n\
            └────┴──────────┴────────┘\\\n\
            performa@host:~$";
        let markdown = text_table_markdown(pm2).expect("a table");
        // Escaped for Markdown; the paste shows `performa@host:~$ pm2 ls`.
        assert!(
            markdown.starts_with(r"performa@host:\~\$ pm2 ls"),
            "{markdown}"
        );
        assert!(
            markdown.contains("| id | name | status |\n|---|---|---|\n"),
            "{markdown}"
        );
        assert!(markdown.contains("| 13 | runner | online |"), "{markdown}");
        assert!(
            markdown.trim_end().ends_with(r"performa@host:\~\$"),
            "{markdown}"
        );
        let rtf = markdown_to_rtf(&markdown, 9000);
        assert!(rtf.contains("performa@host:~$ pm2 ls"), "{rtf}");
        assert!(rtf.contains(r"\cell"), "{rtf}");

        let mysql = "+----+-------+\n| id | title |\n+----+-------+\n|  1 | a_b   |\n|  2 | c     |\n+----+-------+";
        let markdown = text_table_markdown(mysql).expect("a table");
        assert!(markdown.contains("| 1 | a\\_b |"), "{markdown}");

        // Markdown tables and prose are left to the other paste paths.
        assert!(text_table_markdown("| a | b |\n|---|---|\n| 1 | 2 |").is_none());
        assert!(text_table_markdown("just a line | with a pipe\nand more").is_none());
    }
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
    fn hostile_link_targets_cannot_inject_rtf() {
        for target in [
            r"https://x.test/}}{\object\objemb{\objdata 01}}{",
            r#"https://x.test/a"b"#,
            "https://例え.jp/ä",
            "https://x.test/\u{1}ctl",
        ] {
            let rtf = convert(&format!("[klik]({target})"));
            assert_eq!(rtf.matches("\\field").count(), 1, "{rtf}");
            assert!(!rtf.contains("\\object"), "{rtf}");
            let start = rtf.find("HYPERLINK \"").expect("link") + "HYPERLINK \"".len();
            let end = start + rtf[start..].find('"').expect("closing quote");
            let stored = &rtf[start..end];
            assert!(
                stored
                    .chars()
                    .all(|ch| (' '..='~').contains(&ch) && !matches!(ch, '{' | '}' | '\\' | '"')),
                "unsafe target {stored:?}"
            );
            assert_eq!(links::percent_decode_text(stored), target.trim(), "{rtf}");
        }
    }

    #[test]
    fn only_safe_schemes_become_links() {
        for target in [
            "javascript:alert(1)",
            "vbscript:x",
            "search-ms:query=a",
            "data:text/html,x",
            "ms-settings:",
        ] {
            let rtf = convert(&format!("[klik]({target})"));
            assert!(!rtf.contains("\\field"), "{target} became a link: {rtf}");
            assert!(rtf.contains("klik"), "{rtf}");
        }
        for target in [
            "https://x.test",
            "mailto:a@b.c",
            "nodi://note/abc",
            r"C:\Data\a.pdf",
        ] {
            let rtf = convert(&format!("[klik]({target})"));
            assert!(rtf.contains("\\field"), "{target} lost its link: {rtf}");
        }
    }

    #[test]
    fn local_paths_with_braces_and_non_ascii_round_trip() {
        let rtf = convert(r"[doc](C:\Folder {1}\ä.pdf)");
        let start = rtf.find("HYPERLINK \"").expect("link") + "HYPERLINK \"".len();
        let end = start + rtf[start..].find('"').expect("closing quote");
        assert_eq!(
            links::file_url_path(&rtf[start..end]).as_deref(),
            Some(r"C:\Folder {1}\ä.pdf")
        );
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

    fn formulas_in(markdown: &str) -> Vec<String> {
        let rtf = convert(markdown);
        // Decode the \uN? escapes back to text, then cut out the placeholders.
        let mut text = String::new();
        let mut rest = rtf.as_str();
        while let Some(at) = rest.find("\\u") {
            text.push_str(&rest[..at]);
            let digits: String = rest[at + 2..]
                .chars()
                .take_while(|ch| *ch == '-' || ch.is_ascii_digit())
                .collect();
            let value: i32 = digits.parse().unwrap_or(63);
            text.push(char::from_u32((value as i16 as u16) as u32).unwrap_or('?'));
            rest = &rest[at + 2 + digits.len() + 1..];
        }
        text.push_str(rest);
        text.split(FORMULA_START)
            .skip(1)
            .map(|part| {
                part.split(FORMULA_END)
                    .next()
                    .unwrap_or("")
                    .replace("\\{", "{")
                    .replace("\\}", "}")
                    .replace("\\\\", "\\")
            })
            .collect()
    }

    #[test]
    fn pasted_formulas_become_placeholders() {
        assert_eq!(formulas_in("Energi $E = mc^2$ saja"), vec!["E = mc^2"]);
        assert_eq!(
            formulas_in(r"Rumus \(a_1 + b_2\) di tengah"),
            vec!["a_1 + b_2"]
        );
        assert_eq!(
            formulas_in("Lihat:\n\n\\[\n\\frac{a}{b}\n\\]\n\nlanjut"),
            vec![r"\frac{a}{b}"]
        );
        assert_eq!(formulas_in("$$x^2$$"), vec!["x^2"]);
        // Prices and lone dollars stay text.
        assert!(formulas_in("Harga $5 dan $10").is_empty());
        assert!(formulas_in("biaya $ 5 $ total").is_empty());
        // `_` inside LaTeX is not italic.
        let rtf = convert("$a_i b_j$");
        assert!(!rtf.contains("\\i "), "{rtf}");
    }

    #[test]
    fn display_math_alone_counts_as_markdown() {
        assert!(looks_like_markdown("\\[\nx^2\n\\]"));
        assert!(looks_like_markdown("$$E=mc^2$$"));
        assert!(looks_like_markdown("Persamaan $x^2$ dan **tebal**"));
        assert!(!looks_like_markdown("Harga $5 saja"));
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
