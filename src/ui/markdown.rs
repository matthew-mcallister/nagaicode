use std::fmt;
use std::iter::iter;

use crossterm::style::{
    Attribute, Attributes, Color, Colors, ContentStyle, ResetColor, SetAttribute,
    SetAttributes, SetBackgroundColor, SetColors, SetForegroundColor, SetStyle,
    SetUnderlineColor,
};
use crossterm::Command;
use markdown::mdast::{InlineCode, Node, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::ui::style::{TextStyle, Theme};
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

/// Replaces runs of whitespace with a single space character.
///
/// Technically we should not eliminate whitespace that is combined with a
/// diacritic, e.g. " \u{0301}", but no one renders this correctly, making
/// usage so rare that it is not worth supporting. Poor design decision by the
/// Unicode Consortium.
fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for t in s.split_whitespace() {
        out.push_str(t);
        out.push(' ');
    }
    out
}

impl MarkupBuilder {
    fn new(theme: &'static Theme) -> Self {
        Self {
            theme,
            plain_text: String::new(),
            markers: Vec::new(),
            cur_style: theme.text_base,
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

fn write_style(out: &mut String, style: TextStyle) {
    let _ = SetStyle(ContentStyle::from(style)).write_ansi(out);
}

fn paragraph_to_rows<'a>(
    theme: &'static Theme,
    width: usize,
    paragraph: &'a Paragraph,
) -> Box<dyn Iterator<Item = String> + 'a> {
    let mut builder = MarkupBuilder::new(theme);
    builder.push_all(&paragraph.children);
    let MarkupBuilder {
        plain_text,
        markers,
        ..
    } = builder;

    Box::new(iter!(move || {
        let mut cur_style = theme.text_base;
        let mut marker_idx = 0usize;
        let mut offset = 0usize;

        for line in plain_text.split('\n') {
            let rows = wrap_line(width, line);

            for row in rows {
                let mut out = String::new();

                // Update and reapply styles at start of row
                while marker_idx < markers.len() && markers[marker_idx].offset <= offset {
                    cur_style = markers[marker_idx].style;
                    marker_idx += 1;
                }
                write_style(&mut out, cur_style);
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
                        write_style(&mut out, cur_style);
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

/// Renders formatted lines from a flow (multiline) node. Panics on invalid
/// nodes.
fn to_rows<'a>(
    theme: &'static Theme,
    width: usize,
    node: &'a Node,
) -> Box<dyn Iterator<Item = String> + 'a> {
    match node {
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
        | Node::Heading(_)
        | Node::Table(_)
        | Node::ThematicBreak(_)
        | Node::TableRow(_)
        | Node::TableCell(_)
        | Node::ListItem(_)
        | Node::Definition(_)
        | Node::Html(_) => todo!(),

        Node::Paragraph(paragraph) => paragraph_to_rows(theme, width, paragraph),

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
    use super::paragraph_to_rows;
    use crate::ui::style::THEME_DARK;
    use markdown::mdast::Node;

    fn render(text: &str, width: usize) -> Vec<String> {
        let node = markdown::to_mdast(text, &markdown::ParseOptions::default()).unwrap();
        let Node::Root(root) = node else { panic!() };
        let Node::Paragraph(p) = &root.children[0] else { panic!() };
        paragraph_to_rows(&THEME_DARK, width, p).collect()
    }

    #[test]
    fn plain() {
        assert_eq!(render("hello world", 80), vec!["\x1b[38;5;15mhello world "]);
    }

    #[test]
    fn bold() {
        assert_eq!(render("**hi**", 80), vec!["\x1b[38;5;15m\x1b[1mhi "]);
    }

    #[test]
    fn wrap() {
        assert_eq!(render("hello world foo", 8), vec![
            "\x1b[38;5;15mhello ",
            "\x1b[38;5;15mworld ",
            "\x1b[38;5;15mfoo ",
        ]);
    }

    #[test]
    fn styles() {
        assert_eq!(
            render("**bold** *italic*\\\n`code`", 80),
            vec![
                "\x1b[38;5;15m\x1b[1mbold \x1b[38;5;15m\x1b[3mitalic ",
                "\x1b[38;2;254;240;138mcode",
            ],
        );
    }
}
