//! TeX-like box layout for a parsed formula.
//!
//! Font measurement is injected through [`Metrics`] so the layout runs (and is
//! tested) without any OS. Coordinates: x grows right, y grows down, the
//! formula baseline is y = 0. Text items are positioned by their baseline
//! origin, rules by their top-left corner, lines by absolute points.

use super::parse::{AccentKind, Class, Node, Variant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Italic,
    Upright,
    Bold,
}

/// Advance width and ink extents (both ≥ 0) of a piece of text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

pub trait Metrics {
    fn measure(&self, text: &str, size: f32, style: Style) -> TextMetrics;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Draw {
    Text {
        text: String,
        size: f32,
        style: Style,
    },
    /// Filled rectangle; the item position is its top-left corner.
    Rule { width: f32, height: f32 },
    /// Polyline in absolute coordinates (the item position is ignored).
    Line {
        points: Vec<(f32, f32)>,
        thickness: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub x: f32,
    pub y: f32,
    pub draw: Draw,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub items: Vec<Item>,
}

impl Layout {
    fn empty() -> Self {
        Self::default()
    }

    /// Copies `other`'s items into `self`, offset by (`dx`, `dy`).
    fn place(&mut self, other: &Layout, dx: f32, dy: f32) {
        for item in &other.items {
            let draw = match &item.draw {
                Draw::Line { points, thickness } => Draw::Line {
                    points: points.iter().map(|(x, y)| (x + dx, y + dy)).collect(),
                    thickness: *thickness,
                },
                other => other.clone(),
            };
            self.items.push(Item {
                x: item.x + dx,
                y: item.y + dy,
                draw,
            });
        }
    }
}

/// Lays out `node` at `size` pixels per em in display style.
pub fn layout(node: &Node, metrics: &dyn Metrics, size: f32) -> Layout {
    let context = Context {
        metrics,
        size,
        base: size,
        display: true,
    };
    context.node(node)
}

#[derive(Clone, Copy)]
struct Context<'a> {
    metrics: &'a dyn Metrics,
    size: f32,
    base: f32,
    display: bool,
}

/// Spacing role of an atom inside a row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
}

impl<'a> Context<'a> {
    fn axis(&self) -> f32 {
        self.size * 0.25
    }

    fn rule(&self) -> f32 {
        (self.size * 0.055).max(1.0)
    }

    fn script(&self) -> Self {
        Self {
            size: (self.size * 0.71).max(self.base * 0.5),
            display: false,
            ..*self
        }
    }

    fn text_style(&self) -> Self {
        Self {
            display: false,
            ..*self
        }
    }

    fn text(&self, text: &str, style: Style, size: f32) -> Layout {
        let metrics = self.metrics.measure(text, size, style);
        // Italic glyphs lean past their advance width.
        let width = metrics.width
            + if style == Style::Italic {
                size * 0.04
            } else {
                0.0
            };
        Layout {
            width,
            ascent: metrics.ascent,
            descent: metrics.descent,
            items: vec![Item {
                x: 0.0,
                y: 0.0,
                draw: Draw::Text {
                    text: text.to_owned(),
                    size,
                    style,
                },
            }],
        }
    }

    fn node(&self, node: &Node) -> Layout {
        match node {
            Node::Symbol {
                text,
                class,
                variant,
            } => {
                let style = match (variant, class) {
                    (Variant::Bold, _) => Style::Bold,
                    (Variant::Normal, Class::Variable) => Style::Italic,
                    _ => Style::Upright,
                };
                self.text(text, style, self.size)
            }
            Node::Row(nodes) => self.row(nodes),
            Node::Text(text) => self.text(text, Style::Upright, self.size),
            Node::Space(em) => Layout {
                width: em * self.size,
                ..Layout::empty()
            },
            Node::Frac {
                num,
                den,
                bar,
                display,
            } => self.fraction(num, den, *bar, display.unwrap_or(self.display)),
            Node::Sqrt { index, body } => self.sqrt(index.as_deref(), body),
            Node::Scripts { base, sup, sub } => self.scripts(base, sup.as_deref(), sub.as_deref()),
            Node::BigOp { text, glyph, .. } => self.big_op(text, *glyph),
            Node::Delimited { open, body, close } => {
                let inner = self.node(body);
                self.delimit(inner, open, close)
            }
            Node::Accent { kind, body } => self.accent(*kind, body),
            Node::Matrix {
                rows,
                open,
                close,
                left_aligned,
            } => {
                let grid = self.matrix(rows, *left_aligned);
                self.delimit(grid, open, close)
            }
        }
    }

