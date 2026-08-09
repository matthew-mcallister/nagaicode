use std::fmt;

use crossterm::style::{
    Attribute, Attributes, Color, Colors, ContentStyle, ResetColor, SetAttribute,
    SetAttributes, SetBackgroundColor, SetColors, SetForegroundColor, SetStyle,
    SetUnderlineColor,
};
use crossterm::Command;
use markdown::mdast::Node;

use crate::ui::style::{TextStyle, Theme};

/// Enum representation of a terminal control sequence (e.g. set style, set
/// color).
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ConsoleCommand {
    /// Sets the foreground color.
    SetForegroundColor(Color),
    /// Sets the background color.
    SetBackgroundColor(Color),
    /// Sets the underline color.
    SetUnderlineColor(Color),
    /// Sets a single attribute.
    SetAttribute(Attribute),
    /// Sets multiple attributes at once.
    SetAttributes(Attributes),
    /// Sets both the foreground and background colors.
    SetColors(Colors),
    /// Sets a full content style.
    SetStyle(ContentStyle),
    /// Resets all styling.
    ResetColor,
    /// Replaces the terminal text style, transitioning from one style to
    /// another.
    UpdateStyle(TextStyle, TextStyle),
}

impl Command for ConsoleCommand {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        match self {
            Self::SetForegroundColor(color) => SetForegroundColor(*color).write_ansi(f),
            Self::SetBackgroundColor(color) => SetBackgroundColor(*color).write_ansi(f),
            Self::SetUnderlineColor(color) => SetUnderlineColor(*color).write_ansi(f),
            Self::SetAttribute(attribute) => SetAttribute(*attribute).write_ansi(f),
            Self::SetAttributes(attributes) => SetAttributes(*attributes).write_ansi(f),
            Self::SetColors(colors) => SetColors(*colors).write_ansi(f),
            Self::SetStyle(style) => SetStyle(*style).write_ansi(f),
            Self::ResetColor => ResetColor.write_ansi(f),
            Self::UpdateStyle(old, new) => {
                crate::ui::style::UpdateStyle(*old, *new).write_ansi(f)
            }
        }
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> std::io::Result<()> {
        match self {
            Self::SetForegroundColor(color) => SetForegroundColor(*color).execute_winapi(),
            Self::SetBackgroundColor(color) => SetBackgroundColor(*color).execute_winapi(),
            Self::SetUnderlineColor(color) => SetUnderlineColor(*color).execute_winapi(),
            Self::SetAttribute(attribute) => SetAttribute(*attribute).execute_winapi(),
            Self::SetAttributes(attributes) => SetAttributes(*attributes).execute_winapi(),
            Self::SetColors(colors) => SetColors(*colors).execute_winapi(),
            Self::SetStyle(style) => SetStyle(*style).execute_winapi(),
            Self::ResetColor => ResetColor.execute_winapi(),
            Self::UpdateStyle(old, new) => {
                crate::ui::style::UpdateStyle(*old, *new).execute_winapi()
            }
        }
    }
}

/// Out-of-line instruction to emit a control sequence.
#[derive(Debug)]
struct Control {
    /// Byte offset to insert control sequence in plain_text
    offset: usize,
    /// Command to emit
    command: ConsoleCommand,
}

/// Marked up paragraph with out-of-line control sequences.
#[derive(Debug)]
struct Markup {
    /// Single line of plain text with collapsed whitespace.
    plain_text: String,
    /// Array of control sequences
    control: Vec<Control>,
}

