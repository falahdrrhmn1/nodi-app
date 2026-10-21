//! Note (Rich Edit RTF) → Markdown, for export and the read-only Markdown view.
//!
//! The conversion is one-way and never touches the note itself. Formatting
//! Markdown cannot express (colours, highlight, fonts, alignment) is dropped
//! and reported in [`Export::lossy`]. Formulas become `$LaTeX$`, pictures are
//! returned as separate files, one-cell Consolas tables become fenced code
//! blocks, other tables become GFM tables.

use std::collections::{BTreeSet, HashMap};

/// Result of a conversion.
#[derive(Debug, Default)]
pub struct Export {
    pub markdown: String,
    /// Pictures referenced from the Markdown: (file name, bytes).
    pub assets: Vec<(String, Vec<u8>)>,
    /// Human-readable notes about formatting that could not be kept.
    pub lossy: Vec<String>,
}

/// Converts RTF to Markdown. `asset_dir` is the folder name used in image
/// links (e.g. `"My Note_files"`).
pub fn rtf_to_markdown(rtf: &str, asset_dir: &str) -> Export {
    let document = Reader::new(rtf).read();
    Writer::new(asset_dir).write(&document)
}

// ------------------------------------------------------------------ model

#[derive(Debug, Clone, Default, PartialEq)]
struct Format {
    bold: bool,
    italic: bool,
    strike: bool,
    underline: bool,
    code: bool,
    size: i32,
}

#[derive(Debug, Clone)]
enum Inline {
    Text(String, Format, Option<String>),
    Picture(Vec<u8>),
    /// A formula picture (LaTeX from its `nodi-math:` link).
    Math(String),
}

#[derive(Debug, Clone, Default)]
struct Paragraph {
    inlines: Vec<Inline>,
    list: Option<bool>, // Some(true) = numbered
}

#[derive(Debug)]
enum Block {
    Paragraph(Paragraph),
    Table(Vec<Vec<Vec<Paragraph>>>), // rows → cells → paragraphs
}

// ----------------------------------------------------------------- reader

#[derive(Debug, Clone)]
struct State {
    format: Format,
    font: i32,
    skip: bool,
    uc: usize,
    destination: Destination,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Destination {
    Normal,
    FontTable,
    FieldInstruction,
    FieldResult,
    Picture,
    ListText,
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    stack: Vec<State>,
    state: State,
    fonts: HashMap<i32, String>,
    font_name: String,
    blocks: Vec<Block>,
    paragraph: Paragraph,
    in_table: bool,
    cell: Vec<Paragraph>,
    row: Vec<Vec<Paragraph>>,
    rows: Vec<Vec<Vec<Paragraph>>>,
    instruction: String,
    link: Option<String>,
    field_depth: Option<usize>,
    picture_hex: Vec<u8>,
    picture_depth: Option<usize>,
    pending_skip: usize,
    high_surrogate: Option<u16>,
    lossy: BTreeSet<&'static str>,
}

impl<'a> Reader<'a> {
    fn new(rtf: &'a str) -> Self {
        let state = State {
            format: Format {
                size: 22,
                ..Format::default()
            },
            font: 0,
            skip: false,
            uc: 1,
            destination: Destination::Normal,
        };
        Self {
            bytes: rtf.as_bytes(),
            pos: 0,
            stack: Vec::new(),
            state,
            fonts: HashMap::new(),
            font_name: String::new(),
            blocks: Vec::new(),
            paragraph: Paragraph::default(),
            in_table: false,
            cell: Vec::new(),
            row: Vec::new(),
            rows: Vec::new(),
            instruction: String::new(),
            link: None,
            field_depth: None,
            picture_hex: Vec::new(),
            picture_depth: None,
            pending_skip: 0,
            high_surrogate: None,
            lossy: BTreeSet::new(),
        }
    }

