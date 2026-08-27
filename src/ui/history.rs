//! Scrolling chat history. Data is constructed as a linked list of rows for
//! fast viewport scrolling and rendering.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use fnv::FnvHashMap;
use serde_json::{Value, json};

use crate::arena::{Arena, Id};
use crate::query::{DataQuery, QueryError, QueryField, ToJson};
use crate::session::{Item, ItemType};
use crate::ui::Component;
use crate::ui::canvas::Canvas;
use crate::ui::markdown::{MarkdownResult, ResumePoint};
use crate::ui::style::{Style, Theme};
use crate::ui::styled_string::StyledString;
use crate::ui::text::{SPACES, wrap_line, wrap_line_naive};

pub(crate) fn render_help(
    theme: &'static Theme,
    width: usize,
    content: &str,
) -> Vec<StyledString> {
    content.lines().flat_map(|line| {
        wrap_line(width - 4, line)
            .into_iter()
            .map(|row| {
                let style = Style::new(theme.text_subtle, theme.bg_base);
                let mut s = StyledString::new(style, width + 4);
                s.push("▐ ", 2);
                s.set_text(theme.text_quote);
                s.push(&row.to_padded_string(width - 4), width - 4);
                s.push("  ", 2);
                s
            })
        })
        .collect()
}

pub(crate) fn render_error(
    theme: &'static Theme,
    width: usize,
    content: &str,
) -> Vec<StyledString> {
    content.lines().flat_map(|line| {
        wrap_line(width - 4, line)
            .into_iter()
            .map(|row| {
                let style = Style::new(theme.text_error, theme.bg_base);
                let mut s = StyledString::new(style, width + 4);
                s.push("▐ ", 2);
                s.set_text(theme.text_subtle);
                s.push(&row.to_padded_string(width - 4), width - 4);
                s.push("  ", 2);
                s
            })
        })
        .collect()
}

pub(crate) fn render_prompt(
    theme: &'static Theme,
    width: usize,
    content: &str,
) -> Vec<StyledString> {
    if content.is_empty() {
        return Vec::new();
    }
    let style = Style::new(theme.text_base, theme.bg_prompt);

    let make_padding = || {
        let mut s = StyledString::new(style, width + 4);
        s.push(&SPACES[..width], width);
        s
    };

    let mut rows = vec![make_padding()];
    rows.extend(content.lines().flat_map(|line| {
        wrap_line(width - 4, line).into_iter().map(|row| {
            let mut s = StyledString::new(style, width + 4);
            s.push("  ", 2);
            s.push(&row.to_padded_string(width - 4), width - 4);
            s.push("  ", 2);
            s
        })
    }));
    rows.push(make_padding());
    rows
}

pub(crate) fn render_markdown(
    theme: &'static Theme,
    width: usize,
    content: &str,
) -> MarkdownResult {
    let mut result = crate::ui::markdown::render_markdown(theme, width - 4, content);
    for row in result.rows.iter_mut() {
        let mut padded = StyledString::new(theme.base_style(), width + 4);
        padded.push("  ", 2);
        padded.push_styled(row);
        padded.pad_to_width(width);
        *row = padded;
    }
    result
}

pub(crate) fn render_thought(
    theme: &'static Theme,
    width: usize,
    content: &str,
) -> MarkdownResult {
    let bar_style = Style::new(theme.text_thought, theme.bg_base);
    let mut result = crate::ui::markdown::render_markdown(theme, width - 4, content);
    for row in result.rows.iter_mut() {
        let mut padded = StyledString::new(bar_style, width + 4);
        padded.push("▐ ", 2);
        padded.push_styled(row);
        padded.pad_to_width(width);
        *row = padded;
    }
    result
}

fn render_command_prompt(
    theme: &'static Theme,
    width: usize,
    content: &str,
) -> Vec<StyledString> {
    let style = Style::new(theme.text_base, theme.bg_base);
    content
        .lines()
        .flat_map(|line| {
            wrap_line_naive(width - 4, line).into_iter().map(|row| {
                let mut s = StyledString::new(style, width + 4);
                s.push("  ", 2);
                s.push(&row.to_padded_string(width - 4), width - 4);
                s.push("  ", 2);
                s
            })
        })
        .collect()
}

fn render_command_output(theme: &'static Theme, width: usize, content: &str) -> Vec<StyledString> {
    let style = Style::new(theme.text_subtle, theme.bg_base);
    content
        .lines()
        .flat_map(|line| {
            wrap_line_naive(width - 4, line).into_iter().map(|row| {
                let mut s = StyledString::new(style, width + 4);
                s.push("  ", 2);
                s.push(&row.to_padded_string(width - 4), width - 4);
                s.push("  ", 2);
                s
            })
        })
        .collect()
}

fn render(
    theme: &'static Theme,
    width: usize,
    ty: HistoryItemType,
    content: &str,
) -> (Vec<StyledString>, ResumePoint) {
    match ty {
        HistoryItemType::Help => (render_help(theme, width, content), ResumePoint { offset: 0, row: 0 }),
        HistoryItemType::Error => (render_error(theme, width, content), ResumePoint { offset: 0, row: 0 }),
        HistoryItemType::User => (render_prompt(theme, width, content), ResumePoint { offset: 0, row: 0 }),
        HistoryItemType::CommandPrompt => (render_command_prompt(theme, width, content), ResumePoint { offset: 0, row: 0 }),
        HistoryItemType::CommandOutput => (render_command_output(theme, width, content), ResumePoint { offset: 0, row: 0 }),
        HistoryItemType::Thought => {
            let result = render_thought(theme, width, content);
            (result.rows, result.resume_point)
        }
        _ => {
            let result = render_markdown(theme, width, content);
            (result.rows, result.resume_point)
        }
    }
}

