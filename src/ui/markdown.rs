// FIXME maybe: accumulate nested prefixes instead of concatenating repeatedly.
// FIXME: need to cap prefix size/set a lower bound on width to prevent
// underflow and broken layout
// FIXME: probably should escape actual escape sequences if they appear in the
// source
use std::iter::iter;

use crossterm::Command;
use markdown::mdast::{Blockquote, Definition, FootnoteDefinition, InlineCode, Node, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::ui::style::{TextStyle, Theme, UpdateStyle};
use crate::ui::text::{wrap_line, wrap_line_naive, SPACES, TAB_WIDTH};

/// Out-of-line style marker.
#[derive(Debug)]
struct Marker {
    /// Byte offset to insert control sequence in plain_text
    offset: usize,
    /// Style to apply
    style: TextStyle,
}

/// Rendering state threaded through the markdown renderer.
#[derive(Clone, Copy)]
struct Context {
    theme: &'static Theme,
    width: usize,
    base_style: TextStyle,
    // FIXME: Current behavior is a slightly crude hack; ideally block quotes
    // would flip the meaning of italic and non-italic
    block_quote: bool,
    code: bool,
}

impl Context {
    const fn with_width(mut self, width: usize) -> Self {
        self.width = width;
        self
    }

    const fn with_theme(mut self, theme: &'static Theme) -> Self {
        self.theme = theme;
        self
    }

    const fn with_base_style(mut self, base_style: TextStyle) -> Self {
        self.base_style = base_style;
        self
    }

    const fn set_block_quote(mut self, block_quote: bool) -> Self {
        self.block_quote = block_quote;
        self
    }

    const fn set_code(mut self, code: bool) -> Self {
        self.code = code;
        self
    }

    fn update_style(&self, out: &mut String, prev: TextStyle, style: TextStyle) {
        if self.block_quote || self.code {
            return;
        }
        let _ = UpdateStyle(prev, style).write_ansi(out);
    }
}

#[derive(Debug)]
struct MarkupBuilder {
    theme: &'static Theme,
    plain_text: String,
    markers: Vec<Marker>,
    cur_style: TextStyle,
}

/// Replaces runs of whitespace with a single space character. Preserves
/// leading and trailing whitespace.
///
/// Technically we should not eliminate whitespace that is combined with a
/// diacritic, e.g. " \u{0301}", but no one renders this correctly, making
/// usage so rare that it is not worth supporting. Poor design decision by the
/// Unicode Consortium.
fn collapse_whitespace(out: &mut String, s: &str) {
    if s.starts_with(char::is_whitespace) {
        out.push(' ');
    }

    let mut words = s.split_whitespace();
    let Some(t) = words.next() else { return };
    out.push_str(t);

    for t in words {
        out.push(' ');
        out.push_str(t);
    }

    if s.ends_with(char::is_whitespace) {
        out.push(' ');
    }
}

impl MarkupBuilder {
    fn new(ctx: Context) -> Self {
        Self {
            theme: ctx.theme,
            plain_text: String::new(),
            markers: Vec::new(),
            cur_style: ctx.base_style,
        }
    }

    fn push_text(&mut self, s: &str) {
        collapse_whitespace(&mut self.plain_text, s);
    }

    /// Removes newlines, expands tabs, but preserves otherwise whitespace.
    fn push_inline_code(&mut self, s: &str) {
        let mut col = self.plain_text.width();
        for c in s.chars() {
            match c {
                '\n' | '\r' => {
                    self.plain_text.push(' ');
                    col += 1;
                }
                '\t' => {
                    let n = TAB_WIDTH - col % TAB_WIDTH;
                    self.plain_text.push_str(&SPACES[..n]);
                    col += n;
                }
                _ => {
                    self.plain_text.push(c);
                    if let Some(w) = UnicodeWidthChar::width(c) {
                        col += w;
                    }
                }
            }
        }
    }

    fn push_all(&mut self, nodes: &[Node]) {
        for node in nodes {
            self.push_node(node);
        }
    }

    fn set_style(&mut self, style: TextStyle) {
        if style != self.cur_style {
            self.markers.push(Marker {
                offset: self.plain_text.len(),
                style: style,
            });
            self.cur_style = style;
        }
    }

    fn push_node(&mut self, node: &Node) {
        match node {
            // Plain text
            Node::Text(text) => self.push_text(&text.value),
            Node::Break(_) => self.plain_text.push('\n'),

            // Styling
            Node::InlineCode(inner) => {
                let prev = self.cur_style;
                self.set_style(self.theme.text_code);
                self.push_inline_code(&inner.value);
                self.set_style(prev);
            }
            Node::Emphasis(inner) => {
                let prev = self.cur_style;
                self.set_style(prev.italicized());
                self.push_all(&inner.children);
                self.set_style(prev);
            }
            Node::Strong(inner) => {
                let prev = self.cur_style;
                self.set_style(prev.bolded());
                self.push_all(&inner.children);
                self.set_style(prev);
            }
            Node::Delete(inner) => {
                let prev = self.cur_style;
                self.set_style(prev.struck_out());
                self.push_all(&inner.children);
                self.set_style(prev);
            }

            // Nodes that can't be rendered faithfully in the terminal; we
            // fall back to rendering as plaintext.
            Node::Link(link) => {
                self.push_text("![");
                self.push_all(&link.children);
                self.push_text("]");
                self.push_text("[");
                self.push_text(&link.url);
                self.push_text("]");
            }
            Node::LinkReference(link) => {
                self.push_text("![");
                self.push_all(&link.children);
                self.push_text("]");
                self.push_text("[");
                self.push_text(&link.identifier);
                self.push_text("]");
            }
            Node::Image(image) => {
                self.push_text("![");
                self.push_text(&image.alt);
                self.push_text("]");
                self.push_text("(");
                self.push_text(&image.url);
                if let Some(title) = &image.title {
                    self.push_text("\"");
                    // FIXME: Should escape non-printable characters and \"
                    self.push_text(&title);
                    self.push_text("\"");
                }
                self.push_text(")");
            }
            Node::ImageReference(image) => {
                self.push_text("![");
                self.push_text(&image.alt);
                self.push_text("]");
                self.push_text("[");
                self.push_text(&image.identifier);
                self.push_text("]");
            }
            Node::InlineMath(math) => {
                let prev = self.cur_style;
                self.set_style(self.theme.text_math);
                self.push_inline_code(&math.value);
                self.set_style(prev);
            }
            Node::FootnoteReference(footnote) => {
                self.push_text("[^");
                self.push_text(&footnote.identifier);
                self.push_text("]");
            }
            Node::Html(html) => {
                // Treat HTML as code
                self.push_node(&Node::InlineCode(InlineCode {
                    value: html.value.clone(),
                    position: html.position.clone(),
                }));
            }

            // Non-phrasing nodes
            Node::Root(_)
            | Node::Blockquote(_)
            | Node::FootnoteDefinition(_)
            | Node::MdxJsxFlowElement(_)
            | Node::List(_)
            | Node::MdxjsEsm(_)
            | Node::Toml(_)
            | Node::Yaml(_)
            | Node::Code(_)
            | Node::Math(_)
            | Node::MdxFlowExpression(_)
            | Node::Heading(_)
            | Node::Table(_)
            | Node::ThematicBreak(_)
            | Node::TableRow(_)
            | Node::TableCell(_)
            | Node::ListItem(_)
            | Node::Definition(_)
            | Node::Paragraph(_)
            // Unsupported nodes, library shouldn't produce these
            | Node::MdxTextExpression(_)
            | Node::MdxJsxTextElement(_)
                => unreachable!("broken markdown AST"),
        }
    }
}

fn paragraph_to_rows<'a>(
    ctx: Context,
    paragraph: &'a Paragraph,
) -> Box<dyn Iterator<Item = String> + 'a> {
    let mut builder = MarkupBuilder::new(ctx);
    builder.push_all(&paragraph.children);
    let MarkupBuilder {
        plain_text,
        markers,
        ..
    } = builder;

    Box::new(iter!(move || {
        let mut cur_style = ctx.base_style;
        let mut marker_idx = 0usize;
        let mut offset = 0usize;

        for line in plain_text.split('\n') {
            let rows = wrap_line(ctx.width, line);

            for row in rows {
                let mut out = String::with_capacity(2 * row.graphemes.len());

                // Re-apply current style at start of each row
                while marker_idx < markers.len() && markers[marker_idx].offset <= offset {
                    cur_style = markers[marker_idx].style;
                    marker_idx += 1;
                }
                ctx.update_style(&mut out, ctx.base_style, cur_style);

                for g in &row.graphemes {
                    // Newline added by wrap_line, not part of the source text
                    if g.data == "\n" {
                        continue;
                    }

                    let prev_style = cur_style;
                    while marker_idx < markers.len() && markers[marker_idx].offset <= offset {
                        cur_style = markers[marker_idx].style;
                        marker_idx += 1;
                    }
                    ctx.update_style(&mut out, prev_style, cur_style);

                    out.push_str(g.formatted());
                    offset += g.data.len();
                }

                yield out;
            }

            // Skip the newline separating this line from the next
            offset += 1;
        }
    })())
}

