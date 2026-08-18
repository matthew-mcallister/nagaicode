//! Scrolling chat history. Data is constructed as a linked list of rows for
//! fast viewport scrolling and rendering.

use std::fmt;

use crossterm::Command;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::SetStyle;
use derive_more::From;

use crate::arena::{Arena, Id};
use crate::ui::markdown::{render_markdown, ResumePoint};
use crate::ui::style::Theme;
use crate::ui::Component;
use crate::ui::text::wrap_line;

pub(crate) fn render_help(
    theme: &'static Theme,
    width: usize,
    content: &str,
) -> Vec<String> {
    let mut prefix = String::new();
    let _ = SetStyle(theme.text_quote.into()).write_ansi(&mut prefix);
    prefix.push('▐');
    prefix.push(' ');
    content.lines().flat_map(|line|
        wrap_line(width - 2, line)
            .into_iter()
            .map(|row| format!("{}{}", prefix, row.to_padded_string(width - 2)))
    ).collect()
}

pub(crate) fn render_error(
    theme: &'static Theme,
    width: usize,
    content: &str,
) -> Vec<String> {
    let mut prefix = String::new();
    let _ = SetStyle(theme.text_error.into()).write_ansi(&mut prefix);
    prefix.push('▐');
    prefix.push(' ');
    let _ = SetStyle(theme.text_subtle.into()).write_ansi(&mut prefix);
    content.lines().flat_map(|line|
        wrap_line(width - 2, line)
            .into_iter()
            .map(|row| format!("{}{}", prefix, row.to_padded_string(width - 2)))
    ).collect()
}

#[derive(Debug)]
pub struct HistoryRow {
    item: Id<HistoryItem>,
    /// Preformatted, pre-padded row contents
    preformatted: String,
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
    rendered: Vec<String>,
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

#[derive(Clone, Debug)]
pub enum HistoryItemContent {
    Help(String),
    Error(String),
    Markdown(String),
}

#[derive(Clone, Debug)]
pub enum HistoryItemState {
    Help(String),
    Error(String),
    Markdown {
        content: String,
        resume_point: ResumePoint,
    },
}

impl From<HistoryItemState> for HistoryItemContent {
    fn from(value: HistoryItemState) -> Self {
        match value {
            HistoryItemState::Help(content) => HistoryItemContent::Help(content),
            HistoryItemState::Error(content) => HistoryItemContent::Error(content),
            HistoryItemState::Markdown { content, .. } => HistoryItemContent::Markdown(content),
        }
    }
}

#[derive(Debug)]
pub struct HistoryItem {
    state: HistoryItemState,
    first_row: Id<HistoryRow>,
    last_row: Id<HistoryRow>,
    num_rows: usize,
}

impl HistoryItem {
    fn from_markdown(
        theme: &'static Theme,
        items: &mut Arena<HistoryItem>,
        rows: &mut Arena<HistoryRow>,
        prev: Id<HistoryRow>,
        width: usize,
        md: &str,
    ) -> Id<Self> {
        let result = render_markdown(theme, width, md);
        Self::from_rows(
            items,
            rows,
            prev,
            width,
            HistoryItemState::Markdown {
                content: md.to_string(),
                resume_point: result.resume_point,
            },
            result.rows,
        )
    }

    fn from_help(
        theme: &'static Theme,
        items: &mut Arena<HistoryItem>,
        rows: &mut Arena<HistoryRow>,
        prev: Id<HistoryRow>,
        width: usize,
        content: &str,
    ) -> Id<Self> {
        Self::from_rows(
            items,
            rows,
            prev,
            width,
            HistoryItemState::Help(content.to_string()),
            render_help(theme, width, content),
        )
    }

    fn from_error(
        theme: &'static Theme,
        items: &mut Arena<HistoryItem>,
        rows: &mut Arena<HistoryRow>,
        prev: Id<HistoryRow>,
        width: usize,
        content: &str,
    ) -> Id<Self> {
        Self::from_rows(
            items,
            rows,
            prev,
            width,
            HistoryItemState::Error(content.to_string()),
            render_error(theme, width, content),
        )
    }