fn get_item_type(item: &Item) -> HistoryItemType {
    match item.ty().unwrap() {
        ItemType::UserText => HistoryItemType::User,
        ItemType::ResponseText => HistoryItemType::Response,
        ItemType::Reasoning => HistoryItemType::Thought,
    }
}

/// Returns the text to display for an item, preferring the raw text over the
/// summary for reasoning items.
fn item_text(item: &Item) -> &str {
    item.text
        .as_deref()
        .or(item.summary.as_deref())
        .unwrap_or("")
}

#[derive(Debug)]
pub struct HistoryRow {
    item: Id<HistoryItem>,
    /// Preformatted, pre-padded row contents
    preformatted: StyledString,
    prev: Id<HistoryRow>,
    next: Id<HistoryRow>,
}

/// Walks `offset` rows from `base` in the circularly linked row list
/// terminated by the `head` sentinel. Returns `None` if the walk crosses
/// `head`. `base` itself is returned for `offset == 0`.
fn row_offset(
    rows: &Arena<HistoryRow>,
    head: Id<HistoryRow>,
    base: Id<HistoryRow>,
    offset: isize,
) -> Option<Id<HistoryRow>> {
    let mut row = base;
    if offset >= 0 {
        for _ in 0..offset {
            row = rows[row].next;
            if row == head {
                return None;
            }
        }
    } else {
        for _ in 0..-offset {
            row = rows[row].prev;
            if row == head {
                return None;
            }
        }
    }
    Some(row)
}

/// Deletes and unlinks rows from the arena
fn remove_rows(rows: &mut Arena<HistoryRow>, prev: Id<HistoryRow>, count: usize) {
    for _ in 0..count {
        let next = rows[prev].next;
        rows[prev].next = rows[next].next;
        rows.remove(next);
    }
}

// Inserts and links rows
fn insert_rows(
    rows: &mut Arena<HistoryRow>,
    item: Id<HistoryItem>,
    rendered: Vec<StyledString>,
    prev: Id<HistoryRow>,
) {
    let next = rows[prev].next;
    let mut last = prev;
    for preformatted in rendered {
        let id = rows.insert(HistoryRow {
            item,
            preformatted,
            prev: last,
            next,
        });
        rows[last].next = id;
        last = id;
    }
    rows[next].prev = last;
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum HistoryItemType {
    Help,
    Error,
    User,
    Thought,
    Response,
    CommandPrompt,
    CommandOutput,
}

impl HistoryItemType {
    pub fn as_str(self) -> &'static str {
        match self {
            HistoryItemType::Help => "help",
            HistoryItemType::Error => "error",
            HistoryItemType::User => "user",
            HistoryItemType::Thought => "thought",
            HistoryItemType::Response => "response",
            HistoryItemType::CommandPrompt => "command_prompt",
            HistoryItemType::CommandOutput => "command_output",
        }
    }
}

/// Exposes the item type as a string, matching `as_str()`.
impl ToJson for HistoryItemType {
    fn to_json(self) -> Value {
        self.as_str().into()
    }
}

#[derive(Clone, Debug)]
pub struct HistoryItem {
    content: String,
    ty: HistoryItemType,
    resume_point: ResumePoint,
    item_id: Option<i32>,
    first_row: Id<HistoryRow>,
    last_row: Id<HistoryRow>,
    num_rows: usize,
}

impl HistoryItem {
    fn new(
        theme: &'static Theme,
        items: &mut Arena<HistoryItem>,
        rows: &mut Arena<HistoryRow>,
        prev: Id<HistoryRow>,
        width: usize,
        ty: HistoryItemType,
        content: String,
        item_id: Option<i32>,
    ) -> Id<Self> {
        let (rendered, resume_point) = render(theme, width, ty, &content);
        Self::from_rendered(
            theme,
            items,
            rows,
            prev,
            width,
            ty,
            content,
            item_id,
            resume_point,
            rendered,
        )
    }

    fn from_rendered(
        theme: &'static Theme,
        items: &mut Arena<HistoryItem>,
        rows: &mut Arena<HistoryRow>,
        prev: Id<HistoryRow>,
        width: usize,
        ty: HistoryItemType,
        content: String,
        item_id: Option<i32>,
        resume_point: ResumePoint,
        mut rendered: Vec<StyledString>,
    ) -> Id<Self> {
        let next = rows[prev].next;

        let item = items.insert(Self {
            content,
            ty,
            resume_point,
            item_id,
            first_row: Id::null(),
            last_row: Id::null(),
            num_rows: 0,
        });

        if ty != HistoryItemType::CommandPrompt {
            let mut padding = StyledString::new(theme.base_style(), width);
            padding.pad_to_width(width);
            rendered.push(padding); // Add vertical padding row
        }
        let num_rows = rendered.len();
        insert_rows(rows, item, rendered, prev);

        items[item].first_row = rows[prev].next;
        items[item].last_row = rows[next].prev;
        items[item].num_rows = num_rows;

        item
    }