/// Wraps `text` at `width` using naive wrapping and renders each row to a
/// plain string.
fn wrap_naive_rows(width: usize, text: &str) -> impl Iterator<Item = String> + use<> {
    wrap_line_naive(width, text).into_iter().map(|row| {
        let mut out = String::with_capacity(row.graphemes.len());
        for g in &row.graphemes {
            // Newline added by wrap_line_naive, not part of the source
            if g.data == "\n" {
                continue;
            }
            out.push_str(g.formatted());
        }
        out
    })
}

fn preformatted_to_rows<'a>(
    ctx: Context,
    style: TextStyle,
    value: &'a str,
) -> Box<dyn Iterator<Item = String> + 'a> {
    // Style updates are frozen so the preformatted content renders with one
    // uniform style.
    let pre_ctx = ctx
        .with_base_style(style)
        .set_code(true);

    Box::new(iter!(move || {
        for line in value.split('\n') {
            for row in wrap_naive_rows(pre_ctx.width, line) {
                let mut out = String::with_capacity(row.len() + 32);
                // Step into the preformatted style from the enclosing context's
                // style
                ctx.update_style(&mut out, ctx.base_style, pre_ctx.base_style);
                out.push_str(&row);
                yield out;
            }
        }
    })())
}

fn blockquote_to_rows<'a>(
    ctx: Context,
    quote: &'a Blockquote,
) -> Box<dyn Iterator<Item = String> + 'a> {
    // One column for the left border prefix
    let inner_width = ctx.width.saturating_sub(1);

    let content_style = ctx.theme.text_subtle.italicized();

    let mut children = quote
        .children
        .iter()
        .map(move |child| {
            to_rows(
                ctx.with_width(inner_width)
                    .with_base_style(content_style)
                    .set_block_quote(true),
                child,
            )
        });

    Box::new(iter!(move || {
        for rows in &mut children {
            for row in rows {
                let mut out = String::with_capacity(row.len() + 16);
                ctx.update_style(&mut out, ctx.base_style, ctx.theme.text_quote);
                out.push('\u{2595}');
                out.push_str(&row);
                yield out;
            }
        }
    })())
}

