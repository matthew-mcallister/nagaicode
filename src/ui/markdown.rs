// FIXME: need to cap prefix size/set a lower bound on width to prevent
// underflow and broken layout
// FIXME: probably should escape the actual escape char if it appears in source
use markdown::mdast::{Blockquote, Definition, FootnoteDefinition, Heading, InlineCode, List, Node, Text};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::ui::style::{Style, TextStyle, Theme};
use crate::ui::styled_string::{SavePoint, StyledString};
use crate::ui::text::{Row, SPACES, TAB_WIDTH, wrap_line, wrap_line_naive};

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

/// Appends `count` spaces to `out`.
fn push_spaces(out: &mut StyledString, count: usize) {
    let mut remaining = count;
    while remaining > 0 {
        let n = remaining.min(SPACES.len());
        out.push(&SPACES[..n], n);
        remaining -= n;
    }
}

/// Out-of-line style marker.
#[derive(Debug)]
struct Marker {
    /// Byte offset to insert control sequence in plain_text
    offset: usize,
    /// Style to apply
    style: Style,
}

#[derive(Debug)]
struct PhrasingBuilder {
    theme: &'static Theme,
    plain_text: String,
    markers: Vec<Marker>,
    cur_style: Style,
}

impl PhrasingBuilder {
    fn new(theme: &'static Theme, base_style: Style) -> Self {
        Self {
            theme,
            plain_text: String::new(),
            markers: Vec::new(),
            cur_style: base_style,
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

    fn set_text(&mut self, style: TextStyle) {
        let style = self.cur_style.with_text(style);
        self.set_style(style);
    }

    fn set_style(&mut self, style: Style) {
        if style != self.cur_style {
            self.markers.push(Marker {
                offset: self.plain_text.len(),
                style,
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
                self.set_text(self.theme.text_code);
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
                    self.push_text(title);
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
                self.set_text(self.theme.text_math);
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

/// High-level renderer for dealing with flow layout constructs
#[derive(Debug)]
struct FlowBuilder {
    theme: &'static Theme,
    /// Total render width in columns. Every completed row is exactly this
    /// wide. Less-than-full lines must be padded with spaces.
    width: usize,
    /// Per-row prefix, applied at the start of every new row.
    prefix: StyledString,
    /// Current row, always starts with prefix
    row: StyledString,
    /// Completed rows.
    rows: Vec<StyledString>,
    /// Recursion depth
    depth: usize,
    /// Place to begin at next rerender. This is the start of the last flow
    /// item at top level.
    resume_point: ResumePoint,
}

impl FlowBuilder {
    fn new(theme: &'static Theme, width: usize) -> Self {
        Self {
            theme,
            width,
            prefix: StyledString::new(theme.base_style(), 2 * width),
            row: StyledString::new(theme.base_style(), 2 * width),
            rows: Vec::new(),
            depth: 0,
            resume_point: ResumePoint {
                offset: 0,
                row: 0,
            },
        }
    }

    fn apply_prefix(&mut self) {
        let remain = self.width - self.prefix.width();
        self.row = self.prefix.clone_with_capacity(self.prefix.len() + 2 * remain);
    }

    /// Pads the current row to `width` cols and starts a new row
    fn end_row(&mut self) {
        let pad = self.width - self.row.width();
        push_spaces(&mut self.row, pad);
        let remain = self.width - self.prefix.width();
        let row = self.prefix.clone_with_capacity(self.prefix.len() + 2 * remain);
        self.rows.push(std::mem::replace(&mut self.row, row));
    }

    fn save(&mut self) -> SavePoint {
        self.prefix.save()
    }

    fn restore(&mut self, saved: SavePoint) {
        // Only restore before text has been pushed to row
        debug_assert_eq!(self.row.width(), self.prefix.width());
        self.prefix.restore(saved);
        self.apply_prefix();
    }

    fn finish(mut self) -> MarkdownResult {
        if self.row.len() > self.prefix.len() {
            self.end_row();
        }
        MarkdownResult {
            rows: self.rows,
            resume_point: self.resume_point,
        }
    }

    fn remaining_width(&self) -> usize {
        self.width - self.row.width()
    }
}

fn push_rows(fb: &mut FlowBuilder, rows: &[Row]) {
    for row in rows {
        for g in &row.graphemes {
            // Newline added by the wrapping routine, not part of the source
            if g.data == "\n" {
                continue;
            }
            fb.row.push(g.formatted(), g.width as usize);
        }
        fb.end_row();
    }
}

/// Renders phrasing content (the inline children of a paragraph or heading).
/// The returned iterator is owned, so `children` need only live for the
/// duration of this call.
fn push_phrasing(flow: &mut FlowBuilder, children: &[Node]) {
    let mut phrasing = PhrasingBuilder::new(flow.theme, flow.prefix.cur_style());
    phrasing.push_all(children);
    let PhrasingBuilder {
        plain_text,
        markers,
        ..
    } = phrasing;

    let mut cur_style = flow.prefix.cur_style();
    let mut marker_idx = 0usize;
    let mut offset = 0usize;

    for line in plain_text.split('\n') {
        let rows = wrap_line(flow.remaining_width(), line);

        for row in rows {
            while marker_idx < markers.len() && markers[marker_idx].offset <= offset {
                cur_style = markers[marker_idx].style;
                marker_idx += 1;
            }
            // Re-apply current style at start of each row
            flow.row.set_style(cur_style);

            for g in &row.graphemes {
                // Newline added by wrap_line, not part of the source text
                if g.data == "\n" {
                    continue;
                }

                while marker_idx < markers.len() && markers[marker_idx].offset <= offset {
                    cur_style = markers[marker_idx].style;
                    marker_idx += 1;
                }
                flow.row.set_style(cur_style);

                flow.row.push(g.formatted(), g.width as usize);
                offset += g.data.len();
            }

            flow.end_row();
        }

        // Skip the newline separating this line from the next
        offset += 1;
    }
}

fn heading_to_rows(flow: &mut FlowBuilder, heading: &Heading) {
    let mut children = Vec::with_capacity(heading.children.len() + 1);
    children.push(Node::Text(Text {
        value: format!("{} ", "#".repeat(heading.depth as usize)),
        position: None,
    }));

    children.extend(heading.children.iter().cloned());
    let restore_point = flow.save();
    flow.prefix.set_style(flow.prefix.cur_style().bolded());
    flow.row.set_style(flow.row.cur_style().bolded());
    push_phrasing(flow, &children);
    flow.restore(restore_point);
}

fn push_preformatted(flow: &mut FlowBuilder, style: Style, value: &str) {
    let restore = flow.save();
    flow.prefix.set_style(style);
    flow.prefix.freeze_style(true);
    flow.row.set_style(style);
    flow.row.freeze_style(true);

    for line in value.split('\n') {
        let rows = wrap_line_naive(flow.remaining_width(), line);
        push_rows(flow, &rows);
    }

    flow.restore(restore);
}

/// Renders a thematic break as a row of `width` hyphens.
fn thematic_break(flow: &mut FlowBuilder) {
    let count = flow.remaining_width();
    let restore = flow.save();
    flow.row.set_text(flow.theme.text_subtle);
    flow.row.push(&"┄".repeat(count), count);
    flow.end_row();
    flow.restore(restore);
}

fn blockquote(flow: &mut FlowBuilder, quote: &Blockquote) {
    let restore = flow.save();
    let border_style = flow.theme.text_quote;

    flow.prefix.set_text(border_style);
    flow.prefix.push("\u{2590} ", 2);
    flow.prefix.freeze_style(true);

    flow.row.set_text(border_style);
    flow.row.push("\u{2590} ", 2);
    flow.row.freeze_style(true);

    push_flow_children(flow, true, &quote.children);

    flow.restore(restore);
}

fn push_flow_children(flow: &mut FlowBuilder, spread: bool, children: &[Node]) {
    flow.depth += 1;
    for (i, child) in children.iter().enumerate() {
        if i > 0 && spread {
            // Empty line
            flow.end_row();
        }

        if flow.depth <= 1
            && i == children.len() - 1
            && let Some(point) = child.position()
        {
            // Set resume point if at top level
            flow.resume_point = ResumePoint {
                offset: point.start.offset,
                row: flow.rows.len(),
            }
        }

        push_flow_node(flow, child);
    }
    flow.depth -= 1;
}

fn footnote_definition(flow: &mut FlowBuilder, footnote: &FootnoteDefinition) {
    let header = format!("[^{}]:", footnote.identifier);
    let rows = wrap_line_naive(flow.remaining_width(), &header);
    push_rows(flow, &rows);

    let restore = flow.save();
    flow.prefix.push("  ", 2);
    flow.row.push("  ", 2);
    push_flow_children(flow, true, &footnote.children);
    flow.restore(restore);
}

fn definition(flow: &mut FlowBuilder, definition: &Definition) {
    let line = format!("[{}]: {}", definition.identifier, definition.url);
    let rows = wrap_line_naive(flow.remaining_width(), &line);
    push_rows(flow, &rows);
}

/// Renders a list. Unordered lists prefix the first row of each item with
/// "- ", ordered lists with a left-aligned number. Continuation rows are
/// indented to align with the item content. When the list is spread, a blank
/// line separates the children.
fn list(flow: &mut FlowBuilder, list: &List) {
    // The number prefix is padded so all items share a common width
    let prefix_width = if list.ordered {
        3 + (list.children.len().max(1)).ilog10() as usize
    } else {
        2
    };
    let indent = " ".repeat(prefix_width);

    let mut first = true;
    for (i, child) in list.children.iter().enumerate() {
        if !first && list.spread {
            flow.end_row();
        }
        first = false;

        let prefix = if list.ordered {
            format!("{:<width$}", format!("{}.", i + 1), width = prefix_width)
        } else {
            "- ".to_string()
        };

        let restore = flow.save();
        flow.prefix.push(&indent, prefix_width);
        flow.row.push(&prefix, prefix_width);

        match child {
            Node::ListItem(item) => {
                push_flow_children(flow, item.spread, &item.children);
            }
            other => {
                push_flow_node(flow, other);
            }
        }

        flow.restore(restore);
    }
}

fn push_flow_node(flow: &mut FlowBuilder, node: &Node) {
    match node {
        Node::Root(root) => push_flow_children(flow, true, &root.children),

        Node::Table(_)
        | Node::TableRow(_)
        | Node::TableCell(_) => todo!(),

        Node::Paragraph(paragraph) => push_phrasing(flow, &paragraph.children),
        Node::Blockquote(quote) => blockquote(flow, quote),
        Node::Code(code) => push_preformatted(flow, Style::new(flow.theme.text_code, flow.theme.bg_base), &code.value),
        Node::Math(math) => push_preformatted(flow, Style::new(flow.theme.text_math, flow.theme.bg_base), &math.value),
        Node::FootnoteDefinition(footnote) => footnote_definition(flow, footnote),
        Node::Definition(def) => definition(flow, def),
        Node::Html(html) => push_preformatted(flow, Style::new(flow.theme.text_code, flow.theme.bg_base), &html.value),
        Node::Heading(heading) => heading_to_rows(flow, heading),
        Node::ThematicBreak(_) => thematic_break(flow),
        Node::ListItem(list_item) => {
            push_flow_children(flow, list_item.spread, &list_item.children)
        }
        Node::List(l) => list(flow, l),

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
        => unreachable!("broken markdown AST"),
    }
}

/// Place to begin at next Markdown rerender
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResumePoint {
    /// Byte offset in source
    pub offset: usize,
    /// Row in rendered output
    pub row: usize,
}

/// Output of Markdown rendering runction
#[derive(Debug)]
pub struct MarkdownResult {
    /// Rendered rows
    pub rows: Vec<StyledString>,
    /// Place to begin next rerender
    pub resume_point: ResumePoint,
}

/// Converts a markdown document into preformatted lines ready to be printed
/// to stdout. Every row is exactly `width` columns wide.
pub fn render_markdown(
    theme: &'static Theme,
    width: usize,
    text: &str,
) -> MarkdownResult {
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

    render_mdast(theme, width, &node)
}

pub fn render_mdast(
    theme: &'static Theme,
    width: usize,
    node: &Node,
) -> MarkdownResult {
    let mut flow = FlowBuilder::new(theme, width);

    flow.apply_prefix();
    push_flow_node(&mut flow, node);

    flow.finish()
}

#[cfg(test)]
mod test_paragraph {
    use crate::ui::canvas::render_canvas;
    use crate::ui::style::{Style, THEME_DARK, UpdateStyle};
    use crate::ui::style::testing::{ResetBold, ResetItalic, SetBold, SetItalic};

    fn render(text: &str, width: usize) -> String {
        let mut result = super::render_markdown(&THEME_DARK, width, text);
        render_canvas(&mut result.rows[..])
    }

    #[test]
    fn plain() {
        let base = THEME_DARK.base_style();
        assert_eq!(render("hello world", 20), format!("{base}hello world         "));
    }

    #[test]
    fn bold() {
        let base = THEME_DARK.base_style();
        let bold = SetBold;
        assert_eq!(render("**hi**", 20), format!("{base}{bold}hi                  "));
    }

    #[test]
    fn inline_spacing() {
        let base = THEME_DARK.base_style();
        let italic = SetItalic;
        let reset_italic = ResetItalic;
        assert_eq!(render("a*b*c", 20), format!("{base}a{italic}b{reset_italic}c                 "));
        assert_eq!(render("a *h*c", 20), format!("{base}a {italic}h{reset_italic}c                "));
        assert_eq!(render("a*h* c", 20), format!("{base}a{italic}h{reset_italic} c                "));
        assert_eq!(render("a *h* c", 20), format!("{base}a {italic}h{reset_italic} c               "));
    }

    #[test]
    fn wrap() {
        let base = THEME_DARK.base_style();
        assert_eq!(
            render("hello world foo", 8),
            format!("{base}hello   \n{base}world   \n{base}foo     "),
        );
    }

    #[test]
    fn styles() {
        let base = THEME_DARK.base_style();
        let code = Style::new(THEME_DARK.text_code, THEME_DARK.bg_base);
        let bold = SetBold;
        let reset_bold = ResetBold;
        let italic = SetItalic;
        assert_eq!(
            render("**bold** *italic*\\\n`code`", 20),
            format!("{base}{bold}bold{reset_bold} {italic}italic         \n{code}code                "),
        );
    }

    #[test]
    fn blockquote() {
        let quote = Style::new(THEME_DARK.text_quote, THEME_DARK.bg_base);

        assert_eq!(
            render("> hello *world*", 20),
            format!("{quote}▐ hello world       "),
        );
        assert_eq!(
            render("> **bold** `code`", 20),
            format!("{quote}▐ bold code         "),
        );
        assert_eq!(
            render("> hello world foo", 8),
            format!("{quote}▐ hello \n{quote}▐ world \n{quote}▐ foo   "),
        );
        assert_eq!(
            render("> first\n>\n> second", 16),
            format!("{quote}▐ first         \n{quote}▐               \n{quote}▐ second        "),
        );
    }

    #[test]
    fn two_paragraphs() {
        let base = THEME_DARK.base_style();
        assert_eq!(
            render("first paragraph\n\nsecond paragraph", 20),
            format!("{base}first paragraph     \n{base}                    \n{base}second paragraph    "),
        );
    }

    #[test]
    fn code() {
        let code = Style::new(THEME_DARK.text_code, THEME_DARK.bg_base);
        assert_eq!(
            render("```\nfn main() {}\n```", 20),
            format!("{code}fn main() {{}}        "),
        );
    }

    #[test]
    fn code_wrap() {
        let code = Style::new(THEME_DARK.text_code, THEME_DARK.bg_base);
        assert_eq!(
            render("```\nabcdefgh\n```", 4),
            format!("{code}abcd\n{code}efgh"),
        );
    }

    #[test]
    fn math() {
        let math = Style::new(THEME_DARK.text_math, THEME_DARK.bg_base);
        assert_eq!(
            render("$$\nx^2 + y^2 = z^2\n$$", 20),
            format!("{math}x^2 + y^2 = z^2     "),
        );
    }

    #[test]
    fn inline_math() {
        let base = THEME_DARK.base_style();
        let math = Style::new(THEME_DARK.text_math, THEME_DARK.bg_base);
        let to_math = UpdateStyle(base, math);
        let from_math = UpdateStyle(math, base);
        assert_eq!(
            render("a $x^2$ b", 20),
            format!("{base}a {to_math}x^2{from_math} b             "),
        );
    }

    #[test]
    fn footnote_definition() {
        let base = THEME_DARK.base_style();
        assert_eq!(
            render("text[^1]\n\n[^1]: the note", 20),
            format!("{base}text[^1]            \n{base}                    \n{base}[^1]:               \n{base}  the note          "),
        );
    }

    #[test]
    fn footnote_definition_wrap() {
        let base = THEME_DARK.base_style();
        assert_eq!(
            render("[^1]: aaaabbbb", 8),
            format!("{base}[^1]:   \n{base}  aaaabb\n{base}  bb    "),
        );
    }

    #[test]
    fn definition() {
        let base = THEME_DARK.base_style();
        assert_eq!(
            render("[foo]: https://example.com", 28),
            format!("{base}[foo]: https://example.com  "),
        );
    }

    #[test]
    fn definition_wrap() {
        let base = THEME_DARK.base_style();
        assert_eq!(
            render("[foo]: https://example.com", 16),
            format!("{base}[foo]: https://e\n{base}xample.com      "),
        );
    }

    #[test]
    fn html() {
        let code = Style::new(THEME_DARK.text_code, THEME_DARK.bg_base);
        assert_eq!(
            render("<div>\n<p>hi</p>\n</div>", 20),
            format!("{code}<div>               \n{code}<p>hi</p>           \n{code}</div>              "),
        );
    }

    #[test]
    fn heading() {
        let header = THEME_DARK.base_style().bolded();
        let italic = SetItalic;
        assert_eq!(render("# Hello", 20), format!("{header}# Hello             "));
        assert_eq!(render("#### Deep", 20), format!("{header}#### Deep           "));
        assert_eq!(
            render("## Hello *world*", 20),
            format!("{header}## Hello {italic}world      "),
        );
    }

    #[test]
    fn thematic_break() {
        let subtle = Style::new(THEME_DARK.text_subtle, THEME_DARK.bg_base);
        assert_eq!(
            render("---", 10),
            format!("{subtle}┄┄┄┄┄┄┄┄┄┄"),
        );
    }

    #[test]
    fn unordered_list() {
        let base = THEME_DARK.base_style();
        let italic = SetItalic;
        assert_eq!(render("- a", 16), format!("{base}- a             "));
        assert_eq!(render("- a\n- b", 16), format!("{base}- a             \n{base}- b             "));
        assert_eq!(
            render("- a\n\n- b", 16),
            format!("{base}- a             \n{base}                \n{base}- b             "),
        );
        assert_eq!(render("- *hi*", 16), format!("{base}- {italic}hi            "));
        assert_eq!(render("- hello world foo", 8), format!("{base}- hello \n{base}  world \n{base}  foo   "));
    }

    #[test]
    fn ordered_list() {
        let base = THEME_DARK.base_style();
        assert_eq!(render("1. a\n2. b", 16), format!("{base}1. a            \n{base}2. b            "));
        let src = "\
            1. item\n\
            1. item\n\
            1. item\n\
            1. item\
        ";
        assert_eq!(
            render(src, 12),
            format!("\
{base}1. item     \n\
{base}2. item     \n\
{base}3. item     \n\
{base}4. item     "),
        );
        assert_eq!(
            render("1. hello world foo", 8),
            format!("{base}1. hello\n{base}        \n{base}   world\n{base}    foo "),
        );
        assert_eq!(render("1. a\n1. b", 16), format!("{base}1. a            \n{base}2. b            "));
    }

    #[test]
    fn nested_lists() {
        let base = THEME_DARK.base_style();
        assert_eq!(
            render("- one\n  - two\n  - three", 12),
            format!("{base}- one       \n{base}  - two     \n{base}  - three   "),
        );
        assert_eq!(
            render("- one\n  1. two\n  2. three", 12),
            format!("{base}- one       \n{base}  1. two    \n{base}  2. three  "),
        );
        assert_eq!(
            render("1. one\n    - two\n    - three", 12),
            format!("{base}1. one      \n{base}   - two    \n{base}   - three  "),
        );
        assert_eq!(
            render("1. one\n    1. two\n    2. three", 12),
            format!("{base}1. one      \n{base}   1. two   \n{base}   2. three "),
        );
    }

    #[test]
    fn resume_point() {
        let res = super::render_markdown(&THEME_DARK, 20, "");
        assert_eq!(res.resume_point, super::ResumePoint { offset: 0, row: 0 });

        let res = super::render_markdown(&THEME_DARK, 20, "hello world");
        assert_eq!(res.resume_point, super::ResumePoint { offset: 0, row: 0 });

        let text = "first paragraph\n\nsecond paragraph";
        let res = super::render_markdown(&THEME_DARK, 20, text);
        assert_eq!(
            res.resume_point,
            super::ResumePoint {
                offset: 17,
                row: 2,
            }
        );

        let text = "hello world foo\n\nbar";
        let res = super::render_markdown(&THEME_DARK, 8, text);
        assert_eq!(
            res.resume_point,
            super::ResumePoint {
                offset: 17,
                row: 4,
            }
        );

        // Children inside a blockquote should not overwrite the top-level resume point
        let text = "> first\n>\n> second";
        let res = super::render_markdown(&THEME_DARK, 16, text);
        assert_eq!(
            res.resume_point,
            super::ResumePoint {
                offset: 0,
                row: 0,
            }
        );

        let text2 = "> quote\n\nparagraph";
        let res2 = super::render_markdown(&THEME_DARK, 20, text2);
        assert_eq!(
            res2.resume_point,
            super::ResumePoint {
                offset: 9,
                row: 2,
            }
        );

        let text = "# Title\n\n```\nlet x = 1;\n```\n\n- item 1\n- item 2";
        let res = super::render_markdown(&THEME_DARK, 20, text);
        let list_offset = text.find("- item 1").unwrap();
        assert_eq!(
            res.resume_point,
            super::ResumePoint {
                offset: list_offset,
                row: 4,
            }
        );
    }
}