    fn from_content(
        theme: &'static Theme,
        items: &mut Arena<HistoryItem>,
        rows: &mut Arena<HistoryRow>,
        prev: Id<HistoryRow>,
        width: usize,
        content: HistoryItemContent,
    ) -> Id<Self> {
        match content {
            HistoryItemContent::Markdown(md) => {
                Self::from_markdown(theme, items, rows, prev, width, &md)
            }
            HistoryItemContent::Help(content) => {
                Self::from_help(theme, items, rows, prev, width, &content)
            }
            HistoryItemContent::Error(content) => {
                Self::from_error(theme, items, rows, prev, width, &content)
            }
        }
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
        let HistoryItemState::Markdown { content, resume_point } = &mut self.state else {
            panic!("can only append to markdown");
        };
        let old_offset = resume_point.offset;
        let old_row = resume_point.row;

        content.push_str(delta);

        let result = render_markdown(theme, width, &content[old_offset..]);

        // Find insertion position
        let steps_back = self.num_rows - 1 - old_row;
        let first = row_offset(rows, head, self.last_row, -(steps_back as isize))
            .expect("resume point out of bounds");
        let prev = rows[first].prev;
        let next = rows[self.last_row].next;
        let item_id = rows[self.first_row].item;

        // Replace rows
        remove_rows(rows, prev, self.num_rows - old_row);

        let mut rendered: Vec<String> = result.rows;
        rendered.push(" ".repeat(width));
        let len = rendered.len();

        insert_rows(rows, item_id, rendered, prev);

        // Update item
        if old_row == 0 {
            self.first_row = rows[prev].next;
        }
        self.last_row = rows[next].prev;
        self.num_rows = old_row + len;

        *resume_point = ResumePoint {
            offset: old_offset + result.resume_point.offset,
            row: old_row + result.resume_point.row,
        };
    }