    // Appends text and rerenders
    pub fn append(
        &mut self,
        theme: &'static Theme,
        rows: &mut Arena<HistoryRow>,
        head: Id<HistoryRow>,
        width: usize,
        delta: &str,
    ) {
        let old_offset = self.resume_point.offset;
        let old_row = self.resume_point.row;

        self.content.push_str(delta);

        let result = render_markdown(theme, width, &self.content[old_offset..]);

        // Find insertion position
        let steps_back = self.num_rows - 1 - old_row;
        let first = row_offset(rows, head, self.last_row, -(steps_back as isize))
            .expect("resume point out of bounds");
        let prev = rows[first].prev;
        let next = rows[self.last_row].next;
        let item_id = rows[self.first_row].item;

        // Replace rows
        remove_rows(rows, prev, self.num_rows - old_row);

        let mut rendered: Vec<StyledString> = result.rows;
        let mut padding = StyledString::new(theme.base_style(), width);
        padding.pad_to_width(width);
        rendered.push(padding);
        let len = rendered.len();

        insert_rows(rows, item_id, rendered, prev);

        // Update item
        if old_row == 0 {
            self.first_row = rows[prev].next;
        }
        self.last_row = rows[next].prev;
        self.num_rows = old_row + len;

        self.resume_point = ResumePoint {
            offset: old_offset + result.resume_point.offset,
            row: old_row + result.resume_point.row,
        };
    }

    // Updates and re-renders the item
    pub fn update(
        &mut self,
        theme: &'static Theme,
        rows: &mut Arena<HistoryRow>,
        width: usize,
        new_value: &str,
    ) {
        self.content = new_value.to_string();
        let (rendered, resume_point) = render(theme, width, self.ty, new_value);

        let prev = rows[self.first_row].prev;
        let next = rows[self.last_row].next;
        let item_id = rows[self.first_row].item;
        remove_rows(rows, prev, self.num_rows);

        let mut rendered = rendered;
        let mut padding = StyledString::new(theme.base_style(), width);
        padding.pad_to_width(width);
        rendered.push(padding);
        let len = rendered.len();
        insert_rows(rows, item_id, rendered, prev);

        self.first_row = rows[prev].next;
        self.last_row = rows[next].prev;
        self.num_rows = len;
        self.resume_point = resume_point;
    }
}

