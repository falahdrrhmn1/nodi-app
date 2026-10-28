//! Clipboard "HTML Format" (CF_HTML) → Markdown, so pasting from ChatGPT, a
//! web page or a documentation site keeps its structure. The result is fed to
//! [`crate::markdown_in`], so both paths share one RTF writer.
//!
//! The parser is deliberately small and tolerant: a tag scanner with a stack
//! of list depths, formatting flags and table cells. It never fetches
//! anything; `<img>` becomes a link to its source.

/// Converts a raw CF_HTML byte stream. The fragment is located through the
/// `StartFragment`/`EndFragment` header offsets; a document without them is
/// scanned from `<body>` (or from the start).
pub fn cf_html_to_markdown(bytes: &[u8]) -> String {
    let fragment = fragment(bytes);
    let html = String::from_utf8_lossy(fragment);
    html_to_markdown(&html)
}

/// Converts an HTML fragment or document to Markdown.
pub fn html_to_markdown(html: &str) -> String {
    let mut parser = Parser::new();
    parser.run(html);
    parser.finish()
}

/// Slice of `bytes` holding the HTML fragment (CF_HTML offsets are bytes).
fn fragment(bytes: &[u8]) -> &[u8] {
    let text = String::from_utf8_lossy(bytes);
    let head: String = text.chars().take(2048).collect();
    let start = header_offset(&head, "StartFragment");
    let end = header_offset(&head, "EndFragment");
    if let (Some(start), Some(end)) = (start, end) {
        if start < end
            && end <= bytes.len()
            && start < bytes.len()
            && std::str::from_utf8(&bytes[start..end]).is_ok()
        {
            return &bytes[start..end];
        }
    }
    // No usable offsets: use the body, or everything.
    let lower = text.to_ascii_lowercase();
    let begin = lower
        .find("<body")
        .and_then(|at| lower[at..].find('>').map(|gt| at + gt + 1))
        .or_else(|| lower.find("<html").map(|_| 0))
        .unwrap_or(0);
    let close = lower.rfind("</body>").unwrap_or(bytes.len());
    let close = close.max(begin).min(bytes.len());
    &bytes[begin.min(close)..close]
}

fn header_offset(head: &str, name: &str) -> Option<usize> {
    let at = head.find(name)?;
    let rest = &head[at + name.len()..];
    let rest = rest.trim_start_matches([':', ' ', '\t']);
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|value| *value > 0)
}

// --------------------------------------------------------------------- parser

#[derive(Default)]
struct TableData {
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    cell: Option<String>,
}

struct Parser {
    out: String,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    quote: usize,
    /// `(ordered, number)` per open list.
    lists: Vec<(bool, usize)>,
    in_item: bool,
    link: Option<String>,
    pre: Option<String>,
    pre_language: String,
    /// Element being skipped and how many of its kind are open inside it.
    skip: Option<(String, usize)>,
    table: Option<TableData>,
    /// Inside `<math>` / a MathJax script: the TeX collected so far
    /// (`display`, text). Only TeX sources are read; the visual MathML is not.
    math: Option<(bool, String)>,
    /// Inside a TeX `<annotation>` / `<script type="math/tex">`.
    math_source: bool,
}

impl Parser {
    fn new() -> Self {
        Self {
            out: String::new(),
            bold: false,
            italic: false,
            strike: false,
            code: false,
            quote: 0,
            lists: Vec::new(),
            in_item: false,
            link: None,
            pre: None,
            pre_language: String::new(),
            skip: None,
            table: None,
            math: None,
            math_source: false,
        }
    }

    fn run(&mut self, html: &str) {
        let mut index = 0;
        while index < html.len() {
            let rest = &html[index..];
            let Some(lt) = rest.find('<') else {
                self.text(rest);
                break;
            };
            self.text(&rest[..lt]);
            let after = &rest[lt..];
            if let Some(comment) = after.strip_prefix("<!--") {
                match comment.find("-->") {
                    Some(end) => index += lt + 4 + end + 3,
                    None => break,
                }
                continue;
            }
            let Some(gt) = after.find('>') else {
                self.text(after);
                break;
            };
            self.tag(&after[1..gt]);
            index += lt + gt + 1;
        }
    }