    fn role(node: &Node) -> Role {
        match node {
            Node::Symbol { class, .. } => match class {
                Class::Binary => Role::Bin,
                Class::Relation => Role::Rel,
                Class::Open => Role::Open,
                Class::Close => Role::Close,
                Class::Punct => Role::Punct,
                Class::Function => Role::Op,
                _ => Role::Ord,
            },
            Node::BigOp { .. } => Role::Op,
            Node::Scripts { base, .. } => Self::role(base),
            Node::Frac { .. } | Node::Delimited { .. } | Node::Matrix { .. } => Role::Inner,
            Node::Space(_) => Role::Ord,
            _ => Role::Ord,
        }
    }

    /// Space (in em) between two neighbouring roles, TeX's table simplified.
    fn spacing(left: Role, right: Role) -> f32 {
        const THIN: f32 = 3.0 / 18.0;
        const MEDIUM: f32 = 4.0 / 18.0;
        const THICK: f32 = 5.0 / 18.0;
        match (left, right) {
            (Role::Bin, _) | (_, Role::Bin) => MEDIUM,
            (Role::Rel, Role::Rel) => 0.0,
            (Role::Rel, _) | (_, Role::Rel) => THICK,
            (Role::Op, Role::Ord) | (Role::Ord, Role::Op) | (Role::Op, Role::Op) => THIN,
            (Role::Op, Role::Inner) | (Role::Inner, Role::Op) => THIN,
            (Role::Punct, _) => THIN,
            (Role::Inner, Role::Ord) | (Role::Ord, Role::Inner) => THIN,
            _ => 0.0,
        }
    }

    fn row(&self, nodes: &[Node]) -> Layout {
        let mut out = Layout::empty();
        let mut previous: Option<Role> = None;
        for node in nodes {
            if let Node::Row(inner) = node {
                if inner.is_empty() {
                    continue;
                }
            }
            let mut role = Self::role(node);
            // A binary operator at the start or after an operator/relation/
            // opening is unary (−x): no medium spaces.
            if role == Role::Bin
                && matches!(
                    previous,
                    None | Some(Role::Bin | Role::Rel | Role::Open | Role::Punct | Role::Op)
                )
            {
                role = Role::Ord;
            }
            if let Some(left) = previous {
                // Scripts and fractions inside scripts stay tight.
                let space = Self::spacing(left, role);
                let scale = if self.size < self.base * 0.8 && space > 0.17 {
                    0.0
                } else {
                    1.0
                };
                out.width += space * self.size * scale;
            }
            let laid = self.node(node);
            let x = out.width;
            out.place(&laid, x, 0.0);
            out.width += laid.width;
            out.ascent = out.ascent.max(laid.ascent);
            out.descent = out.descent.max(laid.descent);
            previous = Some(role);
        }
        // A trailing binary operator is ordinary too; nothing to undo since
        // spacing is only added before an atom.
        out
    }

