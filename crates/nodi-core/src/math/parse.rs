//! LaTeX math subset → syntax tree.
//!
//! Supported: letters/digits/operators, Greek, common symbols and relations,
//! `^`/`_`, `\frac` `\dfrac` `\tfrac` `\binom`, `\sqrt[n]{}`, big operators
//! with limits (`\sum` `\prod` `\int` `\lim` …), function names (`\sin` …),
//! `\left…\right`, accents (`\hat` `\bar` `\vec` `\tilde` `\dot` `\ddot`
//! `\overline` `\underline`), font switches (`\mathbf` `\mathrm` `\mathit`
//! `\mathbb` `\mathcal` `\text` `\operatorname`), spacing, and the
//! `matrix`/`pmatrix`/`bmatrix`/`Bmatrix`/`vmatrix`/`Vmatrix`/`cases`/
//! `aligned` environments.

use std::fmt;

/// How a symbol is spaced and drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Italic letter (a variable).
    Variable,
    /// Upright ordinary symbol: digits, Greek capitals, ∞, ∂ …
    Ordinary,
    /// Binary operator: + − × ± …
    Binary,
    /// Relation: = < ≤ → …
    Relation,
    /// Punctuation: , ;
    Punct,
    Open,
    Close,
    /// Upright operator name: sin, log …
    Function,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Normal,
    Bold,
    Upright,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccentKind {
    Hat,
    Bar,
    Vec,
    Tilde,
    Dot,
    DDot,
    Overline,
    Underline,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Symbol {
        text: String,
        class: Class,
        variant: Variant,
    },
    Row(Vec<Node>),
    Frac {
        num: Box<Node>,
        den: Box<Node>,
        bar: bool,
        /// `\dfrac` forces display size, `\tfrac` text size, `None` = auto.
        display: Option<bool>,
    },
    Sqrt {
        index: Option<Box<Node>>,
        body: Box<Node>,
    },
    Scripts {
        base: Box<Node>,
        sup: Option<Box<Node>>,
        sub: Option<Box<Node>>,
    },
    /// ∑ ∏ ∫ lim …; `limits` puts scripts above/below in display style.
    BigOp {
        text: String,
        limits: bool,
        /// Drawn as a large glyph (∑) rather than a word (lim).
        glyph: bool,
    },
    Delimited {
        open: String,
        body: Box<Node>,
        close: String,
    },
    Accent {
        kind: AccentKind,
        body: Box<Node>,
    },
    Matrix {
        rows: Vec<Vec<Node>>,
        open: String,
        close: String,
        /// `cases`/`aligned` columns are left aligned.
        left_aligned: bool,
    },
    /// Horizontal space in em.
    Space(f32),
    /// Upright text from `\text{...}` (spaces kept).
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    /// Character offset in the source.
    pub at: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at character {})", self.message, self.at + 1)
    }
}

impl std::error::Error for ParseError {}

/// Parses a LaTeX math expression.
pub fn parse(source: &str) -> Result<Node, ParseError> {
    let mut parser = Parser {
        chars: source.chars().collect(),
        pos: 0,
    };
    let row = parser.row(&[])?;
    if parser.pos < parser.chars.len() {
        return Err(parser.error("unexpected '}'"));
    }
    if row.is_empty() {
        return Err(ParseError {
            message: "the formula is empty".to_owned(),
            at: 0,
        });
    }
    Ok(Node::Row(row))
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

/// What ends the row currently being parsed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    Brace,
    Bracket,
    Right,
    Ampersand,
    NewRow,
    End,
}