    fn from_rows(
        items: &mut Arena<HistoryItem>,
        rows: &mut Arena<HistoryRow>,
        prev: Id<HistoryRow>,
        width: usize,
        state: HistoryItemState,
        mut rendered: Vec<String>,
    ) -> Id<Self> {
        let next = rows[prev].next;

        let item = items.insert(Self {
            state,
            first_row: Id::null(),
            last_row: Id::null(),
            num_rows: 0,
        });

        rendered.push(" ".repeat(width));   // Add vertical padding row
        let num_rows = rendered.len();
        insert_rows(rows, item, rendered, prev);

        items[item].first_row = rows[prev].next;
        items[item].last_row = rows[next].prev;
        items[item].num_rows = num_rows;

        item
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
}

impl History {
    pub fn new(width: usize, max_height: usize, theme: &'static Theme) -> Self {
        let item = Arena::new();
        let mut rows = Arena::new();

        // Insert dummy head, distinct from all other rows.
        let head = rows.insert(HistoryRow {
            item: Id::null(),
            preformatted: String::new(),
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
        }
    }

    pub fn num_rows(&self) -> usize {
        // Subtract header node
        self.rows.len() - 1
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

        let states: Vec<HistoryItemState> = self
            .item
            .iter()
            .map(|(_, item)| item.state.clone())
            .collect();

        self.item.clear();
        self.rows.clear();

        let head = self.rows.insert(HistoryRow {
            item: Id::null(),
            preformatted: String::new(),
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

        for state in states {
            let prev = self.last_row();
            HistoryItem::from_content(
                self.theme,
                &mut self.item,
                &mut self.rows,
                prev,
                width,
                state.into(),
            );
            self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
        }
    }

    /// Appends an item to the history.
    pub fn add_item(&mut self, content: HistoryItemContent) {
        let prev = self.last_row();
        HistoryItem::from_content(
            self.theme,
            &mut self.item,
            &mut self.rows,
            prev,
            self.width,
            content,
        );
        self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1);
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

/// A single drawable row of the history.
#[derive(Debug)]
pub struct HistoryRowRef<'a> {
    row: &'a HistoryRow,
}

impl Command for HistoryRowRef<'_> {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str(&self.row.preformatted)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, From)]
pub enum InEvent {
    Input(Event),
}

impl Component for History {
    type Row<'a> = HistoryRowRef<'a> where Self: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where Self: 'a;
    type InEvent = InEvent;
    type OutEvent = ();

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        let prev = self.rows[self.viewport_top].prev;
        Box::new(
            self.iter_range(prev, self.viewport_bottom)
                .map(|(_, row)| HistoryRowRef { row }),
        )
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

    fn handle_event(&mut self, event: Self::InEvent) -> Self::OutEvent {
        let InEvent::Input(event) = event;
        let KeyEvent { code, modifiers, .. } = match event {
            Event::Key(key) => key,
            _ => return,
        };
        let alt = modifiers.contains(KeyModifiers::ALT);
        match (code, alt) {
            (KeyCode::Up, _) => self.scroll_up(1),
            (KeyCode::Down, _) => self.scroll_down(1),
            (KeyCode::PageUp, _) | (KeyCode::Char('u'), true) => self.scroll_up(self.height() / 2),
            (KeyCode::PageDown, _) | (KeyCode::Char('d'), true) => self.scroll_down(self.height() / 2),
            (KeyCode::Home, _) => self.set_viewport_top_at(self.first_row(), 0),
            (KeyCode::End, _) => self.set_viewport_bottom_at(self.last_row(), self.num_rows() - 1),
            _ => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ui::style::THEME_DARK;

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

    fn render_history(history: &History) -> String {
        history
            .iter_range(history.head, history.last_row())
            .map(|(_, row)| row.preformatted.clone())
            .collect::<Vec<_>>()
            .join("\n")
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
        use crossterm::Command;
        use crossterm::style::SetStyle;

        fn render(content: &str, width: usize) -> String {
            let mut lines = super::render_help(&THEME_DARK, width, content);

            // In tests, strip the style initialization commands for
            // readability.
            let mut prefix = String::new();
            let _ = SetStyle(THEME_DARK.text_quote.into()).write_ansi(&mut prefix);
            for line in lines.iter_mut() {
                *line = line.trim_start_matches(&prefix).to_owned();
            }

            lines.join("\n")
        }

        assert_eq!(render("hello", 10), "▐ hello   ");
        assert_eq!(render("foo\nbar", 8), "▐ foo   \n▐ bar   ");
        assert_eq!(render("hello world", 8), "▐ hello \n▐ world ");
        assert_eq!(render("", 6), "");
    }

    #[test]
    fn test_render_error() {
        use crossterm::Command;
        use crossterm::style::SetStyle;

        fn render(content: &str, width: usize) -> String {
            let mut lines = super::render_error(&THEME_DARK, width, content);

            let mut prefix = String::new();
            let _ = SetStyle(THEME_DARK.text_error.into()).write_ansi(&mut prefix);
            prefix.push('▐');
            prefix.push(' ');
            let _ = SetStyle(THEME_DARK.text_subtle.into()).write_ansi(&mut prefix);
            for line in lines.iter_mut() {
                *line = line.trim_start_matches(&prefix).to_owned();
            }

            lines.join("\n")
        }

        assert_eq!(render("hello", 10), "hello   ");
        assert_eq!(render("foo\nbar", 8), "foo   \nbar   ");
        assert_eq!(render("hello world", 8), "hello \nworld ");
        assert_eq!(render("", 6), "");
    }

    #[test]
    fn test_scroll() {
        let mut history = history(80, 4);
        for i in 0..10 {
            history.add_item(HistoryItemContent::Markdown(format!("message {i}")));
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
            history.add_item(HistoryItemContent::Markdown(format!("message {i}")));
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
            history.add_item(HistoryItemContent::Markdown(format!("message {i}")));
        }

        // Start at the bottom; scroll up so we're not at either extreme.
        history.scroll_up(5);
        assert_ne!(history.viewport_top, history.first_row());
        assert_ne!(history.viewport_bottom, history.last_row());
        assert_eq!(history.viewport_top_pos(), 11);
        assert_eq!(history.viewport_bottom_pos(), 14);

        // End scrolls the viewport to the last row.
        history.handle_event(Event::Key(KeyEvent::from(KeyCode::End)).into());
        assert_eq!(history.viewport_bottom, history.last_row());
        assert_eq!(history.viewport_top_pos(), 16);
        assert_eq!(history.viewport_bottom_pos(), 19);

        // Home scrolls the viewport to the first row.
        history.handle_event(Event::Key(KeyEvent::from(KeyCode::Home)).into());
        assert_eq!(history.viewport_top, history.first_row());
        assert_eq!(history.viewport_top_pos(), 0);
        assert_eq!(history.viewport_bottom_pos(), 3);
    }

    #[test]
    fn test_append() {
        let full = "# Title\n\nfirst\n\nsecond\n\nthird";

        let mut incremental = history(20, 20);
        incremental.add_item(HistoryItemContent::Markdown("# Title".into()));
        update_item(&mut incremental, 0, "\n\nfirst");
        update_item(&mut incremental, 0, "\n\nsecond");
        update_item(&mut incremental, 0, "\n\nthird");
        assert_eq!(incremental.num_rows(), 8);

        let mut whole = history(20, 20);
        whole.add_item(HistoryItemContent::Markdown(full.into()));

        assert_eq!(render_history(&incremental), render_history(&whole));
    }
}