    fn read(mut self) -> (Vec<Block>, BTreeSet<&'static str>) {
        while self.pos < self.bytes.len() {
            let byte = self.bytes[self.pos];
            match byte {
                b'{' => {
                    self.pos += 1;
                    self.stack.push(self.state.clone());
                }
                b'}' => {
                    self.pos += 1;
                    self.end_group();
                }
                b'\\' => self.control(),
                b'\r' | b'\n' => self.pos += 1,
                _ => {
                    self.pos += 1;
                    if self.pending_skip > 0 {
                        self.pending_skip -= 1;
                        continue;
                    }
                    self.text_byte(byte);
                }
            }
        }
        self.finish_paragraph();
        self.close_table();
        (self.blocks, self.lossy)
    }

    fn end_group(&mut self) {
        let depth = self.stack.len();
        if self.state.destination == Destination::FontTable && !self.font_name.is_empty() {
            let name = self.font_name.trim().trim_end_matches(';').to_owned();
            self.fonts.insert(self.state.font, name);
            self.font_name.clear();
        }
        if self.picture_depth == Some(depth) {
            self.picture_depth = None;
            let hex = std::mem::take(&mut self.picture_hex);
            let bytes = decode_hex(&hex);
            let math = self.link.as_deref().and_then(crate::links::parse_math_link);
            if let Some(latex) = math {
                self.push_inline(Inline::Math(latex));
            } else if !bytes.is_empty() {
                self.push_inline(Inline::Picture(bytes));
            }
        }
        if self.field_depth == Some(depth) {
            self.field_depth = None;
            self.link = None;
        }
        if let Some(previous) = self.stack.pop() {
            let leaving = self.state.destination;
            self.state = previous;
            if leaving == Destination::FieldInstruction
                && self.state.destination != Destination::FieldInstruction
            {
                self.link = parse_hyperlink(&self.instruction);
                self.instruction.clear();
            }
        }
    }

    fn control(&mut self) {
        self.pos += 1; // backslash
        let Some(&first) = self.bytes.get(self.pos) else {
            return;
        };
        if !first.is_ascii_alphabetic() {
            self.pos += 1;
            match first {
                b'\'' => {
                    let hex = self.bytes.get(self.pos..self.pos + 2).unwrap_or(b"3f");
                    self.pos += 2;
                    let value = u8::from_str_radix(std::str::from_utf8(hex).unwrap_or("3f"), 16)
                        .unwrap_or(b'?');
                    if self.pending_skip > 0 {
                        self.pending_skip -= 1;
                    } else {
                        self.text_char(cp1252(value));
                    }
                }
                b'*' => self.state.skip = true,
                b'~' => self.text_char('\u{00A0}'),
                b'_' => self.text_char('-'),
                b'-' => {}
                b'\\' | b'{' | b'}' => self.text_char(first as char),
                _ => {}
            }
            return;
        }
        let start = self.pos;
        while self
            .bytes
            .get(self.pos)
            .is_some_and(u8::is_ascii_alphabetic)
        {
            self.pos += 1;
        }
        let word = std::str::from_utf8(&self.bytes[start..self.pos])
            .unwrap_or("")
            .to_owned();
        let number_start = self.pos;
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        while self.bytes.get(self.pos).is_some_and(u8::is_ascii_digit) {
            self.pos += 1;
        }
        let param: Option<i32> = std::str::from_utf8(&self.bytes[number_start..self.pos])
            .ok()
            .and_then(|text| text.parse().ok());
        if self.bytes.get(self.pos) == Some(&b' ') {
            self.pos += 1;
        }
        self.apply(&word, param);
    }