    fn tag(&mut self, raw: &str) {
        let raw = raw.trim();
        if raw.is_empty() {
            return;
        }
        let closing = raw.starts_with('/');
        let raw = raw.trim_start_matches('/').trim_end_matches('/');
        let name_end = raw.find(|ch: char| ch.is_whitespace()).unwrap_or(raw.len());
        let name = raw[..name_end].to_ascii_lowercase();
        let attrs = &raw[name_end..];

        if let Some((skip, depth)) = self.skip.as_mut() {
            if name == *skip {
                if closing {
                    *depth -= 1;
                    if *depth == 0 {
                        self.skip = None;
                    }
                } else if !raw.ends_with('/') {
                    *depth += 1;
                }
            }
            return;
        }
        if self.math_tag(&name, attrs, closing) {
            return;
        }
        if !closing && attr(attrs, "aria-hidden").is_some_and(|value| value == "true") {
            // Decorative duplicates, e.g. KaTeX's visual rendering next to the
            // MathML that carries the TeX. Void elements have nothing inside.
            if !is_void(&name) {
                self.skip_element(name);
            }
            return;
        }
        if matches!(name.as_str(), "script" | "style") {
            if !closing {
                self.skip_element(name);
            }
            return;
        }
        if let Some(pre) = self.pre.clone() {
            if name == "code" && !closing {
                if let Some(language) = attr(attrs, "class").and_then(|class| {
                    class
                        .split_whitespace()
                        .find_map(|part| part.strip_prefix("language-").map(str::to_owned))
                }) {
                    self.pre_language = language;
                }
            } else if closing && name == "pre" {
                let body = pre.trim_end_matches(['\n', '\r']);
                let language = std::mem::take(&mut self.pre_language);
                self.block();
                self.out
                    .push_str(&format!("```{language}\n{body}\n```\n\n"));
                self.pre = None;
            }
            return;
        }
        if name == "pre" && !closing {
            self.block();
            self.pre = Some(String::new());
            return;
        }

        match name.as_str() {
            "head" => {
                if !closing {
                    self.skip_element(name);
                }
            }
            "p" | "div" | "section" | "article" | "header" | "footer" | "main" => {
                if self.lists.is_empty() {
                    self.block();
                } else {
                    self.space();
                }
            }
            "br" => {
                if let Some(pre) = self.pre.as_mut() {
                    pre.push('\n');
                } else {
                    self.out.push_str("  \n");
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if closing {
                    self.block();
                } else {
                    let level = name[1..].parse::<usize>().unwrap_or(1).min(4);
                    self.block();
                    self.quote_prefix();
                    self.out.push_str(&"#".repeat(level));
                    self.out.push(' ');
                }
            }
            "strong" | "b" => self.emphasis("**", closing),
            "em" | "i" => self.emphasis("*", closing),
            "s" | "del" | "strike" => self.emphasis("~~", closing),
            "code" => {
                if self.table.is_some() {
                    return;
                }
                if closing {
                    if self.code {
                        self.emit("`");
                        self.code = false;
                    }
                } else if !self.code {
                    self.emit("`");
                    self.code = true;
                }
            }
            "a" => {
                if closing {
                    if let Some(target) = self.link.take() {
                        let label = ""; // the text was already emitted
                        let _ = label;
                        self.emit(&format!("]({target})"));
                    }
                } else if let Some(target) = attr(attrs, "href") {
                    let target = target.trim();
                    // Only addresses that may become links (no `javascript:`…);
                    // the others stay plain text.
                    if !target.is_empty()
                        && !target.starts_with('#')
                        && crate::links::pasted_link_allowed(target)
                    {
                        self.emit("[");
                        self.link = Some(target.to_owned());
                    }
                }
            }
            "img" => {
                let src = attr(attrs, "src").unwrap_or_default();
                if !src.trim().is_empty() {
                    let alt = attr(attrs, "alt").unwrap_or_default();
                    let text = if alt.trim().is_empty() {
                        src.clone()
                    } else {
                        alt
                    };
                    self.emit(&format!("[{}]({})", text.trim(), src.trim()));
                }
            }
            "ul" | "ol" => {
                if closing {
                    self.lists.pop();
                    self.in_item = false;
                    self.block();
                } else {
                    self.block();
                    self.lists.push((name == "ol", 0));
                }
            }
            "li" => {
                if closing {
                    self.in_item = false;
                    self.line_break();
                } else {
                    self.line_break();
                    let depth = self.lists.len().saturating_sub(1);
                    let ordered = self.lists.last_mut();
                    let marker = match ordered {
                        Some((true, number)) => {
                            *number += 1;
                            format!("{number}. ")
                        }
                        _ => "- ".to_owned(),
                    };
                    self.out.push_str(&"  ".repeat(depth));
                    self.out.push_str(&marker);
                    self.in_item = true;
                }
            }
            "blockquote" => {
                if closing {
                    self.quote = self.quote.saturating_sub(1);
                    self.block();
                } else {
                    self.block();
                    self.quote += 1;
                }
            }
            "hr" => {
                self.block();
                self.out.push_str("---\n\n");
            }
            "table" => {
                if closing {
                    self.flush_table();
                } else {
                    self.block();
                    self.table = Some(TableData::default());
                }
            }
            "tr" => {
                if let Some(table) = self.table.as_mut() {
                    if closing {
                        if !table.row.is_empty() {
                            let row = std::mem::take(&mut table.row);
                            table.rows.push(row);
                        }
                    } else {
                        table.row.clear();
                    }
                }
            }
            "td" | "th" => {
                if let Some(table) = self.table.as_mut() {
                    if closing {
                        let cell = table.cell.take().unwrap_or_default();
                        table.row.push(cell.trim().to_owned());
                    } else {
                        table.cell = Some(String::new());
                    }
                }
            }
            _ => {}
        }
    }

    fn skip_element(&mut self, name: String) {
        self.skip = Some((name, 1));
    }

    /// Formulas: Wikipedia's `<math alttext>`, an element with `data-latex`,
    /// KaTeX/ChatGPT MathML with a TeX `<annotation>`, and MathJax's
    /// `<script type="math/tex">`. They become `$…$` (`$$…$$` for display
    /// math), which `markdown_in` turns into formulas. Returns true when the
    /// tag was consumed.
    fn math_tag(&mut self, name: &str, attrs: &str, closing: bool) -> bool {
        if let Some((display, tex)) = self.math.as_mut() {
            let ends = closing && (name == "math" || (name == "script" && self.math_source));
            if name == "annotation" {
                self.math_source = !closing
                    && attr(attrs, "encoding").is_some_and(|encoding| encoding.contains("tex"));
            }
            if ends {
                let (display, tex) = (*display, std::mem::take(tex));
                self.math = None;
                self.math_source = false;
                self.emit_math(&tex, display);
            }
            return true;
        }
        if closing {
            return false;
        }
        if let Some(tex) = attr(attrs, "data-latex") {
            self.emit_math(&tex, false);
            self.skip_element(name.to_owned());
            return true;
        }
        if name == "math" {
            let display = attr(attrs, "display").is_some_and(|value| value == "block");
            if let Some(alt) = attr(attrs, "alttext") {
                let alt = alt.trim();
                let tex = alt
                    .strip_prefix("{\\displaystyle")
                    .and_then(|inner| inner.strip_suffix('}'))
                    .unwrap_or(alt);
                self.emit_math(tex, display);
                self.skip_element(name.to_owned());
                return true;
            }
            self.math = Some((display, String::new()));
            return true;
        }
        if name == "script" {
            if let Some(kind) = attr(attrs, "type").filter(|kind| kind.starts_with("math/tex")) {
                self.math = Some((kind.contains("mode=display"), String::new()));
                self.math_source = true;
                return true;
            }
        }
        false
    }

    fn emit_math(&mut self, tex: &str, display: bool) {
        let tex = tex.trim();
        if tex.is_empty() {
            return;
        }
        if display {
            self.block();
            self.out.push_str(&format!("$${tex}$$"));
            self.block();
        } else {
            self.emit(&format!("${tex}$"));
        }
    }

    /// Text node: decodes entities and collapses whitespace outside `<pre>`.
    fn text(&mut self, text: &str) {
        if self.skip.is_some() {
            return;
        }
        if let Some((_, tex)) = self.math.as_mut() {
            if self.math_source {
                tex.push_str(&decode_entities(text));
            }
            return;
        }
        let decoded = decode_entities(text);
        if let Some(pre) = self.pre.as_mut() {
            pre.push_str(&decoded);
            return;
        }
        if decoded.is_empty() {
            return;
        }
        let collapsed = collapse_whitespace(&decoded);
        if collapsed.is_empty() {
            return;
        }
        if let Some(table) = self.table.as_mut() {
            if let Some(cell) = table.cell.as_mut() {
                if !cell.is_empty() && !cell.ends_with(' ') && collapsed.starts_with(' ') {
                    cell.push_str(collapsed.trim_start());
                } else {
                    cell.push_str(&collapsed);
                }
                return;
            }
        }
        if self.out.is_empty() || self.out.ends_with(['\n', ' ']) {
            self.emit(collapsed.trim_start());
        } else {
            self.emit(&collapsed);
        }
    }

    fn emit(&mut self, text: &str) {
        if let Some(table) = self.table.as_mut() {
            if let Some(cell) = table.cell.as_mut() {
                cell.push_str(text);
                return;
            }
        }
        self.out.push_str(text);
    }

    fn emphasis(&mut self, marker: &str, closing: bool) {
        if closing {
            let flag = match marker {
                "**" => &mut self.bold,
                "~~" => &mut self.strike,
                _ => &mut self.italic,
            };
            if *flag {
                *flag = false;
                self.emit(marker);
            }
        } else {
            let flag = match marker {
                "**" => &mut self.bold,
                "~~" => &mut self.strike,
                _ => &mut self.italic,
            };
            if !*flag {
                *flag = true;
                self.emit(marker);
            }
        }
    }

    /// Ensures a blank line before a new block.
    fn block(&mut self) {
        if self.table.is_some() {
            return;
        }
        let trimmed = self.out.trim_end_matches([' ', '\t', '\n']).len();
        self.out.truncate(trimmed);
        if !self.out.is_empty() {
            self.out.push_str("\n\n");
        }
    }

    fn line_break(&mut self) {
        if self.table.is_some() {
            return;
        }
        let trimmed = self.out.trim_end_matches([' ', '\t']).len();
        self.out.truncate(trimmed);
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    fn space(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with([' ', '\n']) {
            self.emit(" ");
        }
    }

    fn quote_prefix(&mut self) {
        for _ in 0..self.quote {
            self.out.push_str("> ");
        }
    }

    fn flush_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        if table.rows.is_empty() {
            return;
        }
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
        self.block();
        for (index, row) in table.rows.iter().enumerate() {
            let mut cells = row.clone();
            cells.resize(columns, String::new());
            let cells: Vec<String> = cells
                .iter()
                .map(|cell| cell.replace('|', "\\|").replace('\n', " "))
                .collect();
            self.out.push_str(&format!("| {} |\n", cells.join(" | ")));
            if index == 0 {
                self.out.push_str(&format!("|{}", " --- |".repeat(columns)));
                self.out.push('\n');
            }
        }
        self.out.push('\n');
    }

    fn finish(mut self) -> String {
        if let Some(pre) = self.pre.take() {
            self.block();
            self.out
                .push_str(&format!("```\n{}\n```\n", pre.trim_end()));
        }
        self.flush_table();
        let mut text = self.out;
        while text.contains("\n\n\n") {
            text = text.replace("\n\n\n", "\n\n");
        }
        text.trim().to_owned()
    }
}

/// Elements without content or a closing tag.
fn is_void(name: &str) -> bool {
    matches!(
        name,
        "img" | "br" | "hr" | "input" | "meta" | "link" | "wbr" | "source" | "area" | "col"
    )
}

fn attr(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let mut search = 0;
    while let Some(found) = lower[search..].find(name) {
        let at = search + found;
        let before = lower[..at].chars().next_back();
        if before.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_') {
            search = at + name.len();
            continue;
        }
        let rest = attrs[at + name.len()..].trim_start();
        let rest = rest.strip_prefix('=').unwrap_or(rest).trim_start();
        let value = if let Some(rest) = rest.strip_prefix('"') {
            rest.split('"').next().unwrap_or("")
        } else if let Some(rest) = rest.strip_prefix('\'') {
            rest.split('\'').next().unwrap_or("")
        } else {
            rest.split(|ch: char| ch.is_whitespace() || ch == '>')
                .next()
                .unwrap_or("")
        };
        return Some(decode_entities(value));
    }
    None
}

fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    let mut started = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            space = true;
            continue;
        }
        if space && started {
            out.push(' ');
        } else if space {
            // Keep one leading space: it separates inline runs.
            out.push(' ');
        }
        space = false;
        started = true;
        out.push(ch);
    }
    if space {
        out.push(' ');
    }
    out
}

fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let Some(end) = after.find(';') else {
            out.push_str(&rest[amp..]);
            return out;
        };
        let entity = &after[..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{00A0}'),
            "mdash" => Some('—'),
            "ndash" => Some('–'),
            "hellip" => Some('…'),
            "copy" => Some('©'),
            "reg" => Some('®'),
            "trade" => Some('™'),
            "laquo" => Some('«'),
            "raquo" => Some('»'),
            "ldquo" => Some('“'),
            "rdquo" => Some('”'),
            "lsquo" => Some('‘'),
            "rsquo" => Some('’'),
            _ => {
                if let Some(number) = entity.strip_prefix('#') {
                    let value = if let Some(hex) = number.strip_prefix(['x', 'X']) {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        number.parse::<u32>().ok()
                    };
                    value.and_then(char::from_u32)
                } else {
                    None
                }
            }
        };
        match decoded {
            Some(ch) => out.push(ch),
            None => {
                out.push('&');
                out.push_str(entity);
                out.push(';');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown_in::{looks_like_markdown, markdown_to_rtf};

    fn cf_html(fragment: &str) -> Vec<u8> {
        // Build a real CF_HTML header whose offsets are byte positions.
        let body =
            format!("<html><body><!--StartFragment-->{fragment}<!--EndFragment--></body></html>");
        let start_marker = body.find("<!--StartFragment-->").unwrap() + 20;
        let end_marker = body.find("<!--EndFragment-->").unwrap();
        let mut header =
            String::from("Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\n");
        header.push_str("StartFragment:0000000000\r\nEndFragment:0000000000\r\n");
        // Replace the placeholders now that the header length is known.
        let header_len = header.len();
        let start = header_len + start_marker;
        let end = header_len + end_marker;
        let header = format!(
            "Version:0.9\r\nStartHTML:{:010}\r\nEndHTML:{:010}\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\n",
            header_len,
            header_len + body.len(),
            start,
            end
        );
        format!("{header}{body}").into_bytes()
    }

    #[test]
    fn chatgpt_style_html() {
        let html = cf_html(
            "<p>Berikut <strong>ringkasan</strong> singkat:</p>\
             <ol><li>Pertama</li><li>Kedua <code>x = 1</code></li></ol>\
             <pre><code class=\"language-python\">def f():\n    return 1\n</code></pre>\
             <p>Lihat <a href=\"https://example.com\">situs</a> ini.</p>",
        );
        let markdown = cf_html_to_markdown(&html);
        assert!(
            markdown.contains("Berikut **ringkasan** singkat:"),
            "{markdown}"
        );
        assert!(markdown.contains("1. Pertama"), "{markdown}");
        assert!(markdown.contains("2. Kedua `x = 1`"), "{markdown}");
        assert!(markdown.contains("```python"), "{markdown}");
        assert!(markdown.contains("def f():"), "{markdown}");
        assert!(
            markdown.contains("[situs](https://example.com)"),
            "{markdown}"
        );
        assert!(looks_like_markdown(&markdown));
    }

    #[test]
    fn wikipedia_paragraph() {
        let html = "<html><body><p><b>Kecerdasan buatan</b> (bahasa Inggris: \
            <i>artificial intelligence</i>) adalah kecerdasan yang ditambahkan \
            kepada suatu sistem yang bisa diatur dalam konteks ilmiah, atau \
            <a href=\"/wiki/Kecerdasan\">kecerdasan</a> entitas ilmiah.</p></body></html>";
        let markdown = html_to_markdown(html);
        assert!(markdown.starts_with("**Kecerdasan buatan**"), "{markdown}");
        assert!(markdown.contains("*artificial intelligence*"), "{markdown}");
        // A relative address cannot be opened from a note: the text stays,
        // the link does not.
        assert!(markdown.contains("atau kecerdasan entitas"), "{markdown}");
        assert!(!markdown.contains("/wiki/"), "{markdown}");
        assert!(!markdown.contains('<'), "tags left over: {markdown}");
    }

    #[test]
    fn nested_lists_and_tables() {
        let html = "<ul><li>Satu<ul><li>Anak</li></ul></li><li>Dua</li></ul>\
                    <table><tr><th>Nama</th><th>Nilai</th></tr>\
                    <tr><td>A</td><td>92%</td></tr></table>";
        let markdown = html_to_markdown(html);
        assert!(markdown.contains("- Satu"), "{markdown}");
        assert!(markdown.contains("  - Anak"), "{markdown}");
        assert!(markdown.contains("| Nama | Nilai |"), "{markdown}");
        assert!(markdown.contains("| --- | --- |"), "{markdown}");
        assert!(markdown.contains("| A | 92% |"), "{markdown}");
    }

    #[test]
    fn entities_and_images() {
        let markdown = html_to_markdown(
            "<p>AT&amp;T &mdash; 100&#37; <img src=\"https://x/y.png\" alt=\"Logo\"></p>",
        );
        assert!(markdown.contains("AT&T — 100%"), "{markdown}");
        assert!(markdown.contains("[Logo](https://x/y.png)"), "{markdown}");
    }

    #[test]
    fn katex_from_chatgpt_keeps_only_the_tex() {
        let html = r#"<p>Energi <span class="katex"><span class="katex-mathml"><math xmlns="http://www.w3.org/1998/Math/MathML"><semantics><mrow><mi>E</mi><mo>=</mo><mi>m</mi><msup><mi>c</mi><mn>2</mn></msup></mrow><annotation encoding="application/x-tex">E = mc^2</annotation></semantics></math></span><span class="katex-html" aria-hidden="true"><span class="base"><span class="mord">E</span><span class="mrel">=</span><span class="mord">m</span><span class="mord"><span class="mord">c</span><span class="msupsub">2</span></span></span></span></span> saja.</p>"#;
        let markdown = html_to_markdown(html);
        assert_eq!(markdown, "Energi $E = mc^2$ saja.");
    }

    #[test]
    fn katex_display_math_is_a_block() {
        let html = r#"<span class="katex-display"><span class="katex"><span class="katex-mathml"><math display="block"><semantics><mrow></mrow><annotation encoding="application/x-tex">\frac{a}{b}</annotation></semantics></math></span><span class="katex-html" aria-hidden="true"><span>a</span><span>b</span></span></span></span>"#;
        assert_eq!(html_to_markdown(html), r"$$\frac{a}{b}$$");
    }

    #[test]
    fn wikipedia_alttext_and_mathjax_scripts() {
        let wiki = r#"<p>Luas <span class="mwe-math-element"><math alttext="{\displaystyle \pi r^{2}}"><semantics><mi>π</mi></semantics></math><img src="x.svg" class="mwe-math-fallback-image-inline" aria-hidden="true" alt="\pi r^2"></span>.</p>"#;
        assert_eq!(html_to_markdown(wiki), r"Luas $\pi r^{2}$.");
        let mathjax = r#"<p>Nilai <script type="math/tex">x_1 &lt; y</script> dan <script type="math/tex; mode=display">\sum_i x_i</script></p>"#;
        let markdown = html_to_markdown(mathjax);
        assert!(markdown.contains("$x_1 < y$"), "{markdown}");
        assert!(markdown.contains(r"$$\sum_i x_i$$"), "{markdown}");
    }

    #[test]
    fn converts_to_balanced_rtf() {
        let markdown = cf_html_to_markdown(&cf_html("<h2>Judul</h2><p>Teks</p>"));
        let rtf = markdown_to_rtf(&markdown, 9000);
        let mut depth = 0i32;
        for ch in rtf.chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
        assert_eq!(depth, 0);
    }
}