/// Exposed fields:
///
/// - content: string
/// - ty: string
/// - resume_point:
///   - offset: number
///   - row: number
/// - item_id: number | null
/// - first_row: id
/// - last_row: id
/// - num_rows: number
impl DataQuery for HistoryItem {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "content": self.query("/content")?,
                "ty": self.query("/ty")?,
                "resume_point": self.query("/resume_point")?,
                "item_id": self.query("/item_id")?,
                "first_row": self.query("/first_row")?,
                "last_row": self.query("/last_row")?,
                "num_rows": self.query("/num_rows")?,
            }))),
            "content" => Ok(QueryField::Value(json!(self.content))),
            "ty" => Ok(QueryField::Value(json!(self.ty.as_str()))),
            "resume_point" => Ok(QueryField::DataQuery(&self.resume_point)),
            "item_id" => Ok(QueryField::Value(json!(self.item_id))),
            "first_row" => Ok(QueryField::Value(self.first_row.to_json())),
            "last_row" => Ok(QueryField::Value(self.last_row.to_json())),
            "num_rows" => Ok(QueryField::Value(json!(self.num_rows))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[derive(Debug)]
pub struct History {
    item: Arena<HistoryItem>,
    rows: Arena<HistoryRow>,
    width: usize,
    theme: &'static Theme,
    /// Maximum viewport size
    max_height: usize,
    /// Head of circularly linked list. Contains no real data.
    head: Id<HistoryRow>,
    viewport_top: Id<HistoryRow>,
    /// Absolute row index of `viewport_top`
    viewport_top_pos: usize,
    viewport_bottom: Id<HistoryRow>,
    /// Absolute row index of `viewport_bottom`
    viewport_bottom_pos: usize,
    /// Maps an `Item` id to the history item rendering it, so that
    /// `ItemUpdated` events can locate and rerender the right item.
    by_item_id: FnvHashMap<i32, Id<HistoryItem>>,
}

impl History {
    pub fn new(width: usize, max_height: usize, theme: &'static Theme) -> Self {
        let item = Arena::new();
        let mut rows = Arena::new();

        // Insert dummy head, distinct from all other rows.
        let head = rows.insert(HistoryRow {
            item: Id::null(),
            preformatted: StyledString::new(theme.base_style(), 0),
            prev: Id::null(),
            next: Id::null(),
        });
        rows[head].prev = head;
        rows[head].next = head;

        Self {
            item,
            rows,
            width,
            theme,
            max_height,
            head,
            viewport_top: head,
            viewport_bottom: head,
            viewport_top_pos: 0,
            viewport_bottom_pos: 0,
            by_item_id: FnvHashMap::default(),
        }
    }

    pub fn num_rows(&self) -> usize {
        // Subtract header node
        self.rows.len() - 1
    }

    fn item_ids(&self) -> Vec<Id<HistoryItem>> {
        let mut items: Vec<Id<HistoryItem>> = Vec::new();
        for (_, row) in self.iter_range(self.head, self.last_row()) {
            let item = row.item;
            if items.last() != Some(&item) {
                items.push(item);
            }
        }
        items
    }

    fn first_row(&self) -> Id<HistoryRow> {
        self.rows[self.head].next
    }

    fn last_row(&self) -> Id<HistoryRow> {
        self.rows[self.head].prev
    }

    /// Iterate over a range of rows. `prev` is not inclusive; `last` is
    /// inclusive.
    fn iter_range<'a>(&'a self, prev: Id<HistoryRow>, last: Id<HistoryRow>) -> HistoryRowIter<'a> {
        HistoryRowIter {
            rows: &self.rows,
            prev,
            last,
        }
    }

    /// O(n) row lookup relative to base row. Returns None if the offset is
    /// out of bounds.
    fn row_offset(&self, base: Id<HistoryRow>, offset: isize) -> Option<Id<HistoryRow>> {
        row_offset(&self.rows, self.head, base, offset)
    }

    /// O(n) row distance relative to base. base must come before other.
    /// Unspecified result if base comes after other.
    #[cfg(test)]
    fn row_diff(&self, base: Id<HistoryRow>, other: Id<HistoryRow>) -> isize {
        let mut row = base;
        let mut diff = 0;
        while row != other {
            row = self.rows[row].next;
            diff += 1;
        }
        diff
    }

    /// Attempts to set the viewport region based on first row. `pos` is the
    /// absolute row index of `viewport_top` (0-based from `first_row()`).
    fn set_viewport_top_at(&mut self, viewport_top: Id<HistoryRow>, pos: usize) {
        let prev = self.rows[viewport_top].prev;
        self.viewport_top = viewport_top;
        self.viewport_top_pos = pos;
        if let Some(row) = self.row_offset(prev, self.max_height as _) {
            self.viewport_bottom = row;
            self.viewport_bottom_pos = pos + self.max_height - 1;
        } else if viewport_top == self.first_row() {
            self.viewport_bottom = self.last_row();
            self.viewport_bottom_pos = self.num_rows() - 1;
        } else {
            self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
        }
    }

    /// Attempts to set the viewport region based on last row. `pos` is the
    /// absolute row index of `viewport_bottom` (0-based from `first_row()`).
    fn set_viewport_bottom_at(&mut self, viewport_bottom: Id<HistoryRow>, pos: usize) {
        self.viewport_bottom = viewport_bottom;
        self.viewport_bottom_pos = pos;
        if let Some(row) = self.row_offset(self.viewport_bottom, -(self.max_height as isize - 1)) {
            self.viewport_top = row;
            self.viewport_top_pos = pos - (self.max_height - 1);
        } else {
            // Viewport covers entire text
            self.viewport_top = self.first_row();
            self.viewport_top_pos = 0;
        }
    }

    /// Slightly inefficient helper for tests
    #[cfg(test)]
    fn set_viewport_top(&mut self, viewport_top: Id<HistoryRow>) {
        let pos = self.row_diff(self.first_row(), viewport_top) as usize;
        self.set_viewport_top_at(viewport_top, pos);
    }

    pub fn max_height(&self) -> usize {
        self.max_height
    }

    /// Absolute row index of the first visible row (0-based from `first_row()`).
    pub fn viewport_top_pos(&self) -> usize {
        self.viewport_top_pos
    }

    /// Absolute row index of the last visible row (0-based from `first_row()`).
    pub fn viewport_bottom_pos(&self) -> usize {
        self.viewport_bottom_pos
    }

    /// Updates the maximum viewport size, preserving the viewport bottom.
    pub fn set_max_height(&mut self, max_height: usize) {
        if max_height == 0 {
            return;
        }
        self.max_height = max_height;
        self.set_viewport_bottom_at(self.viewport_bottom, self.viewport_bottom_pos);
    }

    /// Updates the wrapping width, re-rendering all markdown items. The
    /// viewport continues to follow the newest messages.
    pub fn set_width(&mut self, width: usize) {
        if width == self.width {
            return;
        }
        self.width = width;

        let saved: Vec<(HistoryItemType, String, Option<i32>)> = self
            .item
            .iter()
            .map(|(_, item)| (item.ty, item.content.clone(), item.item_id))
            .collect();

        self.item.clear();
        self.rows.clear();
        self.by_item_id.clear();

        let head = self.rows.insert(HistoryRow {
            item: Id::null(),
            preformatted: StyledString::new(self.theme.base_style(), 0),
            prev: Id::null(),
            next: Id::null(),
        });
        self.rows[head].prev = head;
        self.rows[head].next = head;
        self.head = head;
        self.viewport_top = head;
        self.viewport_bottom = head;
        self.viewport_top_pos = 0;
        self.viewport_bottom_pos = 0;

        for (ty, content, item_id) in saved {
            let prev = self.last_row();
            let id = HistoryItem::new(
                self.theme,
                &mut self.item,
                &mut self.rows,
                prev,
                width,
                ty,
                content,
                item_id,
            );
            if let Some(id2) = item_id {
                self.by_item_id.insert(id2, id);
            }
            self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
        }
    }

    /// Appends an item to the history.
    fn add_item(&mut self, ty: HistoryItemType, content: String) {
        let prev = self.last_row();
        HistoryItem::new(
            self.theme,
            &mut self.item,
            &mut self.rows,
            prev,
            self.width,
            ty,
            content,
            None,
        );
        self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
    }

    /// Creates (or updates) an item
    fn on_item_created(&mut self, item: &Item) {
        if self.by_item_id.contains_key(&item.id) {
            self.on_item_updated(item);
            return;
        }

        let ty = get_item_type(item);
        let prev = self.last_row();
        let id = HistoryItem::new(
            self.theme,
            &mut self.item,
            &mut self.rows,
            prev,
            self.width,
            ty,
            item_text(item).to_string(),
            Some(item.id),
        );
        self.by_item_id.insert(item.id, id);
        self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
    }

    /// Does an incremental rerender for an updated item
    fn on_item_updated(&mut self, item: &Item) {
        if let Some(&item_id) = self.by_item_id.get(&item.id) {
            let theme = self.theme;
            let width = self.width;
            self.item[item_id].update(theme, &mut self.rows, width, item_text(item));
            self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
        } else {
            self.on_item_created(item);
        }
    }

    fn scroll_up(&mut self, rows: usize) {
        let (top, pos) = match self.row_offset(self.viewport_top, -(rows as isize)) {
            Some(row) => (row, self.viewport_top_pos - rows),
            None => (self.first_row(), 0),
        };
        self.set_viewport_top_at(top, pos);
    }

    fn scroll_down(&mut self, rows: usize) {
        if let Some(top) = self.row_offset(self.viewport_top, rows as isize) {
            self.set_viewport_top_at(top, self.viewport_top_pos + rows);
        } else {
            // Can't scroll any further; anchor to the bottom.
            self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Update<'a> {
    ItemCreated { item: &'a Item },
    ItemUpdated { item: &'a Item },
    HelpMessage(&'a str),
    ErrorMessage(&'a str),
    CommandPrompt(&'a str),
    CommandOutput(&'a str),
}

impl Component for History {
    type Update<'a> = Update<'a>;
    type Event = ();

    fn draw(&self, canvas: Canvas) {
        let prev = self.rows[self.viewport_top].prev;
        for (i, (_, row)) in self.iter_range(prev, self.viewport_bottom).enumerate() {
            if i >= canvas.len() {
                break;
            }
            canvas[i].push_styled(&row.preformatted);
        }
    }

    fn set_width(&mut self, width: usize) {
        History::set_width(self, width);
    }

    fn set_height(&mut self, height: usize) {
        self.set_max_height(height);
    }

    fn set_focus(&mut self, _focused: bool) {}

    fn width(&self) -> usize {
        self.width
    }

    /// Number of actually visible rows.
    fn height(&self) -> usize {
        std::cmp::min(self.max_height, self.num_rows())
    }

    fn cursor(&self) -> Option<(usize, usize)> {
        None
    }

    fn handle_input(&mut self, event: Event) -> Self::Event {
        let KeyEvent {
            code, modifiers, ..
        } = match event {
            Event::Key(key) => key,
            _ => return,
        };
        let alt = modifiers.contains(KeyModifiers::ALT);
        match (code, alt) {
            (KeyCode::Up, _) => self.scroll_up(1),
            (KeyCode::Down, _) => self.scroll_down(1),
            (KeyCode::PageUp, _) | (KeyCode::Char('u'), true) => self.scroll_up(self.height() / 2),
            (KeyCode::PageDown, _) | (KeyCode::Char('d'), true) => {
                self.scroll_down(self.height() / 2)
            }
            (KeyCode::Home, _) => self.set_viewport_top_at(self.first_row(), 0),
            (KeyCode::End, _) => self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1),
            _ => {}
        }
    }

    fn handle_update<'a>(&mut self, update: Self::Update<'a>) {
        match update {
            Update::ItemCreated { item } => self.on_item_created(&item),
            Update::ItemUpdated { item } => self.on_item_updated(&item),
            Update::HelpMessage(content) => self.add_item(HistoryItemType::Help, content.into()),
            Update::ErrorMessage(content) => self.add_item(HistoryItemType::Error, content.into()),
            Update::CommandPrompt(content) => {
                self.add_item(HistoryItemType::CommandPrompt, content.into())
            }
            Update::CommandOutput(content) => {
                self.add_item(HistoryItemType::CommandOutput, content.into())
            }
        }
    }
}

fn by_item_id_json(map: &FnvHashMap<i32, Id<HistoryItem>>) -> Value {
    map.iter()
        .map(|(k, id)| (k.to_string(), (*id).to_json()))
        .collect()
}

/// Exposed fields:
/// - num_rows: number
/// - items: HistoryItem[] (in linked-list order)
/// - width: number
/// - max_height: number
/// - head: id
/// - viewport_top: id
/// - viewport_top_pos: number
/// - viewport_bottom: id
/// - viewport_bottom_pos: number
/// - by_item_id: Map<string, id>
// rows is intentionally not exposed as of yet
impl DataQuery for History {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "num_rows": self.query("/num_rows")?,
                "items": self.query("/items")?,
                "width": self.query("/width")?,
                "max_height": self.query("/max_height")?,
                "head": self.query("/head")?,
                "viewport_top": self.query("/viewport_top")?,
                "viewport_top_pos": self.query("/viewport_top_pos")?,
                "viewport_bottom": self.query("/viewport_bottom")?,
                "viewport_bottom_pos": self.query("/viewport_bottom_pos")?,
                "by_item_id": self.query("/by_item_id")?,
            }))),
            "num_rows" => Ok(QueryField::Value(json!(self.num_rows()))),
            "items" => Ok(QueryField::Boxed(Box::new(HistoryItemsData {
                history: self,
            }))),
            "width" => Ok(QueryField::Value(json!(self.width))),
            "max_height" => Ok(QueryField::Value(json!(self.max_height))),
            "head" => Ok(QueryField::Value(self.head.to_json())),
            "viewport_top" => Ok(QueryField::Value(self.viewport_top.to_json())),
            "viewport_top_pos" => Ok(QueryField::Value(json!(self.viewport_top_pos))),
            "viewport_bottom" => Ok(QueryField::Value(self.viewport_bottom.to_json())),
            "viewport_bottom_pos" => Ok(QueryField::Value(json!(self.viewport_bottom_pos))),
            "by_item_id" => Ok(QueryField::Value(by_item_id_json(&self.by_item_id))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct HistoryRowIter<'i> {
    rows: &'i Arena<HistoryRow>,
    prev: Id<HistoryRow>,
    last: Id<HistoryRow>,
}

impl<'i> Iterator for HistoryRowIter<'i> {
    type Item = (Id<HistoryRow>, &'i HistoryRow);

    fn next(&mut self) -> Option<Self::Item> {
        if self.prev == self.last {
            return None;
        }
        let id = self.rows[self.prev].next;
        self.prev = id;
        Some((id, &self.rows[id]))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        ((self.prev != self.last) as usize, None)
    }
}

impl<'i> DoubleEndedIterator for HistoryRowIter<'i> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.prev == self.last {
            return None;
        }
        let id = self.last;
        self.last = self.rows[self.last].prev;
        Some((id, &self.rows[id]))
    }
}

impl<'i> std::iter::FusedIterator for HistoryRowIter<'i> {}

/// Helper which implements DataQuery for `History.items`. It can fetch a
/// single item or all items at once by iteration.
#[derive(Debug)]
struct HistoryItemsData<'a> {
    history: &'a History,
}