    fn fraction(&self, num: &Node, den: &Node, bar: bool, display: bool) -> Layout {
        let inner = if display {
            self.text_style()
        } else {
            self.script()
        };
        let top = inner.node(num);
        let bottom = inner.node(den);
        let thickness = if bar { self.rule() } else { 0.0 };
        let gap = self.size * if display { 0.14 } else { 0.1 };
        let pad = self.size * 0.12;
        let width = top.width.max(bottom.width) + 2.0 * pad;
        let axis = self.axis();
        let num_baseline = -axis - thickness / 2.0 - gap - top.descent;
        let den_baseline = -axis + thickness / 2.0 + gap + bottom.ascent;

        let mut out = Layout {
            width,
            ascent: -(num_baseline - top.ascent),
            descent: den_baseline + bottom.descent,
            items: Vec::new(),
        };
        out.place(&top, (width - top.width) / 2.0, num_baseline);
        out.place(&bottom, (width - bottom.width) / 2.0, den_baseline);
        if bar {
            out.items.push(Item {
                x: pad * 0.5,
                y: -axis - thickness / 2.0,
                draw: Draw::Rule {
                    width: width - pad,
                    height: thickness,
                },
            });
        }
        out
    }

    fn sqrt(&self, index: Option<&Node>, body: &Node) -> Layout {
        let inner = self.node(body);
        let thickness = self.rule();
        let gap = self.size * 0.12;
        let top = -(inner.ascent.max(self.size * 0.5) + gap + thickness);
        let bottom = inner.descent.max(self.size * 0.05);
        let height = bottom - top;
        let sign_width = self.size * 0.55;

        // Optional index sits above the left hook.
        let index_layout = index.map(|node| self.script().script().node(node));
        let shift = index_layout
            .as_ref()
            .map(|laid| (laid.width - sign_width * 0.45).max(0.0))
            .unwrap_or(0.0);

        let mid = bottom - height * 0.42;
        let points = vec![
            (shift, mid + self.size * 0.04),
            (shift + sign_width * 0.25, mid - self.size * 0.04),
            (shift + sign_width * 0.55, bottom),
            (shift + sign_width, top + thickness / 2.0),
            (
                shift + sign_width + inner.width + self.size * 0.1,
                top + thickness / 2.0,
            ),
        ];
        let mut out = Layout {
            width: shift + sign_width + inner.width + self.size * 0.15,
            ascent: -top,
            descent: bottom,
            items: vec![Item {
                x: 0.0,
                y: 0.0,
                draw: Draw::Line { points, thickness },
            }],
        };
        out.place(&inner, shift + sign_width + self.size * 0.05, 0.0);
        if let Some(laid) = index_layout {
            let baseline = mid - self.size * 0.12;
            out.place(&laid, 0.0, baseline);
            out.ascent = out.ascent.max(-(baseline - laid.ascent));
        }
        out
    }

    fn big_op(&self, text: &str, glyph: bool) -> Layout {
        if !glyph {
            return self.text(text, Style::Upright, self.size);
        }
        let integral = matches!(text, "∫" | "∬" | "∭" | "∮");
        let scale = match (self.display, integral) {
            (true, true) => 1.9,
            (true, false) => 1.55,
            (false, true) => 1.35,
            (false, false) => 1.15,
        };
        let size = self.size * scale;
        let metrics = self.metrics.measure(text, size, Style::Upright);
        let axis = self.axis();
        // Centre the ink on the math axis.
        let baseline = -axis + (metrics.ascent - metrics.descent) / 2.0;
        Layout {
            width: metrics.width + self.size * 0.05,
            ascent: metrics.ascent - baseline,
            descent: metrics.descent + baseline,
            items: vec![Item {
                x: 0.0,
                y: baseline,
                draw: Draw::Text {
                    text: text.to_owned(),
                    size,
                    style: Style::Upright,
                },
            }],
        }
    }