#[derive(Debug)]
struct MarkupBuilder {
    theme: &'static Theme,
    plain_text: String,
    control: Vec<Control>,
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
            control: Vec::new(),
            cur_style: theme.text_base,
        }
    }

    /// Appends collapsed text in the current style.
    fn push_text(&mut self, s: &str) {
        self.plain_text.push_str(&collapse_whitespace(s));
    }

    /// Transitions to a new style, emitting a control sequence if it differs
    /// from the current style.
    fn set_style(&mut self, style: TextStyle) {
        if style != self.cur_style {
            self.control.push(Control {
                offset: self.plain_text.len(),
                command: ConsoleCommand::UpdateStyle(self.cur_style, style),
            });
            self.cur_style = style;
        }
    }

    /// Runs `f` in the given style, then restores the previous style.
    fn run_with_style(&mut self, style: TextStyle, f: impl FnOnce(&mut Self)) {
        let prev = self.cur_style;
        self.set_style(style);
        f(self);
        self.set_style(prev);
    }

    fn push_children(&mut self, children: &[Node]) {
        for child in children {
            self.push_node(child);
        }
    }

    /// Emits the raw text representation of a node we cannot style.
    fn push_fallback(&mut self, node: &Node) {
        let text = match node {
            Node::Image(image) => format!("![{}]", image.alt),
            Node::ImageReference(image) => format!("![{}]", image.alt),
            Node::InlineMath(math) => format!("${}$", math.value),
            Node::FootnoteReference(footnote) => {
                format!("[^{}]", footnote.identifier)
            }
            other => other.to_string(),
        };
        self.push_text(&text);
    }

    fn push_node(&mut self, node: &Node) {
        match node {
            Node::Text(text) => self.push_text(&text.value),
            Node::Break(_) => self.push_text(" "),

            Node::InlineCode(code) => {
                self.run_with_style(self.theme.text_code, |b| b.push_text(&code.value));
            }
            Node::Emphasis(emphasis) => {
                let style = italic(self.theme.text_base);
                self.run_with_style(style, |b| b.push_children(&emphasis.children));
            }
            Node::Strong(strong) => {
                let style = bold(self.theme.text_base);
                self.run_with_style(style, |b| b.push_children(&strong.children));
            }
            Node::Link(link) => {
                let style = underline(self.theme.text_base);
                self.run_with_style(style, |b| b.push_children(&link.children));
                self.run_with_style(self.theme.text_subtle, |b| {
                    b.push_text(&format!(" ({})", link.url));
                });
            }
            Node::LinkReference(link) => {
                self.run_with_style(self.theme.text_subtle, |b| {
                    b.push_children(&link.children);
                });
            }

            // Syntax we cannot faithfully reproduce in the terminal falls back
            // to its raw text representation.
            Node::InlineMath(_)
            | Node::MdxTextExpression(_)
            | Node::Html(_)
            | Node::Delete(_)
            | Node::Image(_)
            | Node::ImageReference(_)
            | Node::FootnoteReference(_)
            | Node::MdxJsxTextElement(_) => self.push_fallback(node),

            // Flow and container nodes should never appear in phrasing
            // position.
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
            | Node::Paragraph(_) => unreachable!("flow node in phrasing context"),
        }
    }

    fn finish(self) -> Markup {
        Markup {
            plain_text: self.plain_text,
            control: self.control,
        }
    }
}

/// Returns `style` with bold enabled.
fn bold(mut style: TextStyle) -> TextStyle {
    style.bold = true;
    style
}

/// Returns `style` with italic enabled.
fn italic(mut style: TextStyle) -> TextStyle {
    style.italic = true;
    style
}

/// Returns `style` with underline enabled.
fn underline(mut style: TextStyle) -> TextStyle {
    style.underline = true;
    style
}

/// Converts phrasing nodes (inline) into markup. Panics on invalid nodes.
///
/// Not all syntax can be properly displayed in the terminal. In these cases,
/// as a fallback, we emit a text representation of the original markdown.
fn to_markup(theme: &'static Theme, node: &Node) -> Markup {
    let mut builder = MarkupBuilder::new(theme);
    builder.push_node(node);
    builder.finish()
}

/// Renders formatted lines from a flow (multiline) node. Panics on invalid
/// nodes.
fn to_rows(
    theme: &'static Theme,
    width: usize,
    node: &Node,
) -> Box<dyn Iterator<Item = String>> {
    match node {
        Node::Root(root) => todo!(),
        Node::Blockquote(blockquote) => todo!(),
        Node::FootnoteDefinition(footnote_definition) => todo!(),
        Node::MdxJsxFlowElement(mdx_jsx_flow_element) => todo!(),
        Node::List(list) => todo!(),
        Node::MdxjsEsm(mdxjs_esm) => todo!(),
        Node::Toml(toml) => todo!(),
        Node::Yaml(yaml) => todo!(),
        Node::Break(_) => todo!(),
        Node::InlineCode(inline_code) => todo!(),
        Node::InlineMath(inline_math) => todo!(),
        Node::Delete(delete) => todo!(),
        Node::Emphasis(emphasis) => todo!(),
        Node::MdxTextExpression(mdx_text_expression) => todo!(),
        Node::FootnoteReference(footnote_reference) => todo!(),
        Node::Html(html) => todo!(),
        Node::Image(image) => todo!(),
        Node::ImageReference(image_reference) => todo!(),
        Node::MdxJsxTextElement(mdx_jsx_text_element) => todo!(),
        Node::Link(link) => todo!(),
        Node::LinkReference(link_reference) => todo!(),
        Node::Strong(strong) => todo!(),
        Node::Text(text) => todo!(),
        Node::Code(code) => todo!(),
        Node::Math(math) => todo!(),
        Node::MdxFlowExpression(mdx_flow_expression) => todo!(),
        Node::Heading(heading) => todo!(),
        Node::Table(table) => todo!(),
        Node::ThematicBreak(thematic_break) => todo!(),
        Node::TableRow(table_row) => todo!(),
        Node::TableCell(table_cell) => todo!(),
        Node::ListItem(list_item) => todo!(),
        Node::Definition(definition) => todo!(),
        Node::Paragraph(paragraph) => todo!(),
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