impl<'h> DataQuery for HistoryItemsData<'h> {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        let items = self.history.item_ids();
        if field.is_empty() {
            let arr: Vec<Value> = items
                .iter()
                .map(|id| self.history.item[*id].query("/"))
                .collect::<Result<_, _>>()?;
            Ok(QueryField::Value(arr.into()))
        } else {
            let index: usize = field
                .parse()
                .map_err(|_| QueryError::InvalidField(field.to_string()))?;
            let id = items
                .get(index)
                .ok_or_else(|| QueryError::InvalidField(field.to_string()))?;
            Ok(QueryField::DataQuery(&self.history.item[*id]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ui::canvas::render_canvas;
    use crate::ui::style::THEME_DARK;
    use crate::ui::style::testing::SetItalic;
    use serde_json::json;

    fn history(width: usize, max_height: usize) -> History {
        History::new(width, max_height, &THEME_DARK)
    }

    fn update_item(history: &mut History, index: usize, delta: &str) {
        let id = history.item.id_at(index);
        let theme = history.theme;
        let width = history.width;
        history.item[id].append(theme, &mut history.rows, history.head, width, delta);
        history.set_viewport_bottom_at(history.last_row(), history.num_rows() - 1);
    }

    fn render_draw(history: &History) -> String {
        let mut rows: Vec<StyledString> = (0..history.height())
            .map(|_| StyledString::new(history.theme.base_style(), history.width()))
            .collect();
        history.draw(&mut rows);
        render_canvas(&mut rows[..])
    }

    #[test]
    fn test_render_draw() {
        let theme = &THEME_DARK;

        let mut h = history(12, 5);
        assert_eq!(h.width(), 12);
        assert_eq!(h.height(), 0);
        assert_eq!(h.cursor(), None);
        assert_eq!(render_draw(&h), "");

        h.handle_update(Update::HelpMessage("hello"));
        assert_eq!(h.height(), 2);
        assert_eq!(h.num_rows(), 2);

        let help_style = Style::new(theme.text_subtle, theme.bg_base);
        let base_style = theme.base_style();
        let italic = SetItalic;

        assert_eq!(
            render_draw(&h),
            format!("{help_style}▐ {italic}hello     \n{base_style}            "),
        );

        let mut h = history(12, 2);
        h.handle_update(Update::HelpMessage("one\ntwo"));
        assert_eq!(h.num_rows(), 3);
        assert_eq!(h.height(), 2);

        assert_eq!(render_draw(&h), format!("{help_style}▐ {italic}two       \n{base_style}            "));

        h.scroll_up(1);
        assert_eq!(render_draw(&h), format!("{help_style}▐ {italic}one       \n{help_style}▐ {italic}two       "));

        h.set_width(14);
        assert_eq!(h.width(), 14);

        h.set_height(4);
        assert_eq!(h.height(), 3);
    }

    #[test]
    fn test_empty_history() {
        let history = history(20, 5);
        assert_eq!(history.num_rows(), 0);
        assert_eq!(history.item.len(), 0);
        assert_eq!(history.viewport_top, history.head);
        assert_eq!(history.viewport_bottom, history.head);
        assert_eq!(history.viewport_top_pos(), 0);
        assert_eq!(history.viewport_bottom_pos(), 0);
    }

    #[test]
    fn test_render_help() {
        let theme = &THEME_DARK;
        let help_style = Style::new(theme.text_subtle, theme.bg_base);
        let italic = SetItalic;

        fn render(content: &str, width: usize) -> String {
            let mut lines = super::render_help(&THEME_DARK, width, content);
            render_canvas(&mut lines[..])
        }

        assert_eq!(render("hello", 14), format!("{help_style}▐ {italic}hello       "));
        assert_eq!(render("foo\nbar", 12), format!("{help_style}▐ {italic}foo       \n{help_style}▐ {italic}bar       "));
        assert_eq!(render("hello world", 12), format!("{help_style}▐ {italic}hello     \n{help_style}▐ {italic}world     "));
        assert_eq!(render("", 8), "");
    }

    #[test]
    fn test_render_error() {
        use crate::ui::style::UpdateStyle;

        let theme = &THEME_DARK;
        let error_style = Style::new(theme.text_error, theme.bg_base);
        let subtle_style = Style::new(theme.text_subtle, theme.bg_base);
        let transition = UpdateStyle(error_style, subtle_style);

        fn render(content: &str, width: usize) -> String {
            let mut lines = super::render_error(&THEME_DARK, width, content);
            render_canvas(&mut lines[..])
        }

        assert_eq!(render("hello", 12), format!("{error_style}▐ {transition}hello     "));
        assert_eq!(render("foo\nbar", 12), format!("{error_style}▐ {transition}foo       \n{error_style}▐ {transition}bar       "));
        assert_eq!(render("hello world", 12), format!("{error_style}▐ {transition}hello     \n{error_style}▐ {transition}world     "));
        assert_eq!(render("", 8), "");
    }

    #[test]
    fn test_render_thought() {
        use crate::ui::style::UpdateStyle;

        let theme = &THEME_DARK;
        let base_style = theme.base_style();
        let thought_style = Style::new(theme.text_thought, theme.bg_base);
        let transition = UpdateStyle(thought_style, base_style);

        fn render(content: &str, width: usize) -> String {
            let mut lines = super::render_thought(&THEME_DARK, width, content).rows;
            render_canvas(&mut lines[..])
        }

        assert_eq!(render("hello", 14), format!("{thought_style}▐ {transition}hello       "));
        assert_eq!(render("foo\nbar", 12), format!("{thought_style}▐ {transition}foo bar   "));
        assert_eq!(render("hello world", 12), format!("{thought_style}▐ {transition}hello     \n{thought_style}▐ {transition}world     "));
        assert_eq!(render("", 8), "");
    }

    #[test]
    fn test_render_prompt() {
        let theme = &THEME_DARK;
        let prompt_style = Style::new(theme.text_base, theme.bg_prompt);

        fn render(content: &str, width: usize) -> String {
            let mut lines = super::render_prompt(&THEME_DARK, width, content);
            render_canvas(&mut lines[..])
        }

        assert_eq!(
            render("hello", 14),
            format!(
                "{prompt_style}              \n{prompt_style}  hello       \n{prompt_style}              "
            )
        );
        assert_eq!(
            render("foo\nbar", 12),
            format!(
                "{prompt_style}            \n{prompt_style}  foo       \n{prompt_style}  bar       \n{prompt_style}            "
            )
        );
        assert_eq!(
            render("hello world", 12),
            format!(
                "{prompt_style}            \n{prompt_style}  hello     \n{prompt_style}  world     \n{prompt_style}            "
            )
        );
        assert_eq!(render("", 8), "");
    }

    #[test]
    fn test_render_command() {
        let theme = &THEME_DARK;
        let prompt_style = Style::new(theme.text_base, theme.bg_base);
        let output_style = Style::new(theme.text_subtle, theme.bg_base);

        fn render_prompt(content: &str, width: usize) -> String {
            let mut lines = super::render_command_prompt(&THEME_DARK, width, content);
            render_canvas(&mut lines[..])
        }
        fn render_output(content: &str, width: usize) -> String {
            let mut lines = super::render_command_output(&THEME_DARK, width, content);
            render_canvas(&mut lines[..])
        }

        assert_eq!(
            render_prompt("!foo", 14),
            format!("{prompt_style}  !foo        ")
        );
        assert_eq!(
            render_prompt("!foo\n!bar", 14),
            format!("{prompt_style}  !foo        \n{prompt_style}  !bar        ")
        );
        assert_eq!(
            render_prompt("!hello world foo", 14),
            format!("{prompt_style}  !hello wor  \n{prompt_style}  ld foo      ")
        );
        assert_eq!(
            render_output("ok", 14),
            format!("{output_style}  ok          ")
        );
        assert_eq!(
            render_output("line1\nline2", 14),
            format!("{output_style}  line1       \n{output_style}  line2       ")
        );
        assert_eq!(render_prompt("", 8), "");
        assert_eq!(render_output("", 8), "");
    }

    #[test]
    fn test_command_item_padding() {
        let mut h = history(20, 10);
        h.handle_update(Update::CommandPrompt("!echo"));
        assert_eq!(h.num_rows(), 1);

        h.handle_update(Update::CommandOutput("hi"));
        assert_eq!(h.num_rows(), 3);

        h.handle_update(Update::CommandOutput("line one\nline two"));
        assert_eq!(h.num_rows(), 6);
    }

    #[test]
    fn test_scroll() {
        let mut history = history(80, 4);
        for i in 0..10 {
            history.add_item(HistoryItemType::Response, format!("message {i}"));
        }
        assert_eq!(history.num_rows(), 20);

        // New items are anchored to the bottom.
        let last = history.last_row();
        let top = history.viewport_top;
        assert_eq!(history.viewport_bottom, last);
        assert_eq!(history.viewport_top_pos(), 16);
        assert_eq!(history.viewport_bottom_pos(), 19);
        assert_ne!(top, history.first_row());

        history.scroll_up(1);
        assert_ne!(history.viewport_bottom, last);
        assert_ne!(history.viewport_top, top);
        assert_eq!(history.viewport_top_pos(), 15);
        assert_eq!(history.viewport_bottom_pos(), 18);

        history.scroll_down(1);
        assert_eq!(history.viewport_top, top);
        assert_eq!(history.viewport_bottom, last);
        assert_eq!(history.viewport_top_pos(), 16);
        assert_eq!(history.viewport_bottom_pos(), 19);

        // Hit bottom
        history.scroll_down(1000);
        assert_eq!(history.viewport_bottom, last);
        assert_eq!(history.viewport_top_pos(), 16);
        assert_eq!(history.viewport_bottom_pos(), 19);

        // Hit top
        history.scroll_up(1000);
        assert_eq!(history.viewport_top, history.first_row());
        assert_eq!(history.viewport_top_pos(), 0);
        assert_eq!(history.viewport_bottom_pos(), 3);
    }

    #[test]
    fn test_set_viewport_top_pos() {
        let mut history = history(80, 4);
        for i in 0..10 {
            history.add_item(HistoryItemType::Response, format!("message {i}"));
        }

        let row = history.row_offset(history.first_row(), 5).unwrap();
        history.set_viewport_top(row);
        assert_eq!(history.viewport_top, row);
        assert_eq!(history.viewport_top_pos(), 5);
        assert_eq!(history.viewport_bottom_pos(), 8);
    }

    #[test]
    fn test_home_end() {
        let mut history = history(80, 4);
        for i in 0..10 {
            history.add_item(HistoryItemType::Response, format!("message {i}"));
        }

        // Start at the bottom; scroll up so we're not at either extreme.
        history.scroll_up(5);
        assert_ne!(history.viewport_top, history.first_row());
        assert_ne!(history.viewport_bottom, history.last_row());
        assert_eq!(history.viewport_top_pos(), 11);
        assert_eq!(history.viewport_bottom_pos(), 14);

        // End scrolls the viewport to the last row.
        history.handle_input(Event::Key(KeyEvent::from(KeyCode::End)));
        assert_eq!(history.viewport_bottom, history.last_row());
        assert_eq!(history.viewport_top_pos(), 16);
        assert_eq!(history.viewport_bottom_pos(), 19);

        // Home scrolls the viewport to the first row.
        history.handle_input(Event::Key(KeyEvent::from(KeyCode::Home)));
        assert_eq!(history.viewport_top, history.first_row());
        assert_eq!(history.viewport_top_pos(), 0);
        assert_eq!(history.viewport_bottom_pos(), 3);
    }

    #[test]
    fn test_append() {
        let full = "# Title\n\nfirst\n\nsecond\n\nthird";

        let mut incremental = history(20, 20);
        incremental.add_item(HistoryItemType::Response, "# Title".into());
        update_item(&mut incremental, 0, "\n\nfirst");
        update_item(&mut incremental, 0, "\n\nsecond");
        update_item(&mut incremental, 0, "\n\nthird");
        assert_eq!(incremental.num_rows(), 8);

        let mut whole = history(20, 20);
        whole.add_item(HistoryItemType::Response, full.into());

        assert_eq!(render_draw(&incremental), render_draw(&whole));
    }

    #[test]
    fn test_history_item_query() {
        let item = HistoryItem {
            content: "hello".into(),
            ty: HistoryItemType::Response,
            resume_point: ResumePoint { offset: 0, row: 0 },
            item_id: None,
            first_row: Id::null(),
            last_row: Id::null(),
            num_rows: 0,
        };
        let expected = json!({
            "content": "hello",
            "ty": "response",
            "resume_point": item.resume_point.query("/").unwrap(),
            "item_id": null,
            "first_row": item.first_row.to_json(),
            "last_row": item.last_row.to_json(),
            "num_rows": 0,
        });
        assert_eq!(item.query("/").unwrap(), expected);
        assert_eq!(item.query("/content").unwrap(), json!("hello"));
        assert_eq!(item.query("/ty").unwrap(), json!("response"));
        assert_eq!(
            item.query("/resume_point").unwrap(),
            item.resume_point.query("/").unwrap()
        );
        assert_eq!(item.query("/item_id").unwrap(), json!(null));
        assert_eq!(item.query("/first_row").unwrap(), item.first_row.to_json());
        assert_eq!(item.query("/last_row").unwrap(), item.last_row.to_json());
        assert_eq!(item.query("/num_rows").unwrap(), json!(0));
    }

    #[test]
    fn test_history_query() {
        let history = history(20, 5);
        let expected = json!({
            "num_rows": 0,
            "items": [],
            "width": 20,
            "max_height": 5,
            "head": history.head.to_json(),
            "viewport_top": history.viewport_top.to_json(),
            "viewport_top_pos": 0,
            "viewport_bottom": history.viewport_bottom.to_json(),
            "viewport_bottom_pos": 0,
            "by_item_id": {},
        });
        assert_eq!(history.query("/").unwrap(), expected);
        assert_eq!(history.query("/num_rows").unwrap(), json!(0));
        assert_eq!(history.query("/items").unwrap(), json!([]));
        assert_eq!(history.query("/width").unwrap(), json!(20));
        assert_eq!(history.query("/max_height").unwrap(), json!(5));
        assert_eq!(history.query("/head").unwrap(), history.head.to_json());
        assert_eq!(
            history.query("/viewport_top").unwrap(),
            history.viewport_top.to_json()
        );
        assert_eq!(history.query("/viewport_top_pos").unwrap(), json!(0));
        assert_eq!(
            history.query("/viewport_bottom").unwrap(),
            history.viewport_bottom.to_json()
        );
        assert_eq!(history.query("/viewport_bottom_pos").unwrap(), json!(0));
        assert_eq!(history.query("/by_item_id").unwrap(), json!({}));
    }
}