impl Parser {
    fn error(&self, message: &str) -> ParseError {
        ParseError {
            message: message.to_owned(),
            at: self.pos.min(self.chars.len()),
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_spaces(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    /// Reads `\name` (letters) or `\x` (one symbol) after the backslash.
    fn command_name(&mut self) -> String {
        let start = self.pos;
        while self.peek().is_some_and(|ch| ch.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        if self.pos == start {
            if let Some(ch) = self.peek() {
                self.pos += 1;
                return ch.to_string();
            }
        }
        self.chars[start..self.pos].iter().collect()
    }

    /// True (and consumes it) when the next command is `\name`.
    fn at_command(&mut self, name: &str) -> bool {
        let save = self.pos;
        self.skip_spaces();
        if self.peek() == Some('\\') {
            self.pos += 1;
            if self.command_name() == name {
                return true;
            }
        }
        self.pos = save;
        false
    }

    /// Parses atoms until one of `stops` (or the end of input).
    fn row(&mut self, stops: &[Stop]) -> Result<Vec<Node>, ParseError> {
        let mut nodes = Vec::new();
        loop {
            self.skip_spaces();
            let Some(ch) = self.peek() else {
                if stops.is_empty() || stops.contains(&Stop::End) {
                    return Ok(nodes);
                }
                return Err(self.error("missing '}' or \\end"));
            };
            let stop = match ch {
                '}' => Some(Stop::Brace),
                ']' if stops.contains(&Stop::Bracket) => Some(Stop::Bracket),
                '&' => Some(Stop::Ampersand),
                '\\' => {
                    let save = self.pos;
                    self.pos += 1;
                    let name = self.command_name();
                    self.pos = save;
                    match name.as_str() {
                        "right" => Some(Stop::Right),
                        "\\" | "cr" => Some(Stop::NewRow),
                        "end" => Some(Stop::End),
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(stop) = stop {
                if stops.contains(&stop) {
                    return Ok(nodes);
                }
                if stop == Stop::Brace && stops.is_empty() {
                    return Ok(nodes);
                }
                return Err(self.error(match stop {
                    Stop::Brace => "unexpected '}'",
                    Stop::Right => "\\right without \\left",
                    Stop::Ampersand => "'&' is only allowed inside a matrix",
                    Stop::NewRow => "'\\\\' is only allowed inside a matrix",
                    Stop::End => "\\end without \\begin",
                    Stop::Bracket => "unexpected ']'",
                }));
            }
            let atom = self.atom()?;
            let atom = self.scripts(atom)?;
            nodes.push(atom);
        }
    }

    /// One `{group}` or single atom, used as an argument.
    fn argument(&mut self) -> Result<Node, ParseError> {
        self.skip_spaces();
        match self.peek() {
            Some('{') => self.group(),
            Some(_) => self.atom(),
            None => Err(self.error("missing argument")),
        }
    }

    fn group(&mut self) -> Result<Node, ParseError> {
        self.pos += 1; // '{'
        let row = self.row(&[Stop::Brace])?;
        if self.peek() != Some('}') {
            return Err(self.error("missing '}'"));
        }
        self.pos += 1;
        Ok(Node::Row(row))
    }

    /// Raw text inside `{...}` (for `\text`, `\begin{name}`).
    fn raw_group(&mut self) -> Result<String, ParseError> {
        self.skip_spaces();
        if self.peek() != Some('{') {
            return Err(self.error("expected '{'"));
        }
        self.pos += 1;
        let mut depth = 0;
        let mut text = String::new();
        while let Some(ch) = self.peek() {
            self.pos += 1;
            match ch {
                '{' => depth += 1,
                '}' if depth == 0 => return Ok(text),
                '}' => depth -= 1,
                _ => {}
            }
            text.push(ch);
        }
        Err(self.error("missing '}'"))
    }

    fn scripts(&mut self, base: Node) -> Result<Node, ParseError> {
        let mut sup = None;
        let mut sub = None;
        loop {
            self.skip_spaces();
            match self.peek() {
                Some('^') if sup.is_none() => {
                    self.pos += 1;
                    sup = Some(Box::new(self.argument()?));
                }
                Some('_') if sub.is_none() => {
                    self.pos += 1;
                    sub = Some(Box::new(self.argument()?));
                }
                Some('^') => return Err(self.error("double superscript")),
                Some('_') => return Err(self.error("double subscript")),
                Some('\'') => {
                    // Primes: x' → x^{′}
                    self.pos += 1;
                    let prime = symbol("\u{2032}", Class::Ordinary);
                    sup = Some(Box::new(match sup.take() {
                        Some(existing) => Node::Row(vec![*existing, prime]),
                        None => prime,
                    }));
                }
                _ => break,
            }
        }
        if sup.is_none() && sub.is_none() {
            Ok(base)
        } else {
            Ok(Node::Scripts {
                base: Box::new(base),
                sup,
                sub,
            })
        }
    }

    fn atom(&mut self) -> Result<Node, ParseError> {
        let Some(ch) = self.peek() else {
            return Err(self.error("unexpected end"));
        };
        match ch {
            '{' => self.group(),
            '\\' => {
                self.pos += 1;
                let name = self.command_name();
                self.command(&name)
            }
            '^' | '_' => Err(self.error("superscript/subscript needs a base, e.g. x^2")),
            _ => {
                self.pos += 1;
                Ok(char_symbol(ch))
            }
        }
    }

    fn command(&mut self, name: &str) -> Result<Node, ParseError> {
        if let Some(node) = named_symbol(name) {
            return Ok(node);
        }
        match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let num = self.argument()?;
                let den = self.argument()?;
                Ok(Node::Frac {
                    num: Box::new(num),
                    den: Box::new(den),
                    bar: true,
                    display: match name {
                        "dfrac" | "cfrac" => Some(true),
                        "tfrac" => Some(false),
                        _ => None,
                    },
                })
            }
            "binom" | "dbinom" | "tbinom" => {
                let top = self.argument()?;
                let bottom = self.argument()?;
                Ok(Node::Delimited {
                    open: "(".to_owned(),
                    body: Box::new(Node::Frac {
                        num: Box::new(top),
                        den: Box::new(bottom),
                        bar: false,
                        display: None,
                    }),
                    close: ")".to_owned(),
                })
            }
            "sqrt" => {
                self.skip_spaces();
                let index = if self.peek() == Some('[') {
                    self.pos += 1;
                    let row = self.row(&[Stop::Bracket])?;
                    if self.peek() != Some(']') {
                        return Err(self.error("missing ']'"));
                    }
                    self.pos += 1;
                    Some(Box::new(Node::Row(row)))
                } else {
                    None
                };
                let body = self.argument()?;
                Ok(Node::Sqrt {
                    index,
                    body: Box::new(body),
                })
            }
            "left" => {
                let open = self.delimiter()?;
                let body = self.row(&[Stop::Right])?;
                if !self.at_command("right") {
                    return Err(self.error("\\left without \\right"));
                }
                let close = self.delimiter()?;
                Ok(Node::Delimited {
                    open,
                    body: Box::new(Node::Row(body)),
                    close,
                })
            }
            "right" => Err(self.error("\\right without \\left")),
            "hat" | "widehat" => self.accent(AccentKind::Hat),
            "bar" => self.accent(AccentKind::Bar),
            "vec" | "overrightarrow" => self.accent(AccentKind::Vec),
            "tilde" | "widetilde" => self.accent(AccentKind::Tilde),
            "dot" => self.accent(AccentKind::Dot),
            "ddot" => self.accent(AccentKind::DDot),
            "overline" => self.accent(AccentKind::Overline),
            "underline" => self.accent(AccentKind::Underline),
            "mathbf" | "boldsymbol" | "bm" => {
                let body = self.argument()?;
                Ok(with_variant(body, Variant::Bold))
            }
            "mathrm" | "operatorname" | "mathup" => {
                let body = self.argument()?;
                let node = with_variant(body, Variant::Upright);
                Ok(if name == "operatorname" {
                    as_function(node)
                } else {
                    node
                })
            }
            "mathit" => self.argument(),
            "mathbb" => {
                let text = self.raw_group()?;
                Ok(Node::Row(
                    text.chars()
                        .filter(|ch| !ch.is_whitespace())
                        .map(|ch| symbol(&double_struck(ch), Class::Ordinary))
                        .collect(),
                ))
            }
            "mathcal" | "mathscr" => {
                let text = self.raw_group()?;
                Ok(Node::Row(
                    text.chars()
                        .filter(|ch| !ch.is_whitespace())
                        .map(|ch| symbol(&script(ch), Class::Ordinary))
                        .collect(),
                ))
            }
            "text" | "textrm" | "textnormal" | "mbox" | "textit" | "textbf" => {
                Ok(Node::Text(self.raw_group()?))
            }
            "," | "thinspace" => Ok(Node::Space(3.0 / 18.0)),
            ":" | ">" | "medspace" => Ok(Node::Space(4.0 / 18.0)),
            ";" | "thickspace" => Ok(Node::Space(5.0 / 18.0)),
            "!" | "negthinspace" => Ok(Node::Space(-3.0 / 18.0)),
            " " => Ok(Node::Space(0.3)),
            "quad" => Ok(Node::Space(1.0)),
            "qquad" => Ok(Node::Space(2.0)),
            "begin" => self.environment(),
            "displaystyle" | "textstyle" | "limits" | "nolimits" | "big" | "Big" | "bigg"
            | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" => Ok(Node::Row(Vec::new())),
            "{" | "}" | "|" | "#" | "%" | "&" | "$" | "_" => {
                let text = if name == "|" { "\u{2016}" } else { name };
                let class = match name {
                    "{" => Class::Open,
                    "}" => Class::Close,
                    _ => Class::Ordinary,
                };
                Ok(symbol(text, class))
            }
            _ => Err(self.error(&format!("unknown command \\{name}"))),
        }
    }

    fn accent(&mut self, kind: AccentKind) -> Result<Node, ParseError> {
        let body = self.argument()?;
        Ok(Node::Accent {
            kind,
            body: Box::new(body),
        })
    }

    /// Delimiter after `\left` / `\right`: `( ) [ ] | . \{ \} \| \langle …`.
    fn delimiter(&mut self) -> Result<String, ParseError> {
        self.skip_spaces();
        let Some(ch) = self.peek() else {
            return Err(self.error("missing delimiter"));
        };
        self.pos += 1;
        if ch == '\\' {
            let name = self.command_name();
            return Ok(match name.as_str() {
                "{" | "lbrace" => "{",
                "}" | "rbrace" => "}",
                "|" | "Vert" => "\u{2016}",
                "langle" => "\u{27E8}",
                "rangle" => "\u{27E9}",
                "lfloor" => "\u{230A}",
                "rfloor" => "\u{230B}",
                "lceil" => "\u{2308}",
                "rceil" => "\u{2309}",
                "vert" => "|",
                _ => return Err(self.error(&format!("unknown delimiter \\{name}"))),
            }
            .to_owned());
        }
        match ch {
            '.' => Ok(String::new()),
            '(' | ')' | '[' | ']' | '|' | '/' => Ok(ch.to_string()),
            _ => Err(self.error("unknown delimiter")),
        }
    }

    fn environment(&mut self) -> Result<Node, ParseError> {
        let name = self.raw_group()?;
        let (open, close, left_aligned) = match name.as_str() {
            "matrix" | "smallmatrix" => ("", "", false),
            "pmatrix" => ("(", ")", false),
            "bmatrix" => ("[", "]", false),
            "Bmatrix" => ("{", "}", false),
            "vmatrix" => ("|", "|", false),
            "Vmatrix" => ("\u{2016}", "\u{2016}", false),
            "cases" => ("{", "", true),
            "aligned" | "align" | "align*" | "gathered" | "array" => ("", "", true),
            _ => return Err(self.error(&format!("unknown environment {{{name}}}"))),
        };
        if name == "array" {
            // Column spec, e.g. {cc}: accepted and ignored.
            self.raw_group()?;
        }
        let mut rows = Vec::new();
        let mut row = Vec::new();
        loop {
            let cell = self.row(&[Stop::Ampersand, Stop::NewRow, Stop::End])?;
            row.push(Node::Row(cell));
            self.skip_spaces();
            match self.peek() {
                Some('&') => {
                    self.pos += 1;
                }
                Some('\\') => {
                    self.pos += 1;
                    let command = self.command_name();
                    if command == "end" {
                        let end = self.raw_group()?;
                        if end != name {
                            return Err(
                                self.error(&format!("\\begin{{{name}}} ended by \\end{{{end}}}"))
                            );
                        }
                        rows.push(std::mem::take(&mut row));
                        break;
                    }
                    rows.push(std::mem::take(&mut row));
                }
                _ => return Err(self.error(&format!("missing \\end{{{name}}}"))),
            }
        }
        // A trailing `\\` leaves an empty last row.
        if rows.len() > 1
            && rows.last().is_some_and(|row| {
                row.len() == 1 && matches!(&row[0], Node::Row(cells) if cells.is_empty())
            })
        {
            rows.pop();
        }
        Ok(Node::Matrix {
            rows,
            open: open.to_owned(),
            close: close.to_owned(),
            left_aligned,
        })
    }
}

fn symbol(text: &str, class: Class) -> Node {
    Node::Symbol {
        text: text.to_owned(),
        class,
        variant: Variant::Normal,
    }
}

fn char_symbol(ch: char) -> Node {
    let (text, class) = match ch {
        'a'..='z' | 'A'..='Z' => (ch.to_string(), Class::Variable),
        '0'..='9' | '.' => (ch.to_string(), Class::Ordinary),
        '+' => ("+".to_owned(), Class::Binary),
        '-' => ("\u{2212}".to_owned(), Class::Binary),
        '*' => ("\u{2217}".to_owned(), Class::Binary),
        '/' => ("/".to_owned(), Class::Ordinary),
        '=' | '<' | '>' => (ch.to_string(), Class::Relation),
        ':' => (":".to_owned(), Class::Relation),
        ',' | ';' => (ch.to_string(), Class::Punct),
        '(' | '[' => (ch.to_string(), Class::Open),
        ')' | ']' => (ch.to_string(), Class::Close),
        '|' => ("|".to_owned(), Class::Ordinary),
        '~' => return Node::Space(0.3),
        other => (other.to_string(), Class::Ordinary),
    };
    Node::Symbol {
        text,
        class,
        variant: Variant::Normal,
    }
}

fn with_variant(node: Node, variant: Variant) -> Node {
    match node {
        Node::Symbol { text, class, .. } => {
            let class = if class == Class::Variable && variant == Variant::Upright {
                Class::Ordinary
            } else {
                class
            };
            Node::Symbol {
                text,
                class,
                variant,
            }
        }
        Node::Row(nodes) => Node::Row(
            nodes
                .into_iter()
                .map(|n| with_variant(n, variant))
                .collect(),
        ),
        Node::Scripts { base, sup, sub } => Node::Scripts {
            base: Box::new(with_variant(*base, variant)),
            sup,
            sub,
        },
        other => other,
    }
}

/// `\operatorname{argmax}`: one upright word spaced like `\sin`.
fn as_function(node: Node) -> Node {
    let mut text = String::new();
    collect_text(&node, &mut text);
    Node::Symbol {
        text,
        class: Class::Function,
        variant: Variant::Upright,
    }
}

fn collect_text(node: &Node, out: &mut String) {
    match node {
        Node::Symbol { text, .. } => out.push_str(text),
        Node::Row(nodes) => nodes.iter().for_each(|node| collect_text(node, out)),
        Node::Text(text) => out.push_str(text),
        _ => {}
    }
}

/// Double-struck letters that live in the Basic Multilingual Plane.
fn double_struck(ch: char) -> String {
    match ch {
        'C' => "\u{2102}",
        'H' => "\u{210D}",
        'N' => "\u{2115}",
        'P' => "\u{2119}",
        'Q' => "\u{211A}",
        'R' => "\u{211D}",
        'Z' => "\u{2124}",
        _ => return ch.to_string(),
    }
    .to_owned()
}

/// Script capitals available in the BMP; others stay as plain letters.
fn script(ch: char) -> String {
    match ch {
        'B' => "\u{212C}",
        'E' => "\u{2130}",
        'F' => "\u{2131}",
        'H' => "\u{210B}",
        'I' => "\u{2110}",
        'L' => "\u{2112}",
        'M' => "\u{2133}",
        'R' => "\u{211B}",
        'e' => "\u{212F}",
        'g' => "\u{210A}",
        'o' => "\u{2134}",
        _ => return ch.to_string(),
    }
    .to_owned()
}

/// Symbols, Greek letters, function names and big operators by name.
fn named_symbol(name: &str) -> Option<Node> {
    use Class::*;
    let greek_lower = [
        ("alpha", "α"),
        ("beta", "β"),
        ("gamma", "γ"),
        ("delta", "δ"),
        ("epsilon", "ϵ"),
        ("varepsilon", "ε"),
        ("zeta", "ζ"),
        ("eta", "η"),
        ("theta", "θ"),
        ("vartheta", "ϑ"),
        ("iota", "ι"),
        ("kappa", "κ"),
        ("lambda", "λ"),
        ("mu", "μ"),
        ("nu", "ν"),
        ("xi", "ξ"),
        ("pi", "π"),
        ("varpi", "ϖ"),
        ("rho", "ρ"),
        ("varrho", "ϱ"),
        ("sigma", "σ"),
        ("varsigma", "ς"),
        ("tau", "τ"),
        ("upsilon", "υ"),
        ("phi", "ϕ"),
        ("varphi", "φ"),
        ("chi", "χ"),
        ("psi", "ψ"),
        ("omega", "ω"),
    ];
    if let Some((_, text)) = greek_lower.iter().find(|(key, _)| *key == name) {
        return Some(symbol(text, Variable));
    }
    let greek_upper = [
        ("Gamma", "Γ"),
        ("Delta", "Δ"),
        ("Theta", "Θ"),
        ("Lambda", "Λ"),
        ("Xi", "Ξ"),
        ("Pi", "Π"),
        ("Sigma", "Σ"),
        ("Upsilon", "Υ"),
        ("Phi", "Φ"),
        ("Psi", "Ψ"),
        ("Omega", "Ω"),
    ];
    if let Some((_, text)) = greek_upper.iter().find(|(key, _)| *key == name) {
        return Some(symbol(text, Ordinary));
    }
    let table: &[(&str, &str, Class)] = &[
        ("infty", "∞", Ordinary),
        ("partial", "∂", Ordinary),
        ("nabla", "∇", Ordinary),
        ("hbar", "ℏ", Ordinary),
        ("ell", "ℓ", Ordinary),
        ("emptyset", "∅", Ordinary),
        ("varnothing", "∅", Ordinary),
        ("forall", "∀", Ordinary),
        ("exists", "∃", Ordinary),
        ("neg", "¬", Ordinary),
        ("lnot", "¬", Ordinary),
        ("angle", "∠", Ordinary),
        ("degree", "°", Ordinary),
        ("circ", "∘", Binary),
        ("prime", "′", Ordinary),
        ("ldots", "…", Ordinary),
        ("dots", "…", Ordinary),
        ("cdots", "⋯", Ordinary),
        ("vdots", "⋮", Ordinary),
        ("ddots", "⋱", Ordinary),
        ("pm", "±", Binary),
        ("mp", "∓", Binary),
        ("times", "×", Binary),
        ("div", "÷", Binary),
        ("cdot", "⋅", Binary),
        ("ast", "∗", Binary),
        ("star", "⋆", Binary),
        ("bullet", "∙", Binary),
        ("oplus", "⊕", Binary),
        ("otimes", "⊗", Binary),
        ("odot", "⊙", Binary),
        ("cup", "∪", Binary),
        ("cap", "∩", Binary),
        ("setminus", "∖", Binary),
        ("wedge", "∧", Binary),
        ("land", "∧", Binary),
        ("vee", "∨", Binary),
        ("lor", "∨", Binary),
        ("leq", "≤", Relation),
        ("le", "≤", Relation),
        ("geq", "≥", Relation),
        ("ge", "≥", Relation),
        ("neq", "≠", Relation),
        ("ne", "≠", Relation),
        ("approx", "≈", Relation),
        ("equiv", "≡", Relation),
        ("sim", "∼", Relation),
        ("simeq", "≃", Relation),
        ("cong", "≅", Relation),
        ("propto", "∝", Relation),
        ("ll", "≪", Relation),
        ("gg", "≫", Relation),
        ("in", "∈", Relation),
        ("notin", "∉", Relation),
        ("ni", "∋", Relation),
        ("subset", "⊂", Relation),
        ("supset", "⊃", Relation),
        ("subseteq", "⊆", Relation),
        ("supseteq", "⊇", Relation),
        ("mid", "∣", Relation),
        ("parallel", "∥", Relation),
        ("perp", "⊥", Relation),
        ("to", "→", Relation),
        ("rightarrow", "→", Relation),
        ("leftarrow", "←", Relation),
        ("gets", "←", Relation),
        ("leftrightarrow", "↔", Relation),
        ("Rightarrow", "⇒", Relation),
        ("implies", "⇒", Relation),
        ("Leftarrow", "⇐", Relation),
        ("Leftrightarrow", "⇔", Relation),
        ("iff", "⇔", Relation),
        ("mapsto", "↦", Relation),
        ("uparrow", "↑", Relation),
        ("downarrow", "↓", Relation),
        ("coloneqq", "≔", Relation),
        ("triangleq", "≜", Relation),
        ("langle", "⟨", Open),
        ("rangle", "⟩", Close),
        ("lfloor", "⌊", Open),
        ("rfloor", "⌋", Close),
        ("lceil", "⌈", Open),
        ("rceil", "⌉", Close),
        ("lbrace", "{", Open),
        ("rbrace", "}", Close),
        ("vert", "|", Ordinary),
        ("Vert", "‖", Ordinary),
    ];
    if let Some((_, text, class)) = table.iter().find(|(key, _, _)| *key == name) {
        return Some(symbol(text, *class));
    }
    let functions = [
        "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh",
        "tanh", "log", "ln", "lg", "exp", "det", "dim", "ker", "deg", "gcd", "arg", "Pr", "hom",
        "mod", "sgn", "tr",
    ];
    if functions.contains(&name) {
        return Some(Node::Symbol {
            text: name.to_owned(),
            class: Function,
            variant: Variant::Upright,
        });
    }
    let big: &[(&str, &str, bool, bool)] = &[
        ("sum", "∑", true, true),
        ("prod", "∏", true, true),
        ("coprod", "∐", true, true),
        ("bigcup", "⋃", true, true),
        ("bigcap", "⋂", true, true),
        ("bigoplus", "⨁", true, true),
        ("bigotimes", "⨂", true, true),
        ("int", "∫", false, true),
        ("iint", "∬", false, true),
        ("iiint", "∭", false, true),
        ("oint", "∮", false, true),
        ("lim", "lim", true, false),
        ("max", "max", true, false),
        ("min", "min", true, false),
        ("sup", "sup", true, false),
        ("inf", "inf", true, false),
        ("argmax", "arg max", true, false),
        ("argmin", "arg min", true, false),
        ("limsup", "lim sup", true, false),
        ("liminf", "lim inf", true, false),
    ];
    if let Some((_, text, limits, glyph)) = big.iter().find(|(key, ..)| *key == name) {
        return Some(Node::BigOp {
            text: (*text).to_owned(),
            limits: *limits,
            glyph: *glyph,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(node: &Node) -> &[Node] {
        match node {
            Node::Row(nodes) => nodes,
            _ => panic!("not a row: {node:?}"),
        }
    }

    #[test]
    fn parses_scripts_and_fraction() {
        let tree = parse(r"E = mc^2 + \frac{a}{b}").unwrap();
        let nodes = row(&tree);
        assert!(matches!(
            &nodes[3],
            Node::Scripts {
                sup: Some(_),
                sub: None,
                ..
            }
        ));
        assert!(matches!(nodes.last(), Some(Node::Frac { bar: true, .. })));
    }

    #[test]
    fn parses_loss_function() {
        let tree = parse(r"L = \frac{1}{n} \sum_{i=1}^{n} (\hat{y}_i - y_i)^2").unwrap();
        let nodes = row(&tree);
        assert!(nodes.iter().any(|node| matches!(node, Node::Scripts { base, .. } if matches!(**base, Node::BigOp { .. }))));
    }

    #[test]
    fn parses_left_right_and_matrix() {
        let tree = parse(r"\left( \begin{pmatrix} 1 & 0 \\ 0 & 1 \end{pmatrix} \right)").unwrap();
        let nodes = row(&tree);
        match &nodes[0] {
            Node::Delimited { open, close, body } => {
                assert_eq!((open.as_str(), close.as_str()), ("(", ")"));
                match &row(body)[0] {
                    Node::Matrix { rows, .. } => {
                        assert_eq!(rows.len(), 2);
                        assert_eq!(rows[0].len(), 2);
                    }
                    other => panic!("expected matrix, got {other:?}"),
                }
            }
            other => panic!("expected delimited, got {other:?}"),
        }
    }

    #[test]
    fn parses_cases_with_trailing_newline() {
        let tree = parse(r"f(x) = \begin{cases} 1 & x > 0 \\ 0 & \text{otherwise} \\ \end{cases}")
            .unwrap();
        let matrix = row(&tree)
            .iter()
            .find(|node| matches!(node, Node::Matrix { .. }))
            .unwrap();
        if let Node::Matrix {
            rows, left_aligned, ..
        } = matrix
        {
            assert_eq!(rows.len(), 2);
            assert!(*left_aligned);
        }
    }

    #[test]
    fn parses_sqrt_with_index_and_primes() {
        let tree = parse(r"\sqrt[3]{x} + f'(x)").unwrap();
        assert!(matches!(&row(&tree)[0], Node::Sqrt { index: Some(_), .. }));
        assert!(matches!(&row(&tree)[2], Node::Scripts { sup: Some(_), .. }));
    }

    #[test]
    fn reports_errors_with_position() {
        let error = parse(r"\frac{a}{b").unwrap_err();
        assert!(error.message.contains('}'), "{error}");
        let error = parse(r"x^2^3").unwrap_err();
        assert!(error.message.contains("double superscript"));
        let error = parse(r"\foo").unwrap_err();
        assert!(error.message.contains("unknown command"));
        assert!(parse("   ").is_err());
        assert!(parse(r"a \right)").is_err());
    }

    #[test]
    fn mathbb_and_text() {
        let tree = parse(r"x \in \mathbb{R} \text{ for all } n").unwrap();
        let nodes = row(&tree);
        assert!(nodes.iter().any(|node| matches!(node, Node::Row(inner) if matches!(&inner[0], Node::Symbol { text, .. } if text == "ℝ"))));
        assert!(nodes
            .iter()
            .any(|node| matches!(node, Node::Text(text) if text == " for all ")));
    }
}