fn flow_content_to_rows<'a>(
    ctx: Context,
    children: &'a [Node],
) -> Box<dyn Iterator<Item = String> + 'a> {
    let mut children = children
        .iter()
        .map(move |child| {
            to_rows(ctx.with_base_style(ctx.theme.text_base), child)
        });

    Box::new(iter!(move || {
        let mut first = true;
        for rows in &mut children {
            if !first {
                yield String::new();
            }
            first = false;
            for row in rows {
                yield row;
            }
        }
    })())
}

fn footnote_definition_to_rows<'a>(
    ctx: Context,
    footnote: &'a FootnoteDefinition,
) -> Box<dyn Iterator<Item = String> + 'a> {
    // Reserve two columns for the indent prefix
    let inner_width = ctx.width.saturating_sub(2);
    let children = flow_content_to_rows(ctx.with_width(inner_width), &footnote.children);
    let header = format!("[^{}]:", footnote.identifier);
    Box::new(
        wrap_naive_rows(ctx.width, &header)
            .chain(children.map(|row| format!("  {}", row))),
    )
}

fn definition_to_rows<'a>(
    ctx: Context,
    definition: &'a Definition,
) -> Box<dyn Iterator<Item = String> + 'a> {
    let line = format!("[{}]: {}", definition.identifier, definition.url);
    Box::new(wrap_naive_rows(ctx.width, &line))
}