    fn scripts(&self, base: &Node, sup: Option<&Node>, sub: Option<&Node>) -> Layout {
        let limits = matches!(base, Node::BigOp { limits: true, .. }) && self.display;
        let base_layout = self.node(base);
        let small = self.script();
        let sup_layout = sup.map(|node| small.node(node));
        let sub_layout = sub.map(|node| small.node(node));

        if limits {
            return self.limits(base_layout, sup_layout, sub_layout);
        }

        let mut out = Layout::empty();
        out.place(&base_layout, 0.0, 0.0);
        out.width = base_layout.width;
        out.ascent = base_layout.ascent;
        out.descent = base_layout.descent;

        let is_op = matches!(base, Node::BigOp { glyph: true, .. });
        let mut up = (base_layout.ascent - self.size * 0.3).max(self.size * 0.42);
        let mut down = (base_layout.descent + self.size * 0.05).max(self.size * 0.22);
        if is_op {
            up = base_layout.ascent - self.size * 0.35;
            down = base_layout.descent - self.size * 0.1;
        }
        if let (Some(top), Some(bottom)) = (&sup_layout, &sub_layout) {
            down = down.max(self.size * 0.27);
            let clearance = (down - bottom.ascent) - (-up + top.descent);
            let minimum = self.size * 0.12;
            if clearance < minimum {
                down += minimum - clearance;
            }
        }
        let x = base_layout.width + self.size * 0.03;
        let mut extra: f32 = 0.0;
        if let Some(top) = &sup_layout {
            out.place(top, x, -up);
            out.ascent = out.ascent.max(up + top.ascent);
            out.descent = out.descent.max(top.descent - up);
            extra = extra.max(top.width);
        }
        if let Some(bottom) = &sub_layout {
            out.place(bottom, x, down);
            out.descent = out.descent.max(down + bottom.descent);
            out.ascent = out.ascent.max(bottom.ascent - down);
            extra = extra.max(bottom.width);
        }
        out.width = x + extra + self.size * 0.04;
        out
    }

    fn limits(&self, base: Layout, sup: Option<Layout>, sub: Option<Layout>) -> Layout {
        let gap = self.size * 0.12;
        let width = [
            base.width,
            sup.as_ref().map_or(0.0, |l| l.width),
            sub.as_ref().map_or(0.0, |l| l.width),
        ]
        .into_iter()
        .fold(0.0, f32::max);
        let mut out = Layout {
            width,
            ascent: base.ascent,
            descent: base.descent,
            items: Vec::new(),
        };
        out.place(&base, (width - base.width) / 2.0, 0.0);
        if let Some(top) = sup {
            let baseline = -(base.ascent + gap + top.descent);
            out.place(&top, (width - top.width) / 2.0, baseline);
            out.ascent = -(baseline - top.ascent);
        }
        if let Some(bottom) = sub {
            let baseline = base.descent + gap + bottom.ascent;
            out.place(&bottom, (width - bottom.width) / 2.0, baseline);
            out.descent = baseline + bottom.descent;
        }
        out
    }

    /// Wraps `inner` in delimiters scaled to its height, centred on the axis.
    fn delimit(&self, inner: Layout, open: &str, close: &str) -> Layout {
        let axis = self.axis();
        let half = (inner.ascent - axis)
            .max(inner.descent + axis)
            .max(self.size * 0.5);
        let needed = half * 2.0 * 1.05;
        let mut out = Layout::empty();
        let mut x = 0.0;
        let glyph = |text: &str| -> Option<Layout> {
            if text.is_empty() {
                return None;
            }
            let natural = self.metrics.measure(text, self.size, Style::Upright);
            let ink = (natural.ascent + natural.descent).max(1.0);
            let size = self.size * (needed / ink).max(1.0);
            let metrics = self.metrics.measure(text, size, Style::Upright);
            let baseline = -axis + (metrics.ascent - metrics.descent) / 2.0;
            Some(Layout {
                width: metrics.width,
                ascent: metrics.ascent - baseline,
                descent: metrics.descent + baseline,
                items: vec![Item {
                    x: 0.0,
                    y: baseline,
                    draw: Draw::Text {
                        text: text.to_owned(),
                        size,
                        style: Style::Upright,
                    },
                }],
            })
        };
        let absorb = |out: &mut Layout, laid: &Layout, x: &mut f32| {
            out.place(laid, *x, 0.0);
            *x += laid.width;
            out.ascent = out.ascent.max(laid.ascent);
            out.descent = out.descent.max(laid.descent);
        };
        let pad = self.size * 0.06;
        if let Some(left) = glyph(open) {
            absorb(&mut out, &left, &mut x);
            x += pad;
        }
        absorb(&mut out, &inner, &mut x);
        if let Some(right) = glyph(close) {
            x += pad;
            absorb(&mut out, &right, &mut x);
        }
        out.width = x;
        out
    }

