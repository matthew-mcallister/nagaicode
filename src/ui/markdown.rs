// FIXME: need to cap prefix size/set a lower bound on width to prevent
// underflow and broken layout
// FIXME: probably should escape the actual escape char if it appears in source
use markdown::mdast::{Blockquote, Definition, FootnoteDefinition, Heading, InlineCode, List, Node, Table, Text};
use serde_json::json;

use crate::query::{DataQuery, QueryError, QueryField};
use crate::ui::markup_string::MarkupString;
use crate::ui::style::{Style, Theme};
use crate::ui::styled_string::{SavePoint, StyledString};
use crate::ui::text::{Row, SPACES, TAB_WIDTH, wrap_line_naive};

/// Replaces runs of whitespace with a single space character. Preserves
/// leading and trailing whitespace.
///
/// Technically we should not eliminate whitespace that is combined with a
/// diacritic, e.g. " \u{0301}", but no one renders this correctly, making
/// usage so rare that it is not worth supporting. Poor design decision by the
/// Unicode Consortium.
fn collapse_whitespace(out: &mut MarkupString, s: &str) {
    if s.starts_with(char::is_whitespace) {
        out.push_char(' ');
    }

    let mut words = s.split_whitespace();
    let Some(t) = words.next() else { return };
    out.push_str(t);

    for t in words {
        out.push_char(' ');
        out.push_str(t);
    }

    if s.ends_with(char::is_whitespace) {
        out.push_char(' ');
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

#[derive(Debug)]
struct PhrasingBuilder<'a> {
    theme: &'a Theme,
    base_style: Style,
    rows: Vec<MarkupString>,
    cur: MarkupString,
}

impl<'a> PhrasingBuilder<'a> {
    fn new(theme: &'a Theme, base_style: Style) -> Self {
        Self {
            theme,
            base_style,
            rows: Vec::new(),
            cur: MarkupString::new(),
        }
    }

    fn rows(&self) -> impl Iterator<Item = &MarkupString> {
        self.rows.iter().chain(std::iter::once(&self.cur))
    }

    fn width(&self) -> usize {
        self.rows().map(MarkupString::width).max().unwrap_or(0)
    }

    fn push_text(&mut self, s: &str) {
        collapse_whitespace(&mut self.cur, s);
    }

    /// Removes newlines, expands tabs, but otherwise preserves whitespace.
    fn push_inline_code(&mut self, s: &str) {
        for c in s.chars() {
            match c {
                '\n' | '\r' => self.cur.push_char(' '),
                '\t' => {
                    let n = TAB_WIDTH - self.cur.width() % TAB_WIDTH;
                    self.cur.push_str(&SPACES[..n]);
                }
                _ => self.cur.push_char(c),
            }
        }
    }

    fn push_all(&mut self, nodes: &[Node]) {
        for node in nodes {
            self.push_node(node);
        }
    }

    fn push_break(&mut self) {
        let state = self.cur.state();
        let mut row = MarkupString::new();
        if state.to_style(self.base_style) != self.base_style {
            row.set_state(state);
        }
        let cur = std::mem::replace(&mut self.cur, row);
        self.rows.push(cur);
    }

    fn push_node(&mut self, node: &Node) {
        match node {
            // Plain text
            Node::Text(text) => self.push_text(&text.value),
            Node::Break(_) => self.push_break(),

            // Styling
            Node::InlineCode(inner) => {
                let prev = self.cur.state();
                self.cur.set_text(self.theme.text_code);
                self.push_inline_code(&inner.value);
                self.cur.set_state(prev);
            }
            Node::Emphasis(inner) => {
                let prev = self.cur.state();
                self.cur.set_state(prev.with_italic());
                self.push_all(&inner.children);
                self.cur.set_state(prev);
            }
            Node::Strong(inner) => {
                let prev = self.cur.state();
                self.cur.set_state(prev.with_bold());
                self.push_all(&inner.children);
                self.cur.set_state(prev);
            }
            Node::Delete(inner) => {
                let prev = self.cur.state();
                self.cur.set_state(prev.with_strikethrough());
                self.push_all(&inner.children);
                self.cur.set_state(prev);
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
                let prev = self.cur.state();
                self.cur.set_text(self.theme.text_math);
                self.push_inline_code(&math.value);
                self.cur.set_state(prev);
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
struct FlowBuilder<'a> {
    theme: &'a Theme,
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

impl<'a> FlowBuilder<'a> {
    fn new(theme: &'a Theme, width: usize) -> Self {
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
        let mut completed = std::mem::replace(&mut self.row, row);
        completed.freeze_style(false);
        self.rows.push(completed);
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

fn push_rows(fb: &mut FlowBuilder<'_>, rows: &[Row]) {
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
fn push_phrasing(flow: &mut FlowBuilder<'_>, children: &[Node]) {
    let mut phrasing = PhrasingBuilder::new(flow.theme, flow.prefix.cur_style());
    phrasing.push_all(children);
    push_phrasing_inner(flow, &phrasing);
}

fn push_phrasing_inner(flow: &mut FlowBuilder<'_>, phrasing: &PhrasingBuilder<'_>) {
    let width = flow.remaining_width();
    for row in phrasing.rows() {
        for wrapped in row.wrap(width) {
            flow.row.push_markup(&wrapped);
            flow.end_row();
        }
    }
}

fn heading_to_rows(flow: &mut FlowBuilder<'_>, heading: &Heading) {
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

fn push_preformatted(flow: &mut FlowBuilder<'_>, style: Style, value: &str) {
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
fn thematic_break(flow: &mut FlowBuilder<'_>) {
    let count = flow.remaining_width();
    let restore = flow.save();
    flow.row.set_text(flow.theme.text_subtle);
    flow.row.push(&"┄".repeat(count), count);
    flow.end_row();
    flow.restore(restore);
}

fn blockquote(flow: &mut FlowBuilder<'_>, quote: &Blockquote) {
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

fn push_flow_children(flow: &mut FlowBuilder<'_>, spread: bool, children: &[Node]) {
    flow.depth += 1;
    let mut prev: Option<(&Node, ResumePoint)> = None;
    for (i, child) in children.iter().enumerate() {
        if i > 0 && spread {
            // Empty line
            flow.end_row();
        }

        let resume = child.position().map(|point| ResumePoint {
            offset: point.start.offset,
            row: flow.rows.len(),
        });

        if flow.depth <= 1 && i == children.len() - 1 {
            // Set resume point if at top level
            flow.resume_point = match (child, prev) {
                (Node::Paragraph(_), Some((Node::List(list), prev))) if list.ordered => prev,
                _ => resume.unwrap_or(flow.resume_point),
            };
        }

        if let Some(point) = resume {
            prev = Some((child, point));
        }

        push_flow_node(flow, child);
    }
    flow.depth -= 1;
}

fn footnote_definition(flow: &mut FlowBuilder<'_>, footnote: &FootnoteDefinition) {
    let header = format!("[^{}]:", footnote.identifier);
    let rows = wrap_line_naive(flow.remaining_width(), &header);
    push_rows(flow, &rows);

    let restore = flow.save();
    flow.prefix.push("  ", 2);
    flow.row.push("  ", 2);
    push_flow_children(flow, true, &footnote.children);
    flow.restore(restore);
}

fn definition(flow: &mut FlowBuilder<'_>, definition: &Definition) {
    let line = format!("[{}]: {}", definition.identifier, definition.url);
    let rows = wrap_line_naive(flow.remaining_width(), &line);
    push_rows(flow, &rows);
}

fn list(flow: &mut FlowBuilder<'_>, list: &List) {
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
                if item.children.is_empty() {
                    push_phrasing(flow, &[Node::Text(Text {
                        value: String::new(),
                        position: None,
                    })]);
                } else {
                    push_flow_children(flow, item.spread, &item.children);
                }
            }
            other => {
                push_flow_node(flow, other);
            }
        }

        flow.restore(restore);
    }
}

// Minimum width of any column that is shrunk to help fit the table
const OVERSIZED_COL_MIN_WIDTH: usize = 20;
const COL_MAX_WIDTH: usize = 60;

// Calculates width of each column in a table
fn column_width(content_width: usize, widths: &mut [usize]) {
    let wc = content_width;
    for w in widths.iter_mut() {
        *w = (*w).min(COL_MAX_WIDTH);
    }
    let total: usize = widths.iter().copied().sum();
    if total <= wc { return; }

    let excess: usize = widths.iter()
        .map(|w| w.saturating_sub(OVERSIZED_COL_MIN_WIDTH))
        .sum();
    let (new_max, mut extra) = if total - wc >= excess {
        // Table is too oversized, make all columns as small as possible
        (OVERSIZED_COL_MIN_WIDTH, 0)
    } else {
        // Greedily shrink the widest columns until we fit
        let mut sorted = widths.to_vec();
        sorted.sort_by(|a, b| b.cmp(a));
        let mut level = sorted[0];
        let mut run = 0;
        let extra;
        let mut i = 0;
        let mut avail = total - wc;
        loop {
            while sorted.get(i) == Some(&level) {
                i += 1;
                run += 1;
            }
            let next_level = sorted.get(i).copied().unwrap_or(OVERSIZED_COL_MIN_WIDTH);
            let delta = level - next_level;
            let (rows, rem) = (avail / run, avail % run);
            if rows >= delta {
                avail -= delta * run;
                level = next_level;
                continue;
            }
            let carry = (rem > 0) as usize;
            level -= rows + carry;
            extra = run - rem;
            break;
        }
        (level, extra)
    };

    for w in widths.iter_mut() {
        let carry = extra.min(1);
        if *w > new_max {
            *w = new_max + carry;
            extra -= carry;
        }
    }
}

#[derive(Debug)]
struct TableBuilder {
    // input
    col_widths: Vec<usize>,
    // state
    row_num: usize,
    rendered: Vec<MarkupString>,
    canvas_row: usize,
    canvas_col: usize,
}

impl TableBuilder {
    fn new(col_widths: Vec<usize>) -> Self {
        Self {
            col_widths,
            row_num: Default::default(),
            rendered: Default::default(),
            canvas_row: Default::default(),
            canvas_col: Default::default(),
        }
    }

    fn width(&self) -> usize {
        4 + self.col_widths.iter().sum::<usize>()
            + 3 * self.col_widths.len().saturating_sub(1)
    }

    // Wrap content to column width, compute row height, push rows w/ leading
    // and trailing horizontal padding.
    fn push_row<'t>(&mut self, cells: &[PhrasingBuilder<'t>]) {
        let cell_rows: Vec<Vec<MarkupString>> = cells
            .iter()
            .zip(&self.col_widths)
            .map(|(cell, &width)| {
                cell.rows()
                    .flat_map(|row| row.wrap(width))
                    .collect()
            })
            .collect();
        let height = cell_rows.iter().map(Vec::len).max().unwrap_or(1);

        for line in 0..height {
            let width = self.width();
            self.canvas_row = self.rendered.len();
            self.rendered.push(MarkupString::with_capacity(width, 2 * self.col_widths.len()));
            self.canvas_col = 0;

            self.push_vertical();

            for (col, rows) in cell_rows.iter().enumerate() {
                self.rendered[self.canvas_row].push_char(' ');
                self.canvas_col += 1;

                if let Some(row) = rows.get(line) {
                    self.rendered[self.canvas_row].push(row);
                }
                self.canvas_col += self.col_widths[col] + 1;

                self.push_vertical();
            }
        }

        self.row_num += 1;
        self.push_horizontal();
    }

    // Push vertical line (|\n|\n|\n...)
    fn push_vertical(&mut self) {
        let row = &mut self.rendered[self.canvas_row];
        row.pad_to_width(self.canvas_col);
        row.push_char('|');
        self.canvas_col += 1;
    }

    // Push "+---+---+..." (or "+===+===+..." if row_num == 1)
    fn push_horizontal(&mut self) {
        let width = self.width();
        let fill = if self.row_num == 1 { "=" } else { "-" };
        let mut row = MarkupString::with_capacity(width, 0);
        row.push_char('+');
        for &col_width in &self.col_widths {
            let n = col_width + 2;
            row.push_str(&fill.repeat(n));
            row.push_char('+');
        }
        self.rendered.push(row);
    }
}

fn push_table(flow: &mut FlowBuilder<'_>, node: &Table) {
    assert!(!node.children.is_empty(), "empty table");
    let num_columns = node.children.iter()
        .map(|row| {
            let Node::TableRow(row) = row else { panic!() };
            row.children.len()
        })
        .max()
        .unwrap();

    let (theme, base_style) = (flow.theme, flow.prefix.cur_style());
    let cells: Vec<Vec<_>> = node.children.iter().map(|row| {
        let Node::TableRow(row) = row else { panic!() };
        let mut cells: Vec<_> = row.children.iter().map(|cell| {
            let Node::TableCell(cell) = cell else { panic!() };
            let mut phrasing = PhrasingBuilder::new(theme, base_style);
            phrasing.push_all(&cell.children);
            phrasing
        }).collect();
        cells.resize_with(num_columns, || PhrasingBuilder::new(theme, base_style));
        cells
    }).collect();

    // 2 cols padding left/right edges + 3 cols padding between each column
    let padding = 4 + 3 * num_columns.saturating_sub(1);
    let content_width = flow.remaining_width().saturating_sub(padding);
    let mut col_widths: Vec<_> = (0..num_columns)
        .map(|col|
            cells.iter()
                .map(|row| row[col].width())
                .max()
                .unwrap()
        )
        .collect();
    column_width(content_width, &mut col_widths);

    let mut builder = TableBuilder::new(col_widths);

    builder.push_horizontal();
    for row in cells {
        builder.push_row(&row);
    }

    let width = flow.remaining_width();
    for row in &builder.rendered {
        for wrapped in row.wrap_naive(width) {
            flow.row.push_markup(&wrapped);
            flow.end_row();
        }
    }
}

fn push_flow_node(flow: &mut FlowBuilder<'_>, node: &Node) {
    match node {
        Node::Root(root) => push_flow_children(flow, true, &root.children),

        Node::Table(table) => push_table(flow, table),
        Node::TableRow(_)
        | Node::TableCell(_) => {}, // TODO!

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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResumePoint {
    /// Byte offset in source
    pub offset: usize,
    /// Row in rendered output
    pub row: usize,
}

impl ResumePoint {
    pub fn add(self, other: Self) -> Self {
        Self {
            offset: self.offset + other.offset,
            row: self.row + other.row,
        }
    }
}

impl DataQuery for ResumePoint {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "offset": self.query("/offset")?,
                "row": self.query("/row")?,
            }))),
            "offset" => Ok(QueryField::Value(json!(self.offset))),
            "row" => Ok(QueryField::Value(json!(self.row))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
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
    theme: &Theme,
    width: usize,
    text: &str,
) -> MarkdownResult {
    let node = markdown::to_mdast(
        text,
        &markdown::ParseOptions {
            constructs: markdown::Constructs {
                gfm_footnote_definition: true,
                gfm_label_start_footnote: true,
                gfm_table: true,
                math_flow: true,
                math_text: true,
                ..Default::default()
            },
            ..Default::default()
        },
    ).expect("invalid markdown");
    render_mdast(theme, width, &node)
}

pub fn render_mdast(
    theme: &Theme,
    width: usize,
    node: &Node,
) -> MarkdownResult {
    let mut flow = FlowBuilder::new(theme, width);

    flow.apply_prefix();
    push_flow_node(&mut flow, node);

    flow.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ui::canvas::render_canvas;
    use crate::ui::style::{Style, THEME_DARK, UpdateStyle};
    use crate::ui::style::testing::{ResetBold, ResetItalic, SetBold, SetItalic};
    use crate::ui::styled_string::StyledString;

    fn render(text: &str, width: usize) -> String {
        let mut result = super::render_markdown(&THEME_DARK, width, text);
        render_canvas(&mut result.rows[..])
    }

    fn render_incremental(chunks: &[&str], width: usize) -> String {
        let mut source = String::new();
        let mut rows = Vec::new();
        let mut resume = ResumePoint::default();

        for chunk in chunks {
            source.push_str(chunk);
            let result = super::render_markdown(&THEME_DARK, width, &source[resume.offset..]);
            rows.truncate(resume.row);
            rows.extend(result.rows);
            resume = resume.add(result.resume_point);
        }

        render_canvas(&mut rows[..])
    }

    fn render_fully_incremental(s: &str, width: usize) -> String {
        let chunks: Vec<&str> = s.char_indices()
            .map(|(i, c)| &s[i..i + c.len_utf8()])
            .collect();
        render_incremental(&chunks, width)
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
        let reset_bold = ResetBold;
        assert_eq!(render("**hi**", 20), format!("{base}{bold}hi{reset_bold}                  "));
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
        let reset_italic = ResetItalic;
        let from_code = UpdateStyle(code, base);
        assert_eq!(
            render("**bold** *italic*\\\n`code`", 20),
            format!("{base}{bold}bold{reset_bold} {italic}italic{reset_italic}         \n{code}code{from_code}                "),
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
    fn completed_rows_not_frozen() {
        let base = THEME_DARK.base_style();
        let header = base.bolded();
        let code = Style::new(THEME_DARK.text_code, THEME_DARK.bg_base);
        let to_header = UpdateStyle(code, header);

        let result = super::render_markdown(&THEME_DARK, 20, "```\nx\n```");
        let mut s = StyledString::new(base, 32);
        s.push_styled(&result.rows[0]);
        s.set_style(header);
        s.push("|", 1);
        assert_eq!(format!("{s}"), format!("{code}x                   {to_header}|"));

        let quote = Style::new(THEME_DARK.text_quote, THEME_DARK.bg_base);
        let to_header = UpdateStyle(quote, header);
        let result = super::render_markdown(&THEME_DARK, 20, "> hi");
        let mut s = StyledString::new(base, 32);
        s.push_styled(&result.rows[0]);
        s.set_style(header);
        s.push("|", 1);
        assert_eq!(format!("{s}"), format!("{quote}▐ hi                {to_header}|"));
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
        let reset_italic = ResetItalic;
        assert_eq!(render("# Hello", 20), format!("{header}# Hello             "));
        assert_eq!(render("#### Deep", 20), format!("{header}#### Deep           "));
        assert_eq!(
            render("## Hello *world*", 20),
            format!("{header}## Hello {italic}world{reset_italic}      "),
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
        let reset_italic = ResetItalic;
        assert_eq!(render("- a", 16), format!("{base}- a             "));
        assert_eq!(render("- a\n- b", 16), format!("{base}- a             \n{base}- b             "));
        assert_eq!(
            render("- a\n\n- b", 16),
            format!("{base}- a             \n{base}                \n{base}- b             "),
        );
        assert_eq!(render("- *hi*", 16), format!("{base}- {italic}hi{reset_italic}            "));
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

    #[test]
    fn test_query() {
        let rp = ResumePoint { offset: 1, row: 2 };
        let expected = json!({ "offset": 1, "row": 2 });
        assert_eq!(rp.query("/").unwrap(), expected);
        assert_eq!(rp.query("/offset").unwrap(), json!(1));
        assert_eq!(rp.query("/row").unwrap(), json!(2));
    }

    #[test]
    fn test_list_empty_items() {
        let base = THEME_DARK.base_style();
        let res = render("1.\n1.\n1.\n", 10);
        assert_eq!(res, format!("{base}1.        \n{base}2.        \n{base}3.        "));
    }

    #[test]
    fn incremental_ordered_list() {
        const CONTENT: &str = dedent::dedent!(r#"
            1. one

            2. two
        "#);
        let chunks: Vec<&str> = CONTENT.char_indices()
            .map(|(i, c)| &CONTENT[i..i + c.len_utf8()])
            .collect();

        let base = THEME_DARK.base_style();
        let out = render_incremental(&chunks, 10);
        assert_eq!(out, format!("{base}1. one    \n{base}          \n{base}2. two    "));
    }

    #[test]
    fn incremental_unordered_list() {
        const CONTENT: &str = dedent::dedent!(r#"
            - one

            - two
        "#);
        let chunks: Vec<&str> = CONTENT.char_indices()
            .map(|(i, c)| &CONTENT[i..i + c.len_utf8()])
            .collect();

        let base = THEME_DARK.base_style();
        let out = render_incremental(&chunks, 10);
        assert_eq!(out, format!("{base}- one     \n{base}          \n{base}- two     "));
    }

    #[test]
    fn table() {
        let style = THEME_DARK.base_style();
        assert_eq!(
            render("| H1 | H2 |\n| --- | --- |\n| foo | baz |\n| bar | bum |", 20),
            format!("\
                {style}+-----+-----+       \n\
                {style}| H1  | H2  |       \n\
                {style}+=====+=====+       \n\
                {style}| foo | baz |       \n\
                {style}+-----+-----+       \n\
                {style}| bar | bum |       \n\
                {style}+-----+-----+       \
            "),
        );
    }

    #[test]
    fn table_incremental() {
        let style = THEME_DARK.base_style();
        assert_eq!(
            render_fully_incremental("| H1 | H2 |\n| --- | --- |\n| foo | baz |\n| bar | bum |", 20),
            format!("\
                {style}+-----+-----+       \n\
                {style}| H1  | H2  |       \n\
                {style}+=====+=====+       \n\
                {style}| foo | baz |       \n\
                {style}+-----+-----+       \n\
                {style}| bar | bum |       \n\
                {style}+-----+-----+       \
            "),
        );
    }

    #[test]
    fn table_cell_style_does_not_bleed() {
        let base = THEME_DARK.base_style();
        let italic = SetItalic;
        let reset_italic = ResetItalic;
        assert_eq!(
            render("| *a* | b |\n| --- | --- |\n| c | d |", 20),
            format!("\
                {base}+---+---+           \n\
                {base}| {italic}a{reset_italic} | b |           \n\
                {base}+===+===+           \n\
                {base}| c | d |           \n\
                {base}+---+---+           \
            "),
        );
    }

    #[test]
    fn table_overflow_wraps() {
        let style = THEME_DARK.base_style();
        assert_eq!(
            render("| head11 | header2 |\n| --- | --- |\n| cell11 | celltwo |", 20),
            format!("\
                {style}+--------+---------+\n\
                {style}| head11 | header2 |\n\
                {style}+========+=========+\n\
                {style}| cell11 | celltwo |\n\
                {style}+--------+---------+\
            "),
        );
        // XXX: Would prefer if history was horizontally scrollable
        assert_eq!(
            render("| head11 | header2 |\n| --- | --- |\n| cell11 | celltwo |", 10),
            format!("\
                {style}+--------+\n\
                {style}---------+\n\
                {style}| head11 |\n\
                {style} header2 |\n\
                {style}+========+\n\
                {style}=========+\n\
                {style}| cell11 |\n\
                {style} celltwo |\n\
                {style}+--------+\n\
                {style}---------+\
            "),
        );
    }

    #[test]
    fn table_column_width() {
        fn width<const N: usize>(outer: usize, mut widths: [usize; N]) -> [usize; N] {
            column_width(outer, &mut widths[..]);
            widths
        }

        assert_eq!(width(100, [25, 25, 25, 25, 25]), [20, 20, 20, 20, 20]);
        assert_eq!(width(100, [50, 25, 25, 25, 25]), [20, 20, 20, 20, 20]);
        assert_eq!(width(100, [25, 25, 25, 25, 3]), [25, 24, 24, 24, 3]);
        assert_eq!(width(100, [10; 11]), [10; 11]);
    }

    #[test]
    fn incomplete_table() {
        let style = THEME_DARK.base_style();
        assert_eq!(
            render("\
                |   | B | b |\n\
                |---|---|---|\n\
                | B\
            ", 13),
            format!("\
                {style}+---+---+---+\n\
                {style}|   | B | b |\n\
                {style}+===+===+===+\n\
                {style}| B |   |   |\n\
                {style}+---+---+---+\
            "),
        );
    }
}