fn to_rows<'a>(
    ctx: Context,
    node: &'a Node,
) -> Box<dyn Iterator<Item = String> + 'a> {
    match node {
        Node::Root(root) => flow_content_to_rows(ctx, &root.children),

        Node::List(_)
        | Node::Heading(_)
        | Node::Table(_)
        | Node::ThematicBreak(_)
        | Node::TableRow(_)
        | Node::TableCell(_)
        | Node::ListItem(_)
        | Node::Html(_) => todo!(),

        Node::Paragraph(paragraph) => paragraph_to_rows(ctx, paragraph),
        Node::Blockquote(quote) => blockquote_to_rows(ctx, quote),
        Node::Code(code) => preformatted_to_rows(ctx, ctx.theme.text_code, &code.value),
        Node::Math(math) => preformatted_to_rows(ctx, ctx.theme.text_math, &math.value),
        Node::FootnoteDefinition(footnote) => footnote_definition_to_rows(ctx, footnote),
        Node::Definition(definition) => definition_to_rows(ctx, definition),

        // Phrasing nodes
        | Node::Break(_)
        | Node::InlineCode(_)
        | Node::InlineMath(_)
        | Node::Delete(_)
        | Node::Emphasis(_)
        | Node::MdxTextExpression(_)
        | Node::MdxJsxFlowElement(_)
        | Node::MdxFlowExpression(_)
        | Node::MdxjsEsm(_)
        | Node::FootnoteReference(_)
        | Node::Image(_)
        | Node::ImageReference(_)
        | Node::MdxJsxTextElement(_)
        | Node::Link(_)
        | Node::LinkReference(_)
        | Node::Strong(_)
        | Node::Text(_)
        // Unsupported nodes, library shouldn't produce these
        | Node::Toml(_)
        | Node::Yaml(_)
        => unreachable!("broken markdown AST")
    }
}

/// Converts a markdown document into preformatted lines ready to be printed
/// to stdout.
pub fn render_markdown(
    theme: &'static Theme,
    width: usize,
) -> Vec<String> {
    todo!()
}
#[cfg(test)]
mod test_paragraph {
    use crate::ui::style::THEME_DARK;