    fn accent(&self, kind: AccentKind, body: &Node) -> Layout {
        let inner = self.node(body);
        let thickness = self.rule();
        let gap = self.size * 0.08;
        let width = inner.width.max(self.size * 0.4);
        let mut out = Layout {
            width,
            ascent: inner.ascent,
            descent: inner.descent,
            items: Vec::new(),
        };
        out.place(&inner, (width - inner.width) / 2.0, 0.0);
        let top = -(inner.ascent.max(self.size * 0.45) + gap);
        match kind {
            AccentKind::Bar | AccentKind::Overline => {
                let inset = if kind == AccentKind::Bar {
                    width * 0.12
                } else {
                    0.0
                };
                out.items.push(Item {
                    x: inset,
                    y: top - thickness,
                    draw: Draw::Rule {
                        width: width - 2.0 * inset,
                        height: thickness,
                    },
                });
                out.ascent = -(top - thickness);
            }
            AccentKind::Underline => {
                let y = inner.descent + gap;
                out.items.push(Item {
                    x: 0.0,
                    y,
                    draw: Draw::Rule {
                        width,
                        height: thickness,
                    },
                });
                out.descent = y + thickness;
            }
            AccentKind::Hat => {
                let height = self.size * 0.16;
                let points = vec![
                    (width * 0.22, top),
                    (width * 0.5, top - height),
                    (width * 0.78, top),
                ];
                out.items.push(Item {
                    x: 0.0,
                    y: 0.0,
                    draw: Draw::Line { points, thickness },
                });
                out.ascent = -(top - height - thickness);
            }
            AccentKind::Vec => {
                let y = top - self.size * 0.06;
                let head = self.size * 0.12;
                out.items.push(Item {
                    x: 0.0,
                    y: 0.0,
                    draw: Draw::Line {
                        points: vec![(width * 0.05, y), (width * 0.95, y)],
                        thickness,
                    },
                });
                out.items.push(Item {
                    x: 0.0,
                    y: 0.0,
                    draw: Draw::Line {
                        points: vec![
                            (width * 0.95 - head, y - head * 0.6),
                            (width * 0.95, y),
                            (width * 0.95 - head, y + head * 0.6),
                        ],
                        thickness,
                    },
                });
                out.ascent = -(y - head * 0.6 - thickness);
            }
            AccentKind::Tilde => {
                let amplitude = self.size * 0.05;
                let y = top - amplitude * 1.5;
                let steps = 8;
                let points = (0..=steps)
                    .map(|step| {
                        let t = step as f32 / steps as f32;
                        let x = width * (0.15 + 0.7 * t);
                        let phase = t * std::f32::consts::TAU;
                        (x, y - amplitude * phase.sin())
                    })
                    .collect();
                out.items.push(Item {
                    x: 0.0,
                    y: 0.0,
                    draw: Draw::Line { points, thickness },
                });
                out.ascent = -(y - amplitude * 2.0 - thickness);
            }
            AccentKind::Dot | AccentKind::DDot => {
                let dot = (self.size * 0.11).max(1.5);
                let centres: &[f32] = if kind == AccentKind::Dot {
                    &[0.5]
                } else {
                    &[0.32, 0.68]
                };
                for centre in centres {
                    out.items.push(Item {
                        x: width * centre - dot / 2.0,
                        y: top - dot,
                        draw: Draw::Rule {
                            width: dot,
                            height: dot,
                        },
                    });
                }
                out.ascent = -(top - dot);
            }
        }
        out
    }

