//! Scrolling chat history. Data is constructed as a linked list of rows for
//! fast viewport scrolling and rendering.

use std::fmt;

use crossterm::Command;

use crate::arena::{Arena, Id};
use crate::ui::markdown::render_markdown;
use crate::ui::style::Theme;
use crate::ui::Component;

#[derive(Debug)]
pub struct HistoryRow {
    item: Id<HistoryItem>,
    /// Preformatted, pre-padded row contents
    preformatted: String,
    prev: Id<HistoryRow>,
    next: Id<HistoryRow>,
}

impl HistoryRow {
    fn from_preformatted(preformatted: String) -> Self {
        Self {
            item: Id::null(),
            preformatted,
            next: Id::null(),
            prev: Id::null(),
        }
    }
}

#[derive(Debug)]
pub struct HistoryItem {
    markdown: String,
    first_row: Id<HistoryRow>,
    last_row: Id<HistoryRow>,
    num_rows: usize,
}

impl HistoryItem {
    fn from_markdown(
        theme: &'static Theme,
        items: &mut Arena<HistoryItem>,
        rows: &mut Arena<HistoryRow>,
        width: usize,
        md: &str,
    ) -> Id<Self> {
        let item = items.insert(Self {
            markdown: md.to_string(),
            first_row: Id::null(),
            last_row: Id::null(),
            num_rows: 0,
        });

        let mut first_row = Id::null();
        let mut last_row = Id::null();
        let mut num_rows = 0;

        // Render the markdown, then two blank rows of vertical padding.
        let rows_out = render_markdown(theme, width, md)
            .into_iter()
            .chain(std::iter::repeat_with(|| " ".repeat(width)).take(2));

        for row in rows_out {
            let mut history_row = HistoryRow::from_preformatted(row);
            history_row.item = item;
            history_row.prev = last_row;

            let id = rows.insert(history_row);
            if last_row != Id::null() {
                rows[last_row].next = id;
            } else {
                first_row = id;
            }
            last_row = id;
            num_rows += 1;
        }

        items[item].first_row = first_row;
        items[item].last_row = last_row;
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
    viewport_bottom: Id<HistoryRow>,
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
        }
    }

    fn num_rows(&self) -> usize {
        // Subtract header node
        self.rows.len() - 1
    }

    fn first_row(&self) -> Id<HistoryRow> {
        self.rows[self.head].next
    }

    fn last_row(&self) -> Id<HistoryRow> {
        self.rows[self.head].prev
    }

    fn iter_rows<'a>(&'a self) -> HistoryRowIter<'a> {
        HistoryRowIter {
            rows: &self.rows,
            prev: self.head,
            last: self.last_row(),
        }
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
        let mut row = base;
        if offset >= 0 {
            for _ in 0..offset {
                row = self.rows[row].next;
                if row == self.head { return None; }
            }
        } else {
            for _ in 0..-offset {
                row = self.rows[row].prev;
                if row == self.head { return None; }
            }
        }
        Some(row)
    }

    /// Attempts to set the viewport region based on first row.
    fn set_viewport_top(&mut self, viewport_top: Id<HistoryRow>) {
        let prev = self.rows[viewport_top].prev;
        self.viewport_top = viewport_top;
        if let Some(row) = self
            .iter_range(prev, self.last_row())
            .nth(self.max_height - 1)
            .map(|(id, _)| id)
        {
            self.viewport_bottom = row;
        } else {
            self.viewport_bottom = self.last_row();
        }
    }

    /// Attempts to set the viewport region based on last row.
    fn set_viewport_bottom(&mut self, viewport_bottom: Id<HistoryRow>) {
        self.viewport_bottom = viewport_bottom;
        if let Some(row) = self
            .iter_range(self.head, viewport_bottom)
            .rev()
            .nth(self.max_height - 1)
            .map(|(id, _)| id)
        {
            self.viewport_top = row;
        } else {
            self.viewport_top = self.first_row();
        }
    }

    pub fn max_height(&self) -> usize {
        self.max_height
    }

    /// Updates the maximum viewport size, preserving the viewport bottom.
    pub fn set_max_height(&mut self, max_height: usize) {
        if max_height == 0 {
            return;
        }
        self.max_height = max_height;
        self.set_viewport_bottom(self.viewport_bottom);
    }

    /// Updates the wrapping width, re-rendering all markdown items. The
    /// viewport continues to follow the newest messages.
    pub fn set_width(&mut self, width: usize) {
        if width == self.width {
            return;
        }
        self.width = width;

        let markdown: Vec<String> = self
            .item
            .iter()
            .map(|(_, item)| item.markdown.clone())
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

        for md in markdown {
            let item =
                HistoryItem::from_markdown(self.theme, &mut self.item, &mut self.rows, width, &md);
            self.append_item(item);
        }
    }

    /// Links a newly created (unlinked) item's rows into the circular list,
    /// immediately before the head.
    fn append_item(&mut self, item: Id<HistoryItem>) {
        let first = self.item[item].first_row;
        let last = self.item[item].last_row;
        debug_assert_ne!(first, Id::null());
        debug_assert_ne!(last, Id::null());

        let old_last = self.rows[self.head].prev;
        self.rows[old_last].next = first;
        self.rows[first].prev = old_last;
        self.rows[last].next = self.head;
        self.rows[self.head].prev = last;

        self.set_viewport_bottom(self.last_row());
    }

    /// Appends a raw markdown message to the history.
    pub fn markdown_message(&mut self, md: &str) {
        let item = HistoryItem::from_markdown(self.theme, &mut self.item, &mut self.rows, self.width, md);
        self.append_item(item);

        // Follow the newest messages.
        self.set_viewport_bottom(self.last_row());
    }

    fn scroll_up(&mut self, rows: usize) {
        let top = self
            .row_offset(self.viewport_top, -(rows as isize))
            .unwrap_or(self.first_row());
        self.set_viewport_top(top);
    }

    fn scroll_down(&mut self, rows: usize) {
        if let Some(top) = self.row_offset(self.viewport_top, rows as isize) {
            self.set_viewport_top(top);
        } else {
            // Can't scroll any further; anchor to the bottom.
            self.set_viewport_bottom(self.last_row());
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

impl Component for History {
    type Row<'a> = HistoryRowRef<'a> where Self: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where Self: 'a;

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

    fn width(&self) -> usize {
        self.width
    }

    /// Number of actually visible rows.
    fn height(&self) -> usize {
        std::cmp::min(self.max_height, self.num_rows())
    }

    fn cursor_pos(&self) -> (usize, usize) {
        (0, 0)
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

    /// Strips ANSI escape sequences from a row.
    fn strip_ansi(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    fn plain_rows(history: &History) -> Vec<String> {
        history
            .iter_rows()
            .map(|(_, row)| strip_ansi(&row.preformatted).trim_end().to_string())
            .collect()
    }

    #[test]
    fn test_empty_history() {
        let history = history(20, 5);
        assert_eq!(history.num_rows(), 0);
        assert_eq!(history.item.len(), 0);
        assert_eq!(history.viewport_top, history.head);
        assert_eq!(history.viewport_bottom, history.head);
    }

    #[test]
    fn test_markdown_message_rows() {
        let mut history = history(20, 5);
        history.markdown_message("hello world");
        assert_eq!(history.num_rows(), 3);
        assert_eq!(history.item.len(), 1);
        assert_eq!(history.viewport_bottom, history.last_row());
    }

    #[test]
    fn test_markdown_message_wraps() {
        let mut history = history(10, 5);
        history.markdown_message("hello world foo");
        assert_eq!(history.num_rows(), 4);
    }

    #[test]
    fn test_multiline_markdown_message() {
        let mut history = history(80, 5);
        history.markdown_message("hello\nworld");
        assert_eq!(history.num_rows(), 3);
    }

    #[test]
    fn test_set_width() {
        let mut history = history(10, 10);
        history.markdown_message("hello world foo");
        assert_eq!(plain_rows(&history), vec!["hello", "world foo", "", ""]);

        history.set_width(20);
        assert_eq!(history.num_rows(), 3);
        assert_eq!(plain_rows(&history), vec!["hello world foo", "", ""]);

        history.set_width(10);
        assert_eq!(history.num_rows(), 4);
        assert_eq!(history.viewport_bottom, history.last_row());
    }

    #[test]
    fn test_scroll() {
        let mut history = history(80, 4);
        for i in 0..10 {
            history.markdown_message(&format!("message {i}"));
        }
        assert_eq!(history.num_rows(), 30);

        let last = history.last_row();
        let top = history.viewport_top;
        assert_eq!(history.viewport_bottom, last);
        assert_ne!(top, history.first_row());

        history.scroll_up(1);
        assert_ne!(history.viewport_bottom, last);
        assert_ne!(history.viewport_top, top);

        history.scroll_down(1);
        assert_eq!(history.viewport_top, top);
        assert_eq!(history.viewport_bottom, last);

        // Hit bottom
        history.scroll_down(1000);
        assert_eq!(history.viewport_bottom, last);

        // Hit top
        history.scroll_up(1000);
        assert_eq!(history.viewport_top, history.first_row());
    }
}