    fn render(text: &str, width: usize) -> Vec<String> {
        let node = markdown::to_mdast(
            text,
            &markdown::ParseOptions {
                constructs: markdown::Constructs {
                    gfm_footnote_definition: true,
                    gfm_label_start_footnote: true,
                    math_flow: true,
                    math_text: true,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .unwrap();
        super::to_rows(
            super::Context {
                theme: &THEME_DARK,
                width,
                base_style: THEME_DARK.text_base,
                block_quote: false,
                code: false,
            },
            &node,
        )
        .collect()
    }

    #[test]
    fn plain() {
        assert_eq!(render("hello world", 80), vec!["hello world"]);
    }

    #[test]
    fn bold() {
        assert_eq!(render("**hi**", 80), vec!["\x1b[1mhi"]);
    }

    #[test]
    fn inline_spacing() {
        assert_eq!(render("a*b*c", 80), vec!["a\x1b[3mb\x1b[23mc"]);
        assert_eq!(render("a *h*c", 80), vec!["a \x1b[3mh\x1b[23mc"]);
        assert_eq!(render("a*h* c", 80), vec!["a\x1b[3mh\x1b[23m c"]);
        assert_eq!(render("a *h* c", 80), vec!["a \x1b[3mh\x1b[23m c"]);
    }

    #[test]
    fn wrap() {
        assert_eq!(render("hello world foo", 8), vec![
            "hello ",
            "world ",
            "foo",
        ]);
    }

    #[test]
    fn styles() {
        assert_eq!(
            render("**bold** *italic*\\\n`code`", 80),
            vec![
                "\x1b[1mbold\x1b[22m \x1b[3mitalic",
                "\x1b[38;2;254;240;138mcode",
            ],
        );
    }

    #[test]
    fn blockquote() {
        assert_eq!(
            render("> hello *world*", 80),
            vec!["\x1b[38;2;168;162;158m\x1b[3m▕hello world"],
        );
    }

    #[test]
    fn blockquote_freezes_style() {
        assert_eq!(
            render("> **bold** `code`", 80),
            vec!["\x1b[38;2;168;162;158m\x1b[3m▕bold code"],
        );
    }

    #[test]
    fn blockquote_wrap() {
        assert_eq!(
            render("> hello world foo", 8),
            vec![
                "\x1b[38;2;168;162;158m\x1b[3m▕hello ",
                "\x1b[38;2;168;162;158m\x1b[3m▕world ",
                "\x1b[38;2;168;162;158m\x1b[3m▕foo",
            ],
        );
    }

    #[test]
    fn two_paragraphs() {
        assert_eq!(
            render("first paragraph\n\nsecond paragraph", 80),
            vec!["first paragraph", "", "second paragraph"],
        );
    }

    #[test]
    fn code() {
        assert_eq!(
            render("```\nfn main() {}\n```", 80),
            vec!["\x1b[38;2;254;240;138mfn main() {}"],
        );
    }

    #[test]
    fn code_wrap() {
        assert_eq!(
            render("```\nabcdefgh\n```", 4),
            vec![
                "\x1b[38;2;254;240;138mabcd",
                "\x1b[38;2;254;240;138mefgh",
            ],
        );
    }

    #[test]
    fn math() {
        assert_eq!(
            render("$$\nx^2 + y^2 = z^2\n$$", 80),
            vec!["\x1b[38;2;254;240;138m\x1b[3mx^2 + y^2 = z^2"],
        );
    }

    #[test]
    fn inline_math() {
        assert_eq!(
            render("a $x^2$ b", 80),
            vec!["a \x1b[38;2;254;240;138m\x1b[3mx^2\x1b[38;5;15m\x1b[23m b"],
        );
    }

    #[test]
    fn footnote_definition() {
        assert_eq!(
            render("text[^1]\n\n[^1]: the note", 80),
            vec!["text[^1]", "", "[^1]:", "  the note"],
        );
    }

    #[test]
    fn footnote_definition_wrap() {
        assert_eq!(
            render("[^1]: aaaabbbb", 8),
            vec!["[^1]:", "  aaaabb", "  bb"],
        );
    }

    #[test]
    fn definition() {
        assert_eq!(
            render("[foo]: https://example.com", 80),
            vec!["[foo]: https://example.com"],
        );
    }

    #[test]
    fn definition_wrap() {
        assert_eq!(
            render("[foo]: https://example.com", 16),
            vec!["[foo]: https://e", "xample.com"],
        );
    }
}