    fn apply(&mut self, word: &str, param: Option<i32>) {
        let on = param != Some(0);
        match word {
            "fonttbl" => self.state.destination = Destination::FontTable,
            "colortbl" | "stylesheet" | "info" | "generator" | "listtable"
            | "listoverridetable" | "themedata" | "latentstyles" | "pn" | "picprop" | "rsidtbl"
            | "xmlnstbl" | "mmathPr" => self.state.skip = true,
            "f" => {
                let index = param.unwrap_or(0);
                self.state.font = index;
                if self.state.destination != Destination::FontTable {
                    self.state.format.code = self
                        .fonts
                        .get(&index)
                        .is_some_and(|name| is_monospace(name));
                }
            }
            "fmodern" if self.state.destination == Destination::FontTable => {
                self.font_name.push_str("monospace:");
            }
            "fs" => self.state.format.size = param.unwrap_or(22),
            "b" => self.state.format.bold = on,
            "i" => self.state.format.italic = on,
            "strike" | "striked" => self.state.format.strike = on,
            "ul" | "uld" | "uldb" | "ulw" => self.state.format.underline = on,
            "ulnone" => self.state.format.underline = false,
            "plain" => {
                self.state.font = 0;
                self.state.format = Format {
                    size: 22,
                    ..Format::default()
                };
            }
            "cf" if on => {
                self.lossy.insert("text colours");
            }
            "highlight" | "chcbpat" | "cb" if on => {
                if !self.state.format.code {
                    self.lossy.insert("highlighting / text background");
                }
            }
            "qc" | "qr" | "qj" => {
                self.lossy.insert("paragraph alignment");
            }
            "par" => self.finish_paragraph(),
            "line" => self.text_char('\n'),
            "tab" => self.text_char('\t'),
            "emdash" => self.text_char('—'),
            "endash" => self.text_char('–'),
            "bullet" => self.text_char('•'),
            "lquote" => self.text_char('‘'),
            "rquote" => self.text_char('’'),
            "ldblquote" => self.text_char('“'),
            "rdblquote" => self.text_char('”'),
            "uc" => self.state.uc = param.unwrap_or(1).max(0) as usize,
            "u" => {
                let value = param.unwrap_or(63);
                let unit = if value < 0 {
                    (value + 65536) as u16
                } else {
                    value as u16
                };
                self.unicode_unit(unit);
                self.pending_skip = self.state.uc;
            }
            "pard" => {
                self.in_table = false;
                self.paragraph.list = None;
            }
            "intbl" => self.in_table = true,
            "pntext" => {
                self.state.destination = Destination::ListText;
                self.state.skip = true;
            }
            "cell" | "nestcell" => {
                self.finish_paragraph_into_cell();
                let cell = std::mem::take(&mut self.cell);
                self.row.push(cell);
            }
            "row" | "nestrow" => {
                let row = std::mem::take(&mut self.row);
                if !row.is_empty() {
                    self.rows.push(row);
                }
            }
            "field" => self.field_depth = Some(self.stack.len()),
            "fldinst" => {
                self.state.destination = Destination::FieldInstruction;
                self.state.skip = false;
                self.instruction.clear();
            }
            "fldrslt" => self.state.destination = Destination::FieldResult,
            "pict" => {
                self.state.destination = Destination::Picture;
                self.picture_depth = Some(self.stack.len());
                self.picture_hex.clear();
            }
            _ => {}
        }
        // `\pntext` content tells whether the list is numbered.
        if word == "pntext" {
            self.paragraph.list = Some(false);
        }
    }

    fn unicode_unit(&mut self, unit: u16) {
        if (0xD800..0xDC00).contains(&unit) {
            self.high_surrogate = Some(unit);
            return;
        }
        if let Some(high) = self.high_surrogate.take() {
            if let Some(Ok(ch)) = char::decode_utf16([high, unit]).next() {
                self.text_char(ch);
            }
            return;
        }
        if let Some(ch) = char::from_u32(u32::from(unit)) {
            self.text_char(ch);
        }
    }

    fn text_byte(&mut self, byte: u8) {
        if self.state.destination == Destination::Picture {
            // `{\*\picprop ...}` text inside the picture is not image data.
            if !self.state.skip && byte.is_ascii_hexdigit() {
                self.picture_hex.push(byte);
            }
            return;
        }
        self.text_char(byte as char);
    }

