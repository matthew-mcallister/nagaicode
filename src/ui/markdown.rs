// FIXME maybe: accumulate nested prefixes instead of concatenating repeatedly.
use std::iter::iter;

use crossterm::Command;
use markdown::mdast::{Blockquote, InlineCode, Node, Paragraph, Root};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::ui::style::{TextStyle, Theme, UpdateStyle};
use crate::ui::text::{wrap_line, SPACES, TAB_WIDTH};

/// Out-of-line style marker.
#[derive(Debug)]
struct Marker {
    /// Byte offset to insert control sequence in plain_text
    offset: usize,
    /// Style to apply
    style: TextStyle,
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
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    if s.starts_with(char::is_whitespace) {
        out.push(' ');
    }

    let mut words = s.split_whitespace();
    let Some(t) = words.next() else { return out };
    out.push_str(t);

    for t in words {
        out.push(' ');
        out.push_str(t);
    }

    if s.ends_with(char::is_whitespace) {
        out.push(' ');
    }
    out
}

fn update_style(out: &mut String, prev: TextStyle, style: TextStyle) {
    let _ = UpdateStyle(prev, style).write_ansi(out);
}

impl MarkupBuilder {
    fn new(theme: &'static Theme, base: TextStyle) -> Self {
        Self {
            theme,
            plain_text: String::new(),
            markers: Vec::new(),
            cur_style: base,
        }
    }

    fn push_text(&mut self, s: &str) {
        self.plain_text.push_str(&collapse_whitespace(s));
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

            // Nodes that can't be rendered faithfully in the terminal
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
                self.push_text("$");
                self.push_text(&math.value);
                self.push_text("$");
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
    theme: &'static Theme,
    width: usize,
    style: TextStyle,
    paragraph: &'a Paragraph,
) -> Box<dyn Iterator<Item = String> + 'a> {
    let mut builder = MarkupBuilder::new(theme, style);
    builder.push_all(&paragraph.children);
    let MarkupBuilder {
        plain_text,
        markers,
        ..
    } = builder;

    Box::new(iter!(move || {
        let mut cur_style = style;
        let mut marker_idx = 0usize;
        let mut offset = 0usize;

        for line in plain_text.split('\n') {
            let rows = wrap_line(width, line);

            for row in rows {
                let mut out = String::new();

                // The parent is responsible for resetting the style at the
                // start of each row, so begin from the base style and only
                // emit the differences needed to reach the active style.
                while marker_idx < markers.len() && markers[marker_idx].offset <= offset {
                    cur_style = markers[marker_idx].style;
                    marker_idx += 1;
                }
                update_style(&mut out, style, cur_style);
                let mut last_style = cur_style;

                for g in &row.graphemes {
                    // Newline added by wrap_line, not part of the source text
                    if g.data == "\n" {
                        continue;
                    }

                    while marker_idx < markers.len() && markers[marker_idx].offset <= offset {
                        cur_style = markers[marker_idx].style;
                        marker_idx += 1;
                    }
                    if cur_style != last_style {
                        update_style(&mut out, last_style, cur_style);
                        last_style = cur_style;
                    }

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

fn blockquote_to_rows<'a>(
    theme: &'static Theme,
    width: usize,
    style: TextStyle,
    quote: &'a Blockquote,
) -> Box<dyn Iterator<Item = String> + 'a> {
    // One column for the left border prefix
    let inner_width = width.saturating_sub(1);

    let content_style = theme.text_subtle.italicized();

    let mut children = quote
        .children
        .iter()
        .map(move |child| to_rows(theme, inner_width, content_style, child));

    Box::new(iter!(move || {
        for rows in &mut children {
            for row in rows {
                let mut out = String::new();
                update_style(&mut out, style, theme.text_subtle);
                out.push('\u{2595}');
                // The content is italicized, so step the terminal from the
                // non-italic prefix style up to the italicized content style
                // before the child row takes over.
                update_style(&mut out, theme.text_subtle, content_style);
                out.push_str(&row);
                yield out;
            }
        }
    })())
}

fn root_to_rows<'a>(
    theme: &'static Theme,
    width: usize,
    root: &'a Root,
) -> Box<dyn Iterator<Item = String> + 'a> {
    let mut children = root
        .children
        .iter()
        .map(move |child| to_rows(theme, width, theme.text_base, child));

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

fn to_rows<'a>(
    theme: &'static Theme,
    width: usize,
    style: TextStyle,
    node: &'a Node,
) -> Box<dyn Iterator<Item = String> + 'a> {
    match node {
        Node::Root(root) => root_to_rows(theme, width, root),

        Node::FootnoteDefinition(_)
        | Node::MdxJsxFlowElement(_)
        | Node::List(_)
        | Node::MdxjsEsm(_)
        | Node::Toml(_)
        | Node::Yaml(_)
        | Node::Code(_)
        | Node::Math(_)
        | Node::Heading(_)
        | Node::Table(_)
        | Node::ThematicBreak(_)
        | Node::TableRow(_)
        | Node::TableCell(_)
        | Node::ListItem(_)
        | Node::Definition(_)
        | Node::Html(_) => todo!(),

        Node::Paragraph(paragraph) => paragraph_to_rows(theme, width, style, paragraph),
        Node::Blockquote(quote) => blockquote_to_rows(theme, width, style, quote),

        // Phrasing nodes
        | Node::Break(_)
        | Node::InlineCode(_)
        | Node::InlineMath(_)
        | Node::Delete(_)
        | Node::Emphasis(_)
        | Node::MdxTextExpression(_)
        | Node::FootnoteReference(_)
        | Node::Image(_)
        | Node::ImageReference(_)
        | Node::MdxJsxTextElement(_)
        | Node::Link(_)
        | Node::LinkReference(_)
        | Node::Strong(_)
        | Node::Text(_)
        // Unsupported nodes
        | Node::MdxFlowExpression(_)
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
        let node = markdown::to_mdast(text, &markdown::ParseOptions::default()).unwrap();
        super::to_rows(&THEME_DARK, width, THEME_DARK.text_base, &node).collect()
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
            vec!["\x1b[38;2;168;162;158m▕\x1b[3mhello world"],
        );
    }

    #[test]
    fn blockquote_wrap() {
        assert_eq!(
            render("> hello world foo", 8),
            vec![
                "\x1b[38;2;168;162;158m▕\x1b[3mhello ",
                "\x1b[38;2;168;162;158m▕\x1b[3mworld ",
                "\x1b[38;2;168;162;158m▕\x1b[3mfoo",
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
}
