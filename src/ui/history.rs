//! Scrolling chat history. Data is constructed as a linked list of rows for
//! fast viewport scrolling and rendering.

use std::fmt;

use crossterm::Command;

use crate::arena::{Arena, Id};
use crate::ui::text::{Row, wrap_line};
use crate::ui::{write_spaces, Component};

#[derive(Debug)]
pub struct HistoryRow {
    item: Id<HistoryItem>,
    /// Width in columns
    width: usize,
    preformatted: String,
    prev: Id<HistoryRow>,
    next: Id<HistoryRow>,
}

impl HistoryRow {
    fn from_row(row: Row) -> Self {
        let num_bytes = row.graphemes.iter().map(|g| g.formatted().len()).sum();
        let mut content = String::with_capacity(num_bytes);
        let mut width = 0;
        for g in &row.graphemes {
            content.push_str(g.formatted());
            width += g.width as usize;
        }
        Self {
            item: Id::null(),
            width,
            preformatted: content,
            next: Id::null(),
            prev: Id::null(),
        }
    }
}

#[derive(Debug)]
pub struct HistoryItem {
    first_row: Id<HistoryRow>,
    last_row: Id<HistoryRow>,
    num_rows: usize,
}

impl HistoryItem {
    fn from_str(
        items: &mut Arena<HistoryItem>,
        rows: &mut Arena<HistoryRow>,
        width: usize,
        s: &str,
    ) -> Id<Self> {
        let item = items.insert(Self {
            first_row: Id::null(),
            last_row: Id::null(),
            num_rows: 0,
        });

        let mut first_row = Id::null();
        let mut last_row = Id::null();
        let mut num_rows = 0;
        for line in s.split('\n') {
            for row in wrap_line(width, line).into_iter() {
                let mut history_row = HistoryRow::from_row(row);
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
    /// Maximum viewport size
    max_height: usize,
    /// Head of circularly linked list. Contains no real data.
    head: Id<HistoryRow>,
    viewport_top: Id<HistoryRow>,
    viewport_bottom: Id<HistoryRow>,
}

impl History {
    pub fn new(width: usize, max_height: usize) -> Self {
        let item = Arena::new();
        let mut rows = Arena::new();

        // Insert dummy head, distinct from all other rows.
        let head = rows.insert(HistoryRow {
            item: Id::null(),
            width: 0,
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

    /// Updates the wrapping width, re-wrapping all items. The viewport
    /// continues to follow the newest messages.
    pub fn set_width(&mut self, width: usize) {
        if width == self.width {
            return;
        }
        self.width = width;

        // Reconstruct the original text of each item. Wrapping never discards
        // characters, so concatenating the wrapped rows restores the source.
        let items: Vec<String> = self
            .item
            .iter()
            .map(|(_, item)| {
                let mut text = String::new();
                let mut row = item.first_row;
                loop {
                    let current = &self.rows[row];
                    text.push_str(&current.preformatted);
                    if row == item.last_row {
                        break;
                    }
                    row = current.next;
                }
                text
            })
            .collect();

        self.item.clear();
        self.rows.clear();

        let head = self.rows.insert(HistoryRow {
            item: Id::null(),
            width: 0,
            preformatted: String::new(),
            prev: Id::null(),
            next: Id::null(),
        });
        self.rows[head].prev = head;
        self.rows[head].next = head;
        self.head = head;
        self.viewport_top = head;
        self.viewport_bottom = head;

        for text in items {
            let item = HistoryItem::from_str(&mut self.item, &mut self.rows, width, &text);
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

    /// Appends a plaintext system message to the history.
    pub fn system_message(&mut self, text: &str) {
        let header = HistoryItem::from_str(&mut self.item, &mut self.rows, self.width, "System");
        self.append_item(header);

        let padding = HistoryItem::from_str(&mut self.item, &mut self.rows, self.width, "");
        self.append_item(padding);

        let content = HistoryItem::from_str(&mut self.item, &mut self.rows, self.width, text);
        self.append_item(content);

        for _ in 0..2 {
            let padding = HistoryItem::from_str(&mut self.item, &mut self.rows, self.width, "");
            self.append_item(padding);
        }

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
    width: usize,
}

impl Command for HistoryRowRef<'_> {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        f.write_str(&self.row.preformatted)?;
        write_spaces(f, self.width.saturating_sub(self.row.width))
    }
}

impl Component for History {
    type Row<'a> = HistoryRowRef<'a> where Self: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where Self: 'a;

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        let prev = self.rows[self.viewport_top].prev;
        let width = self.width;
        Box::new(
            self.iter_range(prev, self.viewport_bottom)
                .map(move |(_, row)| HistoryRowRef { row, width }),
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

    #[test]
    fn test_empty_history() {
        let history = History::new(20, 5);
        assert_eq!(history.num_rows(), 0);
        assert_eq!(history.item.len(), 0);
        assert_eq!(history.viewport_top, history.head);
        assert_eq!(history.viewport_bottom, history.head);
    }

    #[test]
    fn test_system_message_rows() {
        let mut history = History::new(20, 5);
        history.system_message("hello world");
        assert_eq!(history.num_rows(), 5);
        assert_eq!(history.item.len(), 5);
        assert_eq!(history.viewport_bottom, history.last_row());
    }

    #[test]
    fn test_system_message_wraps() {
        let mut history = History::new(10, 5);
        history.system_message("hello world foo");
        assert_eq!(history.num_rows(), 6);
    }

    #[test]
    fn test_multiline_system_message() {
        let mut history = History::new(80, 5);
        history.system_message("hello\nworld");
        assert_eq!(history.num_rows(), 6);
    }

    #[test]
    fn test_set_width() {
        let mut history = History::new(10, 10);
        history.system_message("hello world foo");
        let rows: Vec<String> = history
            .iter_rows()
            .map(|(_, row)| row.preformatted.clone())
            .collect();
        assert_eq!(rows, vec!["System", "", "hello ", "world foo", "", ""]);

        history.set_width(20);
        assert_eq!(history.num_rows(), 5);
        let rows: Vec<String> = history
            .iter_rows()
            .map(|(_, row)| row.preformatted.clone())
            .collect();
        assert_eq!(rows, vec!["System", "", "hello world foo", "", ""]);

        history.set_width(10);
        assert_eq!(history.num_rows(), 6);
        assert_eq!(history.viewport_bottom, history.last_row());
    }

    #[test]
    fn test_scroll() {
        let mut history = History::new(80, 4);
        for i in 0..10 {
            history.system_message(&format!("message {i}"));
        }
        assert_eq!(history.num_rows(), 50);

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