    fn text_char(&mut self, ch: char) {
        match self.state.destination {
            Destination::FontTable => {
                self.font_name.push(ch);
                return;
            }
            Destination::FieldInstruction => {
                self.instruction.push(ch);
                return;
            }
            Destination::ListText => {
                if ch.is_ascii_digit() {
                    self.paragraph.list = Some(true);
                }
                return;
            }
            Destination::Picture => return,
            _ => {}
        }
        if self.state.skip {
            return;
        }
        let format = self.state.format.clone();
        let link = if self.state.destination == Destination::FieldResult {
            self.link.clone()
        } else {
            None
        };
        if let Some(Inline::Text(text, last_format, last_link)) = self.paragraph.inlines.last_mut()
        {
            if *last_format == format && *last_link == link {
                text.push(ch);
                return;
            }
        }
        self.paragraph
            .inlines
            .push(Inline::Text(ch.to_string(), format, link));
    }

    fn push_inline(&mut self, inline: Inline) {
        self.paragraph.inlines.push(inline);
    }

    fn finish_paragraph(&mut self) {
        if self.in_table {
            self.finish_paragraph_into_cell();
            return;
        }
        self.close_table();
        let paragraph = std::mem::take(&mut self.paragraph);
        self.blocks.push(Block::Paragraph(paragraph));
    }

    fn finish_paragraph_into_cell(&mut self) {
        let paragraph = std::mem::take(&mut self.paragraph);
        if !paragraph.inlines.is_empty() || !self.cell.is_empty() {
            self.cell.push(paragraph);
        }
    }

    fn close_table(&mut self) {
        if !self.row.is_empty() {
            let row = std::mem::take(&mut self.row);
            self.rows.push(row);
        }
        if !self.rows.is_empty() {
            let rows = std::mem::take(&mut self.rows);
            self.blocks.push(Block::Table(rows));
        }
    }
}

fn is_monospace(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("monospace:")
        || ["consolas", "courier", "cascadia", "lucida console", "mono"]
            .iter()
            .any(|known| lower.contains(known))
}

fn parse_hyperlink(instruction: &str) -> Option<String> {
    let at = instruction.find("HYPERLINK")?;
    let rest = &instruction[at + "HYPERLINK".len()..];
    let open = rest.find('"')?;
    let rest = &rest[open + 1..];
    let close = rest.find('"')?;
    Some(rest[..close].to_owned())
}

fn decode_hex(hex: &[u8]) -> Vec<u8> {
    hex.as_chunks::<2>()
        .0
        .iter()
        .filter_map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|text| u8::from_str_radix(text, 16).ok())
        })
        .collect()
}

/// Windows-1252 byte → char (Rich Edit writes `\ansicpg1252`).
fn cp1252(byte: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    if (0x80..0xA0).contains(&byte) {
        HIGH[(byte - 0x80) as usize]
    } else {
        byte as char
    }
}

// ----------------------------------------------------------------- writer

struct Writer {
    asset_dir: String,
    assets: Vec<(String, Vec<u8>)>,
    lossy: BTreeSet<&'static str>,
}

impl Writer {
    fn new(asset_dir: &str) -> Self {
        Self {
            asset_dir: asset_dir.to_owned(),
            assets: Vec::new(),
            lossy: BTreeSet::new(),
        }
    }

