// FIXME maybe: accumulate nested prefixes instead of concatenating repeatedly.
// Probably switch to an append-only implementation.
// FIXME: need to cap prefix size/set a lower bound on width to prevent
// underflow and broken layout
// FIXME: probably should escape the actual escape char if it appears in source
use std::iter::iter;

use crossterm::Command;
use crossterm::style::SetStyle;
use markdown::mdast::{Blockquote, Definition, FootnoteDefinition, Heading, InlineCode, List, Node, Text};
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

/// Renders phrasing content (the inline children of a paragraph or heading).
/// The returned iterator is owned, so `children` need only live for the
/// duration of this call.
fn phrasing_to_rows(
    ctx: Context,
    children: &[Node],
) -> Box<dyn Iterator<Item = String>> {
    let mut builder = MarkupBuilder::new(ctx);
    builder.push_all(children);
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

/// Renders a heading: `#{depth} ` prefix followed by the heading content.
fn heading_to_rows(
    ctx: Context,
    heading: &Heading,
) -> Box<dyn Iterator<Item = String>> {
    let mut children = Vec::with_capacity(heading.children.len() + 1);
    children.push(Node::Text(Text {
        value: format!("{} ", "#".repeat(heading.depth as usize)),
        position: None,
    }));
    children.extend(heading.children.iter().cloned());
    phrasing_to_rows(ctx.with_base_style(ctx.theme.text_header), &children)
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

/// Renders a thematic break as a row of `width` hyphens.
fn thematic_break_to_rows(
    ctx: Context,
) -> Box<dyn Iterator<Item = String>> {
    Box::new(iter!(move || {
        let mut out = String::with_capacity(ctx.width + 32);
        ctx.update_style(&mut out, ctx.base_style, ctx.theme.text_subtle);
        out.push_str(&"┄".repeat(ctx.width));
        yield out;
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
            flow_to_rows(
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
    spread: bool,
    children: &'a [Node],
) -> Box<dyn Iterator<Item = String> + 'a> {
    let mut children = children
        .iter()
        .map(move |child| {
            flow_to_rows(ctx.with_base_style(ctx.theme.text_base), child)
        });

    Box::new(iter!(move || {
        let mut first = true;
        for rows in &mut children {
            if !first && spread {
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
    let children = flow_content_to_rows(ctx.with_width(inner_width), true, &footnote.children);
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

/// Renders a list. Unordered lists prefix the first row of each item with
/// "- ", ordered lists with a left-aligned number. Continuation rows are
/// indented to align with the item content. When the list is spread, a blank
/// line separates the children.
fn list_to_rows<'a>(
    ctx: Context,
    list: &'a List,
) -> Box<dyn Iterator<Item = String> + 'a> {
    // The number prefix is padded so all items share a common width
    let prefix_width = if list.ordered {
        3 + list.children.len().ilog10() as usize
    } else {
        2
    };
    // Reserve the prefix width for the number/indent
    let inner_width = ctx.width.saturating_sub(prefix_width);
    let indent = " ".repeat(prefix_width);

    let children = list
        .children
        .iter()
        .enumerate()
        .map(move |(i, child)| {
            let prefix = if list.ordered {
                format!("{:<width$}", format!("{}.", i + 1), width = prefix_width)
            } else {
                "- ".to_string()
            };
            (flow_to_rows(ctx.with_width(inner_width), child), prefix)
        });

    Box::new(iter!(move || {
        let mut first = true;
        for (rows, prefix) in children {
            if !first && list.spread {
                yield String::new();
            }
            first = false;
            for (i, row) in rows.enumerate() {
                let p = if i == 0 { &prefix } else { &indent };
                let mut out = String::with_capacity(p.len() + row.len());
                out.push_str(p);
                out.push_str(&row);
                yield out;
            }
        }
    })())
}

fn flow_to_rows<'a>(
    ctx: Context,
    node: &'a Node,
) -> Box<dyn Iterator<Item = String> + 'a> {
    match node {
        Node::Root(root) => flow_content_to_rows(ctx, true, &root.children),

        Node::Table(_)
        | Node::TableRow(_)
        | Node::TableCell(_) => todo!(),

        Node::Paragraph(paragraph) => phrasing_to_rows(ctx, &paragraph.children),
        Node::Blockquote(quote) => blockquote_to_rows(ctx, quote),
        Node::Code(code) => preformatted_to_rows(ctx, ctx.theme.text_code, &code.value),
        Node::Math(math) => preformatted_to_rows(ctx, ctx.theme.text_math, &math.value),
        Node::FootnoteDefinition(footnote) => footnote_definition_to_rows(ctx, footnote),
        Node::Definition(definition) => definition_to_rows(ctx, definition),
        Node::Html(html) => preformatted_to_rows(ctx, ctx.theme.text_code, &html.value),
        Node::Heading(heading) => heading_to_rows(ctx, heading),
        Node::ThematicBreak(_) => thematic_break_to_rows(ctx),
        Node::ListItem(list_item) => {
            flow_content_to_rows(ctx, list_item.spread, &list_item.children)
        },
        Node::List(list) => list_to_rows(ctx, list),

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
    text: &str,
) -> Vec<String> {
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
    .expect("invalid markdown");

    let mut style_prefix = String::new();
    let _ = SetStyle(theme.text_base.into()).write_ansi(&mut style_prefix);

    flow_to_rows(
        Context {
            theme,
            width,
            base_style: theme.text_base,
            block_quote: false,
            code: false,
        },
        &node,
    )
    .map(|row| format!("{}{}", style_prefix, &row))
    .collect()
}

#[cfg(test)]
mod test_paragraph {
    use crossterm::Command;
    use crossterm::style::SetStyle;

    use crate::ui::style::THEME_DARK;

    fn render(text: &str, width: usize) -> String {
        let mut lines = super::render_markdown(&THEME_DARK, width, text);

        // In tests, strip the style initialization commands for readability
        let mut prefix = String::new();
        let _ = SetStyle(THEME_DARK.text_base.into()).write_ansi(&mut prefix);
        for line in lines.iter_mut() {
            *line = line.trim_start_matches(&prefix).to_owned();
        }

        lines.join("\n")
    }

    #[test]
    fn plain() {
        assert_eq!(render("hello world", 80), "hello world");
    }

    #[test]
    fn bold() {
        assert_eq!(render("**hi**", 80), "\x1b[1mhi");
    }

    #[test]
    fn inline_spacing() {
        assert_eq!(render("a*b*c", 80), "a\x1b[3mb\x1b[23mc");
        assert_eq!(render("a *h*c", 80), "a \x1b[3mh\x1b[23mc");
        assert_eq!(render("a*h* c", 80), "a\x1b[3mh\x1b[23m c");
        assert_eq!(render("a *h* c", 80), "a \x1b[3mh\x1b[23m c");
    }

    #[test]
    fn wrap() {
        assert_eq!(
            render("hello world foo", 8),
            "hello \nworld \nfoo",
        );
    }

    #[test]
    fn styles() {
        assert_eq!(
            render("**bold** *italic*\\\n`code`", 80),
            "\x1b[1mbold\x1b[22m \x1b[3mitalic\n\x1b[38;2;254;240;138mcode",
        );
    }

    #[test]
    fn blockquote() {
        assert_eq!(
            render("> hello *world*", 80),
            "\x1b[38;2;168;162;158m\x1b[3m▕hello world",
        );
        assert_eq!(
            render("> **bold** `code`", 80),
            "\x1b[38;2;168;162;158m\x1b[3m▕bold code",
        );
        assert_eq!(
            render("> hello world foo", 8),
            "\x1b[38;2;168;162;158m\x1b[3m▕hello \n\x1b[38;2;168;162;158m\x1b[3m▕world \n\x1b[38;2;168;162;158m\x1b[3m▕foo",
        );
    }

    #[test]
    fn two_paragraphs() {
        assert_eq!(
            render("first paragraph\n\nsecond paragraph", 80),
            "first paragraph\n\nsecond paragraph",
        );
    }

    #[test]
    fn code() {
        assert_eq!(
            render("```\nfn main() {}\n```", 80),
            "\x1b[38;2;254;240;138mfn main() {}",
        );
    }

    #[test]
    fn code_wrap() {
        assert_eq!(
            render("```\nabcdefgh\n```", 4),
            "\x1b[38;2;254;240;138mabcd\n\x1b[38;2;254;240;138mefgh",
        );
    }

    #[test]
    fn math() {
        assert_eq!(
            render("$$\nx^2 + y^2 = z^2\n$$", 80),
            "\x1b[38;2;254;240;138m\x1b[3mx^2 + y^2 = z^2",
        );
    }

    #[test]
    fn inline_math() {
        assert_eq!(
            render("a $x^2$ b", 80),
            "a \x1b[38;2;254;240;138m\x1b[3mx^2\x1b[38;5;15m\x1b[23m b",
        );
    }

    #[test]
    fn footnote_definition() {
        assert_eq!(
            render("text[^1]\n\n[^1]: the note", 80),
            "text[^1]\n\n[^1]:\n  the note",
        );
    }

    #[test]
    fn footnote_definition_wrap() {
        assert_eq!(
            render("[^1]: aaaabbbb", 8),
            "[^1]:\n  aaaabb\n  bb",
        );
    }

    #[test]
    fn definition() {
        assert_eq!(
            render("[foo]: https://example.com", 80),
            "[foo]: https://example.com",
        );
    }

    #[test]
    fn definition_wrap() {
        assert_eq!(
            render("[foo]: https://example.com", 16),
            "[foo]: https://e\nxample.com",
        );
    }

    #[test]
    fn html() {
        assert_eq!(
            render("<div>\n<p>hi</p>\n</div>", 80),
            "\x1b[38;2;254;240;138m<div>\n\x1b[38;2;254;240;138m<p>hi</p>\n\x1b[38;2;254;240;138m</div>",
        );
    }

    #[test]
    fn heading() {
        assert_eq!(render("# Hello", 80), "# Hello");
        assert_eq!(render("#### Deep", 80), "#### Deep");
        assert_eq!(
            render("## Hello *world*", 80),
            "## Hello \x1b[3mworld",
        );
    }

    #[test]
    fn thematic_break() {
        assert_eq!(
            render("---", 10),
            "\x1b[38;2;168;162;158m┄┄┄┄┄┄┄┄┄┄",
        );
    }

    #[test]
    fn unordered_list() {
        assert_eq!(render("- a", 80), "- a");
        assert_eq!(render("- a\n- b", 80), "- a\n- b");
        assert_eq!(
            render("- a\n\n- b", 80),
            "- a\n\n- b",
        );
        assert_eq!(
            render("- *hi*", 80),
            "- \x1b[3mhi",
        );
        assert_eq!(
            render("- hello world foo", 8),
            "- hello \n  world \n  foo",
        );
    }

    #[test]
    fn ordered_list() {
        assert_eq!(render("1. a\n2. b", 80), "1. a\n2. b");

        let md = (1..=10)
            .map(|i| format!("{}. item", i))
            .collect::<Vec<_>>()
            .join("\n");
        let expected: String = (1..=10)
            .map(|i| format!("{:<4}{}", format!("{}.", i), "item"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(render(&md, 80), expected);

        assert_eq!(
            render("1. hello world foo", 8),
            "1. hello\n    \n   world\n    foo",
        );

        assert_eq!(render("1. a\n1. b", 80), "1. a\n2. b");
    }

    #[test]
    fn nested_lists() {
        // N.B. Commonmark requires nested lists to be indented 4 spaces, but
        // we render aligned with the other ListItem siblings
        assert_eq!(
            render("- one\n  - two\n  - three", 12),
            "- one\n  - two\n  - three",
        );
        assert_eq!(
            render("- one\n  1. two\n  2. three", 12),
            "- one\n  1. two\n  2. three",
        );
        assert_eq!(
            render("1. one\n    - two\n    - three", 12),
            "1. one\n   - two\n   - three",
        );
        assert_eq!(
            render("1. one\n    1. two\n    2. three", 12),
            "1. one\n   1. two\n   2. three",
        );
    }
}