    fn matrix(&self, rows: &[Vec<Node>], left_aligned: bool) -> Layout {
        let cell_context = self.text_style();
        let laid: Vec<Vec<Layout>> = rows
            .iter()
            .map(|row| row.iter().map(|cell| cell_context.node(cell)).collect())
            .collect();
        let columns = laid.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0.0f32; columns];
        for row in &laid {
            for (index, cell) in row.iter().enumerate() {
                widths[index] = widths[index].max(cell.width);
            }
        }
        let column_gap = self.size * if left_aligned { 1.0 } else { 0.8 };
        let row_gap = self.size * 0.3;
        let strut_ascent = self.size * 0.7;
        let strut_descent = self.size * 0.25;

        let mut out = Layout::empty();
        let mut y = 0.0f32;
        let mut first = true;
        for row in &laid {
            let ascent = row
                .iter()
                .map(|cell| cell.ascent)
                .fold(strut_ascent, f32::max);
            let descent = row
                .iter()
                .map(|cell| cell.descent)
                .fold(strut_descent, f32::max);
            if first {
                y = ascent;
                first = false;
            } else {
                y += row_gap + ascent;
            }
            let mut x = 0.0;
            for (index, cell) in row.iter().enumerate() {
                let offset = if left_aligned {
                    0.0
                } else {
                    (widths[index] - cell.width) / 2.0
                };
                out.place(cell, x + offset, y);
                x += widths[index] + column_gap;
            }
            y += descent;
        }
        let height = y;
        out.width = widths.iter().sum::<f32>() + column_gap * (columns.saturating_sub(1)) as f32;
        // Rows were placed from y = 0 at the top; centre the block on the axis.
        let shift = -height / 2.0 - self.axis();
        let mut centred = Layout {
            width: out.width,
            ascent: -shift,
            descent: height + shift,
            items: Vec::new(),
        };
        centred.place(&out, 0.0, shift);
        centred
    }
}

#[cfg(test)]
mod tests {
    use super::super::parse::parse;
    use super::*;

    /// Every glyph is 0.5 em wide, 0.7 em above and 0.2 em below the baseline.
    struct Fixed;
    impl Metrics for Fixed {
        fn measure(&self, text: &str, size: f32, _: Style) -> TextMetrics {
            TextMetrics {
                width: text.chars().count() as f32 * size * 0.5,
                ascent: size * 0.7,
                descent: size * 0.2,
            }
        }
    }

    fn lay(source: &str) -> Layout {
        layout(&parse(source).unwrap(), &Fixed, 20.0)
    }

    #[test]
    fn superscript_raises_and_widens() {
        let plain = lay("x");
        let squared = lay("x^2");
        assert!(squared.width > plain.width);
        assert!(squared.ascent > plain.ascent);
    }

    #[test]
    fn fraction_stacks_above_and_below_axis() {
        let frac = lay(r"\frac{a}{b}");
        assert!(frac.ascent > 20.0 * 0.7, "numerator above the line");
        assert!(frac.descent > 20.0 * 0.2, "denominator below the baseline");
        assert!(frac
            .items
            .iter()
            .any(|item| matches!(item.draw, Draw::Rule { .. })));
    }

    #[test]
    fn binary_operators_get_space_but_unary_minus_does_not() {
        let binary = lay("a-b");
        let unary = lay("-ab");
        assert!(binary.width > unary.width);
    }

    #[test]
    fn display_sum_puts_limits_above_and_below() {
        let sum = lay(r"\sum_{i=1}^{n} i");
        let texts: Vec<&Item> = sum
            .items
            .iter()
            .filter(|item| matches!(&item.draw, Draw::Text { text, .. } if text == "n"))
            .collect();
        assert_eq!(texts.len(), 1);
        assert!(texts[0].y < -20.0, "upper limit sits above the operator");
    }

    #[test]
    fn matrix_and_sqrt_lay_out() {
        let matrix = lay(r"\begin{bmatrix} 1 & 2 \\ 3 & 4 \end{bmatrix}");
        assert!(matrix.ascent + matrix.descent > 2.0 * 20.0 * 0.9);
        let root = lay(r"\sqrt{x}");
        assert!(root
            .items
            .iter()
            .any(|item| matches!(item.draw, Draw::Line { .. })));
    }
}