    fn write(mut self, document: &(Vec<Block>, BTreeSet<&'static str>)) -> Export {
        let (blocks, reader_lossy) = document;
        self.lossy.extend(reader_lossy.iter().copied());
        let mut out = String::new();
        let mut previous_list = false;
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => {
                    let line = self.paragraph(paragraph);
                    let is_list = paragraph.list.is_some();
                    let empty = line.trim().is_empty();
                    if empty {
                        previous_list = false;
                        continue;
                    }
                    if !out.is_empty() {
                        // List items stay together; everything else is its own
                        // Markdown paragraph.
                        out.push_str(if is_list && previous_list {
                            "\n"
                        } else {
                            "\n\n"
                        });
                    }
                    out.push_str(&line);
                    previous_list = is_list;
                }
                Block::Table(rows) => {
                    if !out.is_empty() {
                        out.push_str("\n\n");
                    }
                    out.push_str(&self.table(rows));
                    previous_list = false;
                }
            }
        }
        out.push('\n');
        Export {
            markdown: out,
            assets: self.assets,
            lossy: self.lossy.iter().map(|text| (*text).to_owned()).collect(),
        }
    }

    fn heading_level(paragraph: &Paragraph) -> Option<usize> {
        let mut sizes = Vec::new();
        for inline in &paragraph.inlines {
            match inline {
                Inline::Text(text, format, _) if !text.trim().is_empty() => {
                    if !format.bold || format.size <= 22 {
                        return None;
                    }
                    sizes.push(format.size);
                }
                Inline::Picture(_) => return None,
                _ => {}
            }
        }
        let size = *sizes.iter().max()?;
        Some(if size >= 40 {
            1
        } else if size >= 32 {
            2
        } else if size >= 28 {
            3
        } else {
            4
        })
    }

    fn paragraph(&mut self, paragraph: &Paragraph) -> String {
        if let Some(level) = Self::heading_level(paragraph) {
            let text: String = paragraph
                .inlines
                .iter()
                .filter_map(|inline| match inline {
                    Inline::Text(text, _, _) => Some(text.as_str()),
                    Inline::Picture(_) | Inline::Math(_) => None,
                })
                .collect();
            return format!("{} {}", "#".repeat(level), escape(text.trim()));
        }
        let body = self.inlines(&paragraph.inlines);
        match paragraph.list {
            Some(true) => format!("1. {}", body.trim_start()),
            Some(false) => format!("- {}", body.trim_start()),
            None => {
                // Leading tabs would turn into an indented code block.
                let trimmed = body.trim_start_matches(['\t', ' ']);
                let indent = body.len() - trimmed.len();
                if indent > 0 {
                    format!("{}{}", "\u{00A0}\u{00A0}".repeat(indent), trimmed)
                } else {
                    body
                }
            }
        }
    }

    fn inlines(&mut self, inlines: &[Inline]) -> String {
        let mut out = String::new();
        for inline in inlines {
            match inline {
                Inline::Text(text, format, link) => {
                    out.push_str(&self.run(text, format, link.as_deref()));
                }
                Inline::Picture(bytes) => out.push_str(&self.picture(bytes)),
                Inline::Math(latex) => out.push_str(&format!("${}$", latex.trim())),
            }
        }
        out
    }

    fn run(&mut self, text: &str, format: &Format, link: Option<&str>) -> String {
        if text.is_empty() {
            return String::new();
        }
        if format.size != 22 && !format.code && !format.bold {
            self.lossy.insert("font sizes");
        }
        let lead = &text[..text.len() - text.trim_start().len()];
        let trail = &text[text.trim_end().len()..];
        let core = text.trim();
        if core.is_empty() {
            return text.replace('\n', "  \n");
        }
        let mut body = if format.code {
            let fence = if core.contains('`') { "``" } else { "`" };
            format!("{fence}{core}{fence}")
        } else {
            escape(core).replace('\n', "  \n")
        };
        if format.strike {
            body = format!("~~{body}~~");
        }
        if format.italic {
            body = format!("*{body}*");
        }
        if format.bold {
            body = format!("**{body}**");
        }
        if format.underline && link.is_none() {
            body = format!("<u>{body}</u>");
        }
        if let Some(target) = link {
            let target = target.replace(' ', "%20").replace(')', "%29");
            body = format!("[{body}]({target})");
        }
        format!("{lead}{body}{trail}")
    }

    fn picture(&mut self, bytes: &[u8]) -> String {
        if let Some(latex) = crate::math::latex_from_png(bytes) {
            return format!("${}$", latex.trim());
        }
        let extension = match crate::image::sniff(bytes).map(|info| info.format) {
            Some(crate::image::ImageFormat::Jpeg) => "jpg",
            Some(crate::image::ImageFormat::Gif) => "gif",
            Some(crate::image::ImageFormat::Bmp) => "bmp",
            _ => "png",
        };
        let name = format!("image-{}.{extension}", self.assets.len() + 1);
        self.assets.push((name.clone(), bytes.to_vec()));
        self.lossy.insert("picture display sizes");
        format!("![]({}/{})", self.asset_dir.replace(' ', "%20"), name)
    }

    fn table(&mut self, rows: &[Vec<Vec<Paragraph>>]) -> String {
        // One cell whose text is all code: a code block.
        if rows.len() == 1 && rows[0].len() == 1 {
            let cell = &rows[0][0];
            let all_code = cell.iter().all(|paragraph| {
                paragraph.inlines.iter().all(|inline| match inline {
                    Inline::Text(text, format, _) => format.code || text.trim().is_empty(),
                    Inline::Picture(_) | Inline::Math(_) => false,
                })
            });
            let any_code = cell.iter().any(|paragraph| {
                paragraph.inlines.iter().any(|inline| {
                    matches!(inline, Inline::Text(text, format, _) if format.code && !text.trim().is_empty())
                })
            });
            let all_code = all_code && any_code;
            if all_code {
                let code: Vec<String> = cell
                    .iter()
                    .map(|paragraph| {
                        paragraph
                            .inlines
                            .iter()
                            .filter_map(|inline| match inline {
                                Inline::Text(text, _, _) => Some(text.as_str()),
                                Inline::Picture(_) | Inline::Math(_) => None,
                            })
                            .collect()
                    })
                    .collect();
                let body = code.join("\n");
                let fence = if body.contains("```") { "````" } else { "```" };
                return format!("{fence}\n{body}\n{fence}");
            }
        }
        let columns = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let mut lines = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let mut cells: Vec<String> = row
                .iter()
                .map(|cell| {
                    cell.iter()
                        .map(|paragraph| self.inlines(&paragraph.inlines))
                        .filter(|text| !text.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join("<br>")
                        .replace('|', "\\|")
                        .replace('\n', "<br>")
                })
                .collect();
            cells.resize(columns, String::new());
            lines.push(format!("| {} |", cells.join(" | ")));
            if index == 0 {
                lines.push(format!("|{}", " --- |".repeat(columns)));
            }
        }
        lines.join("\n")
    }
}

/// Escapes characters that would otherwise start Markdown syntax.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(
            ch,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '#' | '|'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = r"{\rtf1\ansi\ansicpg1252\deff0\nouicompat{\fonttbl{\f0\fnil Segoe UI;}{\f1\fmodern Consolas;}}{\colortbl ;\red255\green0\blue0;}";

    fn convert(body: &str) -> Export {
        rtf_to_markdown(&format!("{HEADER}{body}}}"), "note_files")
    }

    #[test]
    fn headings_emphasis_and_code() {
        let out = convert(
            r"\pard\f0\fs40\b Machine Learning\b0\fs22\par Plain \b bold\b0  and \i it\i0  and \f1 code\f0\par",
        );
        assert!(
            out.markdown.starts_with("# Machine Learning\n\n"),
            "{}",
            out.markdown
        );
        assert!(
            out.markdown.contains("Plain **bold** and *it* and `code`"),
            "{}",
            out.markdown
        );
    }

    #[test]
    fn unicode_and_cp1252() {
        let out = convert(r"\pard\f0 caf\'e9 \u8721? \u-10179?\u-8704?\par");
        assert!(out.markdown.contains("café ∑ 😀"), "{}", out.markdown);
    }

    #[test]
    fn links_lists_and_lossy_colour() {
        let out = convert(
            r#"\pard{\pntext\f0 1.\tab}{\*\pn\pnlvlbody\pndec}first\par{\pntext\f0 2.\tab}second\par\pard{\field{\*\fldinst{HYPERLINK "nodi://note/abc"}}{\fldrslt{Gradient}}}\par\cf1 red\cf0\par"#,
        );
        assert!(
            out.markdown.contains("1. first\n1. second"),
            "{}",
            out.markdown
        );
        assert!(
            out.markdown.contains("[Gradient](nodi://note/abc)"),
            "{}",
            out.markdown
        );
        assert!(out.lossy.iter().any(|note| note.contains("colour")));
    }

    #[test]
    fn code_block_table_and_grid_table() {
        let code = convert(
            r"\trowd\cellx9000\pard\intbl\f1 fn main() \{\par     x();\cell\row\pard\f0 after\par",
        );
        assert!(
            code.markdown.contains("```\nfn main() {\n    x();\n```"),
            "{}",
            code.markdown
        );
        let grid = convert(
            r"\trowd\cellx3000\cellx6000\pard\intbl Nama\cell Nilai\cell\row\trowd\cellx3000\cellx6000\pard\intbl Model A\cell 92%\cell\row\pard after\par",
        );
        assert!(
            grid.markdown
                .contains("| Nama | Nilai |\n| --- | --- |\n| Model A | 92% |"),
            "{}",
            grid.markdown
        );
    }

    #[test]
    fn pictures_and_formulas() {
        let formula = crate::math::encode_png(1, 1, &[0, 0], r"E = mc^2");
        let hex: String = formula.iter().map(|byte| format!("{byte:02x}")).collect();
        // A bare PNG header without any Nodi text chunk: an ordinary picture.
        let plain: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0";
        let plain_hex: String = plain.iter().map(|byte| format!("{byte:02x}")).collect();
        let out = convert(&format!(
            r"\pard Energy {{\pict{{\*\picprop{{\sp{{\sn wzDescription}}{{\sv x}}}}}}\pngblip\picwgoal100 {hex}}} and {{\pict\pngblip {plain_hex}}}\par"
        ));
        assert!(
            out.markdown
                .contains("Energy $E = mc^2$ and ![](note_files/image-1.png)"),
            "{}",
            out.markdown
        );
        assert_eq!(out.assets.len(), 1);
    }

    #[test]
    fn real_rich_edit_code_block_and_table() {
        let code = r#"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fnil Segoe UI;}{\f1\fmodern Consolas;}}{\colortbl ;\red0\green0\blue255;}{\*\generator Riched20 10.0.26100}\viewkind4\uc1\trowd\trgaph144\trleft0\clcbpat3\clbrdrl\brdrw10\brdrs\brdrcf2 \cellx16095 \pard\intbl\f1\fs20 fn main() \{\par x();\cell\row \pard\f0\fs22\par\par}"#;
        let out = rtf_to_markdown(code, "f");
        assert!(
            out.markdown.contains("```\nfn main() {\nx();\n```"),
            "{}",
            out.markdown
        );
        let table = r#"{\rtf1\ansi\deff0{\fonttbl{\f0\fnil Segoe UI;}}\viewkind4\uc1\trowd\trgaph108\trpaddl108\clcfpat4\clcbpat3\clbrdrl\brdrw10\brdrs\brdrcf2 \cellx8047\clcfpat4\clcbpat3 \cellx16094 \pard\intbl\f0\fs22 Nama\cell Nilai\cell\row\trowd\trgaph108\clcfpat4 \cellx8047\clcfpat4 \cellx16094 \pard\intbl A\cell 92%\cell\row \pard\par\par}"#;
        let out = rtf_to_markdown(table, "f");
        assert!(
            out.markdown
                .contains("| Nama | Nilai |\n| --- | --- |\n| A | 92% |"),
            "{}",
            out.markdown
        );
    }

    #[test]
    fn markdown_specials_are_escaped() {
        let out = convert(r"\pard 2 * 3 _x_ [a]\par");
        assert!(
            out.markdown.contains(r"2 \* 3 \_x\_ \[a\]"),
            "{}",
            out.markdown
        );
    }
}
