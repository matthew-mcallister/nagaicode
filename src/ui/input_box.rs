//! Input text box. Data structures are similar to @src/ui/history.rs but more
//! complex.
// FIXME: This component has the dubious distinction of having variable height.
// Maybe it's overkill but I kind of would prefer to stick to a a strict
// parent-controls-height model and had make StackedView explicitly resize the
// InputBox to make it grow/shrink when rows are added. This is how variable-
// size textboxes are handled using HTML + JavaScript; CSS alone can't do it
// correctly.


use compact_str::CompactString;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;

use crate::app::AppEvent;
use crate::arena::{Arena, Id};
use crate::query::{DataQuery, QueryError, QueryField, ToJson};
use crate::ui::canvas::Canvas;
use crate::ui::text::{Row, SPACES, strip_cr, wrap_line};
use crate::ui::Component;

/// A pair `(row_id, grapheme_index)` pointing to the location of a grapheme.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
struct GraphemePos(Id<InputRow>, usize);

impl GraphemePos {
    fn row(self) -> Id<InputRow> {
        self.0
    }

    fn grapheme(self) -> usize {
        self.1
    }
}

#[derive(Debug)]
struct InputLine {
    first_row: Id<InputRow>,
    last_row: Id<InputRow>,
    num_rows: usize,
}

impl InputLine {
    /// Constructs an input line from a raw string. String must not contain
    /// line breaks. First and last row will have to be linked in.
    fn from_str(
        lines: &mut Arena<InputLine>,
        rows: &mut Arena<InputRow>,
        width: usize,
        s: &str,
    ) -> Id<Self> {
        debug_assert!(!s.contains('\n'), "unexpected newline");

        let line = lines.insert(Self {
            first_row: Id::null(),
            last_row: Id::null(),
            num_rows: 0,
        });

        let wrapped = wrap_line(width, s);
        let mut first_row = Id::null();
        let mut last_row = Id::null();
        let mut num_rows = 0;
        for row in wrapped.into_iter() {
            let mut input_row = InputRow::from_row(row);
            input_row.line = line;
            input_row.prev = last_row;

            let id = rows.insert(input_row);
            if last_row != Id::null() {
                rows[last_row].next = id;
            } else {
                first_row = id;
            }
            last_row = id;
            num_rows += 1;
        }

        lines[line].first_row = first_row;
        lines[line].last_row = last_row;
        lines[line].num_rows = num_rows;

        line
    }
}

#[derive(Debug)]
struct InputGrapheme {
    data: CompactString,
    width: u8,
    column: u16,
}

impl InputGrapheme {
    fn is_alphanumeric(&self) -> bool {
        self.data.chars().any(|c| c.is_alphanumeric())
    }
}

/// Visual/word-wrapped row
#[derive(Debug)]
struct InputRow {
    /// Line to which row belongs
    line: Id<InputLine>,
    /// Next visual row
    next: Id<InputRow>,
    /// Previous visual row
    prev: Id<InputRow>,
    /// Graphemes. The final row of a line ends with a zero-width newline
    /// grapheme.
    // XXX: This is usually immutable so can probably replace with Box<[...]>
    graphemes: Vec<InputGrapheme>,
    /// Width in columns
    width: usize,
    /// Text for rendering
    preformatted: String,
}

impl std::fmt::Display for InputRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.preformatted)
    }
}

impl InputRow {
    fn new() -> Self {
        Self {
            line: Id::null(),
            next: Id::null(),
            prev: Id::null(),
            // Guarantee all rows, even head, have non-empty graphemes
            graphemes: vec![InputGrapheme { data: "\n".into(), column: 0, width: 0 }],
            width: 0,
            preformatted: String::new(),
        }
    }

    fn from_row(row: Row) -> Self {
        let mut graphemes = Vec::with_capacity(row.graphemes.len());
        let mut column: u16 = 0;
        let mut preformatted = String::new();
        for g in row.graphemes {
            preformatted.push_str(g.formatted());
            let width = g.width;
            graphemes.push(InputGrapheme {
                data: g.data,
                width,
                column,
            });
            column += width as u16;
        }
        InputRow {
            line: Id::null(),
            next: Id::null(),
            prev: Id::null(),
            graphemes,
            width: column as usize,
            preformatted,
        }
    }

    /// Finds the index of the grapheme at a given column within a row. If the
    /// column is past the end, returns the final visitable grapheme in the
    /// row. (The zero-width newline is never matched directly, since it
    /// occupies no columns.)
    fn grapheme_at_col(&self, col: usize) -> usize {
        if col >= self.width {
            self.graphemes.len() - 1
        } else {
            self.graphemes
                .iter()
                .position(|grapheme| col < grapheme.column as usize + grapheme.width as usize)
                .unwrap()
        }
    }
}

#[derive(Debug)]
pub struct InputBox {
    lines: Arena<InputLine>,
    rows: Arena<InputRow>,
    width: usize,
    /// Maximum viewport size
    max_height: usize,
    /// Head of circularly linked list. Contains no real data.
    head: Id<InputRow>,
    viewport_top: Id<InputRow>,
    /// Absolute row index of `viewport_top`
    viewport_top_pos: usize,
    viewport_bottom: Id<InputRow>,
    /// Absolute row index of `viewport_bottom`
    viewport_bottom_pos: usize,
    cursor_row: Id<InputRow>,
    cursor_col: usize,
    buffer: String,
    overwrite_buffer: bool,
}

impl InputBox {
    pub fn new(width: usize, max_height: usize) -> Self {
        let mut lines = Arena::new();
        let mut rows = Arena::new();

        // Insert dummy head, distinct from all other rows.
        let head = rows.insert(InputRow::new());

        // Insert empty first line/row
        let line = lines.insert(InputLine {
            first_row: Id::null(),
            last_row: Id::null(),
            num_rows: 1,
        });
        let first = rows.insert(InputRow {
            line,
            ..InputRow::new()
        });

        rows[head].next = first;
        rows[head].prev = first;
        rows[first].next = head;
        rows[first].prev = head;
        lines[line].first_row = first;
        lines[line].last_row = first;

        Self {
            lines,
            rows,
            width,
            max_height,
            head,
            viewport_top: first,
            viewport_bottom: first,
            viewport_top_pos: 0,
            viewport_bottom_pos: 0,
            cursor_row: first,
            cursor_col: 0,
            buffer: String::new(),
            overwrite_buffer: false,
        }
    }

    pub fn num_rows(&self) -> usize {
        // Subtract header node
        self.rows.len() - 1
    }

    #[allow(dead_code)]
    pub fn num_lines(&self) -> usize {
        self.lines.len()
    }

    /// Returns true if the first character of the input is `c`.
    pub fn starts_with(&self, c: char) -> bool {
        self.iter_graphemes(self.grapheme_start(), self.grapheme_end())
            .next()
            .is_some_and(|(_, g)| g.data.starts_with(c))
    }

    /// Returns true if the input begins with a special command prefix
    /// (`!` or `/`).
    pub fn is_special_command(&self) -> bool {
        self.starts_with('!') || self.starts_with('/')
    }

    fn first_row(&self) -> Id<InputRow> {
        self.rows[self.head].next
    }

    fn last_row(&self) -> Id<InputRow> {
        self.rows[self.head].prev
    }

    /// Iterate over a range of rows. `prev` is not inclusive; `last` is
    /// inclusive.
    fn iter_range<'a>(&'a self, prev: Id<InputRow>, last: Id<InputRow>) -> InputRowIter<'a> {
        InputRowIter { input: self, prev, last }
    }

    fn iter_rows<'a>(&'a self) -> InputRowIter<'a> {
        self.iter_range(self.head, self.last_row())
    }

    #[allow(dead_code)]
    fn iter_line<'a>(&'a self, line: Id<InputLine>) -> InputRowIter<'a> {
        let line = &self.lines[line];
        let prev = self.rows[line.first_row].prev;
        let last = line.last_row;
        self.iter_range(prev, last)
    }

    /// Iterates over graphemes between two arbitrary points. `end` is not
    /// included in the iterator range.
    fn iter_graphemes<'a>(&'a self, start: GraphemePos, end: GraphemePos) -> InputGraphemeIter<'a> {
        InputGraphemeIter::new(self, start, end)
    }

    /// The grapheme position of the cursor.
    fn cursor_pos(&self) -> GraphemePos {
        let index = self.rows[self.cursor_row].grapheme_at_col(self.cursor_col);
        GraphemePos(self.cursor_row, index)
    }

    /// O(n) row lookup relative to base row. Returns None if the offset is
    /// out of bounds.
    fn row_offset(&self, base: Id<InputRow>, offset: isize) -> Option<Id<InputRow>> {
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

    /// O(n) row distance relative to base. base must come before other.
    /// Unspecified result if base comes after other.
    fn row_diff(&self, base: Id<InputRow>, other: Id<InputRow>) -> isize {
        let mut row = base;
        let mut diff = 0;
        while row != other {
            row = self.rows[row].next;
            diff += 1;
        }
        diff
    }

    /// Returns the line after `line`. Result is unspecified if `line` is the
    /// last line in the input.
    fn next_line(&self, line: Id<InputLine>) -> Id<InputLine> {
        self.rows[self.rows[self.lines[line].last_row].next].line
    }

    /// Attempts to set the viewport region based on first row. `pos` is the
    /// row index of the new top row.
    fn set_viewport_top_at(&mut self, viewport_top: Id<InputRow>, pos: usize) {
        let prev = self.rows[viewport_top].prev;
        self.viewport_top = viewport_top;
        self.viewport_top_pos = pos;
        if let Some(row) = self
            .iter_range(prev, self.last_row())
            .nth(self.max_height - 1)
            .map(|(id, _)| id)
        {
            self.viewport_bottom = row;
            self.viewport_bottom_pos = pos + self.max_height - 1;
        } else if self.viewport_top == self.first_row() {
            // Viewport covers entire text
            self.viewport_bottom = self.last_row();
            self.viewport_bottom_pos = self.num_rows() - 1;
        } else {
            let last = self.last_row();
            let last_pos = self.num_rows() - 1;
            self.set_viewport_bottom_at(last, last_pos);
        }
    }

    /// Attempts to set the viewport region based on last row. `pos` is the
    /// row index of the new bottom row.
    fn set_viewport_bottom_at(&mut self, viewport_bottom: Id<InputRow>, pos: usize) {
        self.viewport_bottom = viewport_bottom;
        self.viewport_bottom_pos = pos;
        if let Some(row) = self
            .iter_range(self.head, self.viewport_bottom)
            .rev()
            .nth(self.max_height - 1)
            .map(|(id, _)| id)
        {
            self.viewport_top = row;
            self.viewport_top_pos = pos - (self.max_height - 1);
        } else if self.viewport_bottom == self.last_row() {
            // Viewport covers entire text
            self.viewport_top = self.first_row();
            self.viewport_top_pos = 0;
        } else {
            let first = self.first_row();
            self.set_viewport_top_at(first, 0);
        }
    }

    /// Slightly inefficient helper for tests
    #[cfg(test)]
    fn set_viewport_top(&mut self, viewport_top: Id<InputRow>) {
        let pos = self.row_diff(self.first_row(), viewport_top) as usize;
        self.set_viewport_top_at(viewport_top, pos);
    }

    /// Recomputes the viewport after moving the cursor. Tries to keep
    /// `MARGIN_ROWS` rows between the cursor and the viewport edges. Cursor
    /// position and previous viewport bounds must be known.
    fn recompute_viewport(
        &mut self,
        base: Id<InputRow>,
        base_pos: isize,
        cursor: isize,
        prev_top: isize,
        prev_bottom: isize,
    ) {
        let margin = Self::MARGIN_ROWS;
        let cursor_abs = base_pos + cursor;
        if cursor > prev_top + margin {
            let (bottom, pos) = match self.row_offset(self.cursor_row, margin) {
                Some(row) => (row, (cursor_abs + margin) as usize),
                None => (self.last_row(), self.num_rows() - 1),
            };
            self.set_viewport_bottom_at(bottom, pos);
        } else if cursor < prev_bottom - margin {
            let (top, pos) = match self.row_offset(self.cursor_row, -margin) {
                Some(row) => (row, (cursor_abs - margin) as usize),
                None => (self.first_row(), 0),
            };
            self.set_viewport_top_at(top, pos);
        } else {
            let (bottom, pos) = match self.row_offset(base, prev_bottom) {
                Some(row) => (row, (base_pos + prev_bottom) as usize),
                None => (self.last_row(), self.num_rows() - 1),
            };
            self.set_viewport_bottom_at(bottom, pos);
        }
    }

    /// Computes text of all lines.
    pub fn get_text(&self) -> String {
        let mut out = String::new();
        for (_, row) in self.iter_rows() {
            for g in &row.graphemes {
                out.push_str(&g.data);
            }
        }
        out
    }

    /// Unlinks a line and frees resources.
    fn remove_line(&mut self, line: Id<InputLine>) {
        let first = self.lines[line].first_row;
        let last = self.lines[line].last_row;
        let num_rows = self.lines[line].num_rows;

        let prev = self.rows[first].prev;
        let next = self.rows[last].next;
        self.rows[prev].next = next;
        self.rows[next].prev = prev;

        let mut cur = first;
        for _ in 0..num_rows {
            let next_in_line = self.rows[cur].next;
            self.rows.remove(cur);
            cur = next_in_line;
        }

        self.lines.remove(line);
    }

    /// Links an unlinked line.
    fn link_line(&mut self, prev: Id<InputRow>, line: Id<InputLine>) {
        let line_first = self.lines[line].first_row;
        let line_last = self.lines[line].last_row;

        let next = self.rows[prev].next;
        self.rows[prev].next = line_first;
        self.rows[line_first].prev = prev;
        self.rows[line_last].next = next;
        self.rows[next].prev = line_last;
    }

    /// Inserts arbitrary text as new lines at a given position. The text may
    /// have line breaks; each line break will insert an extra line.
    fn insert_text(&mut self, mut prev: Id<InputRow>, text: &str) {
        for line_text in text.split('\n').map(strip_cr) {
            let line = InputLine::from_str(&mut self.lines, &mut self.rows, self.width, line_text);
            self.link_line(prev, line);
            prev = self.lines[line].last_row;
        }
    }

    pub fn set_text(&mut self, text: &str) {
        // Remove all existing lines.
        while self.rows[self.head].next != self.head {
            let first = self.rows[self.head].next;
            let line = self.rows[first].line;
            self.remove_line(line);
        }

        let mut text = text;
        if text.ends_with('\n') {
            text = &text[..text.len() - 1];
        }
        self.insert_text(self.head, text);
        self.set_viewport_top_at(self.first_row(), 0);
        self.cursor_row = self.first_row();
        self.cursor_col = 0;
        self.buffer.clear();
        self.overwrite_buffer = false;
    }

    /// Updates the wrapping width, re-wrapping all existing text. The cursor
    /// is restored to the same byte offset.
    pub fn set_width(&mut self, width: usize) {
        if width == self.width {
            return;
        }

        let text = self.get_text();
        let cursor_index = self
            .iter_graphemes(GraphemePos(self.first_row(), 0), self.cursor_pos())
            .count();
        let cursor_offset_in_viewport = self.row_diff(self.viewport_top, self.cursor_row);

        self.width = width;
        self.set_text(&text);

        // Restore the cursor to the same grapheme index.
        let end = GraphemePos(self.last_row(), self.rows[self.last_row()].graphemes.len());
        let cursor = self
            .iter_graphemes(GraphemePos(self.first_row(), 0), end)
            .nth(cursor_index)
            .map(|(pos, g)| (pos.row(), g.column as usize));
        if let Some((row, col)) = cursor {
            self.cursor_row = row;
            self.cursor_col = col;
        }

        self.fit_viewport_on_resize(cursor_offset_in_viewport as usize);
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

    #[allow(dead_code)]
    /// Updates the maximum viewport size
    pub fn set_max_height(&mut self, max_height: usize) {
        assert!(max_height > 0);
        self.max_height = max_height;
        self.fit_viewport_on_resize(self.row_diff(self.viewport_top, self.cursor_row) as usize);
    }

    /// Repositions the viewport to keep the cursor in the desired row, as long
    /// as it is possible to do so. `cursor_row` is the desired offset of the
    /// cursor within the viewport.
    fn fit_viewport_on_resize(&mut self, cursor_row: usize) {
        let k = cursor_row.min(self.max_height - 1);

        let cursor_pos =
            self.viewport_top_pos + self.row_diff(self.viewport_top, self.cursor_row) as usize;

        // Scan up k rows and check for the top
        let first = match self.row_offset(self.cursor_row, -(k as isize)) {
            Some(first) => first,
            None => {
                self.set_viewport_top_at(self.first_row(), 0);
                return;
            }
        };

        // Scan down max_height - k rows and check for the bottom
        let down = self.max_height - 1 - k;
        if self.row_offset(self.cursor_row, down as isize).is_none() {
            let last = self.last_row();
            let last_pos = self.num_rows() - 1;
            self.set_viewport_bottom_at(last, last_pos);
            return;
        }

        self.set_viewport_top_at(first, cursor_pos - k);
    }

    /// Deletes a range of graphemes and replaces them with new text. If
    /// `inserted_text` is empty, graphemes will still be deleted without
    /// inserting any new text. `start` is inclusive while `end` is not: `end`
    /// may point one past the last grapheme. If the deletion range contains a
    /// trailing newline, the newline will be deleted and consecutive lines
    /// merged.
    ///
    /// The cursor will be placed after the inserted/deleted text and the
    /// viewport updated appropriately.
    fn splice(&mut self, start: GraphemePos, end: GraphemePos, inserted_text: &str) {
        let start_line = self.rows[start.row()].line;
        let end_line = self.rows[end.row()].line;
        let prev = self.rows[self.lines[start_line].first_row].prev;
        let next = self.rows[self.lines[end_line].last_row].next;

        let viewport_bottom_offset = self.row_diff(prev, self.viewport_bottom);
        let base_pos = self.viewport_bottom_pos as isize - viewport_bottom_offset;
        // FIXME: We assume viewport at least partially intersects splice range
        // but it shouldn't be necessary
        debug_assert!(base_pos >= -1, "splice outside viewport bounds");

        // Cursor byte offset relative to end of last line.
        //
        // Why byte offset? In some cases, pasted codepoints will alter the
        // grapheme segmentation of the rest of the line. This ensures the
        // cursor still points to the same codepoint it did previously.
        let cursor_offset: usize = self.iter_graphemes(end, GraphemePos(next, 0))
            .map(|(_, g)| g.data.len())
            .sum();

        // Splice strings
        let mut out = String::new();
        out.extend(
            self.iter_graphemes(GraphemePos(self.lines[start_line].first_row, 0), start)
                .map(|(_, g)| &g.data[..])
        );
        out.push_str(inserted_text);
        out.extend(
            self.iter_graphemes(end, GraphemePos(next, 0))
                .map(|(_, g)| &g.data[..])
        );

        // Remove trailing newline
        if out.ends_with('\n') { out.pop(); }

        // Delete all affected lines
        let mut line = start_line;
        loop {
            let next_line = self.next_line(line);
            self.remove_line(line);
            if line == end_line { break; }
            line = next_line;
        }

        // Insert new text
        self.insert_text(prev, &out);

        // Recompute cursor position
        let mut bytes = 0;
        let (pos, g) = self.iter_graphemes(GraphemePos(self.first_row(), 0), GraphemePos(next, 0))
            .rev()
            .find(|(_, g)| {
                bytes += g.data.len();
                bytes >= cursor_offset
            })
            .unwrap();
        self.cursor_col = g.column as _;
        self.cursor_row = pos.row();

        let cursor_pos = self.row_diff(prev, self.cursor_row);
        self.recompute_viewport(
            prev,
            base_pos,
            cursor_pos,
            viewport_bottom_offset - self.max_height as isize,
            viewport_bottom_offset,
        );
    }

    /// Appends all text to the end of the buffer. If the overwrite flag is
    /// set, overwrites the buffer and resets the flag.
    fn buffer_append(&mut self, start: GraphemePos, end: GraphemePos) {
        if self.overwrite_buffer {
            self.overwrite_buffer = false;
            self.buffer.clear();
        }
        let mut buffer = std::mem::take(&mut self.buffer); // Memory micro optimization
        buffer.extend(self.iter_graphemes(start, end).map(|(_, g)| &g.data[..]));
        self.buffer = buffer;
    }

    /// Prepends all text to the beginning of the buffer. If the overwrite flag is
    /// set, overwrites the buffer and resets the flag.
    fn buffer_prepend(&mut self, start: GraphemePos, end: GraphemePos) {
        let text: String = self
            .iter_graphemes(start, end)
            .map(|(_, g)| &g.data[..])
            .collect();
        if self.overwrite_buffer {
            self.buffer = text;
            self.overwrite_buffer = false;
        } else {
            let mut combined = text;
            combined.push_str(&self.buffer);
            self.buffer = combined;
        }
    }

    /// Pastes raw text at the cursor position.
    pub fn paste(&mut self, pasted_text: &str) {
        if pasted_text.is_empty() { return; }
        let pos = self.cursor_pos();
        self.splice(pos, pos, pasted_text);
    }

    /// Pastes the contents of the buffer at the cursor position.
    pub fn paste_buffer(&mut self) {
        let text = std::mem::take(&mut self.buffer);
        if text.is_empty() { return; }
        let pos = self.cursor_pos();
        self.splice(pos, pos, &text);
    }

    // FIXME: Make this configurable, and handle 0 specifically
    const MARGIN_ROWS: isize = 3;

    /// Moves the cursor up one or more rows. Preserves the column of the
    /// cursor. If already at the very first row, moves to the start of the
    /// line. Scrolls the viewport if at the top. Tries to keep some rows
    /// between the cursor and viewport edge.
    pub fn move_up(&mut self, rows: usize) {
        self.overwrite_buffer = true;
        let margin = self.row_diff(self.viewport_top, self.cursor_row);
        let mut moved = 0;
        for _ in 0..rows {
            if self.cursor_row == self.first_row() {
                self.cursor_col = 0;
                break;
            }
            self.cursor_row = self.rows[self.cursor_row].prev;
            moved += 1;
        }

        let margin = margin - moved as isize;
        if margin < Self::MARGIN_ROWS {
            let offset = margin - Self::MARGIN_ROWS;
            let (new_top, pos) = match self.row_offset(self.viewport_top, offset) {
                Some(row) => (row, (self.viewport_top_pos as isize + offset) as usize),
                None => (self.first_row(), 0),
            };
            self.set_viewport_top_at(new_top, pos);
        }
    }

    /// Moves the cursor down one or more rows. Preserves the column of the
    /// cursor. If already at the very last row, moves to the end of the line.
    /// Scrolls the viewport if at the bottom. Tries to keep some rows between
    /// the cursor and viewport edge.
    pub fn move_down(&mut self, rows: usize) {
        self.overwrite_buffer = true;
        let margin = self.row_diff(self.cursor_row, self.viewport_bottom);
        let mut moved = 0;
        for _ in 0..rows {
            if self.cursor_row == self.last_row() {
                self.cursor_col = self.rows[self.cursor_row].width;
                break;
            }
            self.cursor_row = self.rows[self.cursor_row].next;
            moved += 1;
        }

        let margin = margin - moved as isize;
        if margin < Self::MARGIN_ROWS {
            let offset = Self::MARGIN_ROWS - margin;
            let (new_bottom, pos) = match self.row_offset(self.viewport_bottom, offset) {
                Some(row) => (row, (self.viewport_bottom_pos as isize + offset) as usize),
                None => (self.last_row(), self.num_rows() - 1),
            };
            self.set_viewport_bottom_at(new_bottom, pos);
        }
    }

    /// Moves the cursor left by one grapheme. If at the start of a row, goes
    /// to the end of the previous row.
    pub fn move_left(&mut self) {
        self.overwrite_buffer = true;
        let pos = self.cursor_pos();
        if let Some((prev, g)) =
            self.iter_graphemes(GraphemePos(self.first_row(), 0), pos).next_back()
        {
            let col = g.column;
            self.cursor_row = prev.row();
            self.cursor_col = col as usize;
        }
    }

    /// Moves the cursor right by one grapheme. If at the end of a row, goes to
    /// the start of the next row.
    pub fn move_right(&mut self) {
        self.overwrite_buffer = true;
        let pos = self.cursor_pos();
        let end = GraphemePos(self.last_row(), self.rows[self.last_row()].graphemes.len());
        if let Some((next, g)) = self.iter_graphemes(pos, end).nth(1) {
            let col = g.column;
            self.cursor_row = next.row();
            self.cursor_col = col as usize;
        }
    }

    /// Deletes the grapheme under the cursor. Does nothing if the cursor is
    /// on the last grapheme of the last line.
    pub fn delete(&mut self) {
        let last = self.last_row();
        let last_len = self.rows[last].graphemes.len();
        let start = self.cursor_pos();
        if let Some((end_pos, _)) =
            self.iter_graphemes(start, GraphemePos(last, last_len)).nth(1)
        {
            self.splice(start, end_pos, "");
        }
    }

    /// Deletes the grapheme preceding the one under the cursor. Does nothing
    /// if the cursor is on the first grapheme of the first line.
    pub fn backspace(&mut self) {
        let pos = self.cursor_pos();
        if let Some((prev_pos, _)) =
            self.iter_graphemes(GraphemePos(self.first_row(), 0), pos).next_back()
        {
            self.splice(prev_pos, pos, "");
        }
    }

    /// Moves the cursor to the beginning of the current logical line. If
    /// already at the beginning of the line, moves back one character.
    pub fn go_to_line_start(&mut self) {
        self.overwrite_buffer = true;
        let line = self.rows[self.cursor_row].line;
        let first_row = self.lines[line].first_row;
        if self.cursor_row == first_row && self.cursor_col == 0 {
            self.move_left();
        } else {
            self.cursor_row = first_row;
            self.cursor_col = 0;
        }
    }

    /// Moves the cursor to the end of the current logical line. If already at
    /// the end of the line, moves forward one character.
    pub fn go_to_line_end(&mut self) {
        self.overwrite_buffer = true;
        let line = self.rows[self.cursor_row].line;
        let last_row = self.lines[line].last_row;
        let last_width = self.rows[last_row].width;
        if self.cursor_row == last_row && self.cursor_col == last_width {
            self.move_right();
        } else {
            self.cursor_row = last_row;
            self.cursor_col = last_width;
        }
    }

    /// Moves the cursor to the very beginning of all input text and scrolls
    /// the viewport so that the first row is visible.
    pub fn go_to_start(&mut self) {
        self.overwrite_buffer = true;
        let first = self.first_row();
        self.cursor_row = first;
        self.cursor_col = 0;
        self.set_viewport_top_at(first, 0);
    }

    /// Moves the cursor to the very end of all input text and scrolls the
    /// viewport so that the final row is visible.
    pub fn go_to_end(&mut self) {
        self.overwrite_buffer = true;
        let last = self.last_row();
        self.cursor_row = last;
        self.cursor_col = self.rows[last].width;
        self.set_viewport_bottom_at(last, self.num_rows() - 1);
    }

    /// Deletes all graphemes from the beginning of the current logical line up
    /// to the cursor. Deletes line ending if used at the start of a line
    pub fn delete_to_line_start(&mut self) {
        let line = self.rows[self.cursor_row].line;
        let first_row = self.lines[line].first_row;
        let mut start = GraphemePos(first_row, 0);
        let pos = self.cursor_pos();
        if start == pos {
            let prev = self.rows[first_row].prev;
            if prev == self.head { return; }
            start = GraphemePos(prev, self.rows[prev].graphemes.len() - 1);
        }
        self.buffer_prepend(start, pos);
        self.splice(start, pos, "");
    }

    /// Deletes all graphemes from the cursor up to the end of the current
    /// logical line, keeping the trailing newline so the line remains. Deletes
    /// line ending if used at the end of a line.
    pub fn delete_to_line_end(&mut self) {
        let line = self.rows[self.cursor_row].line;
        let last_row = self.lines[line].last_row;
        // The final grapheme is the zero-width newline; stop before it.
        let mut end = GraphemePos(last_row, self.rows[last_row].graphemes.len() - 1);
        let pos = self.cursor_pos();
        if pos == end {
            let next = self.rows[last_row].next;
            if next == self.head { return; }
            end = GraphemePos(next, 0);
        }
        self.buffer_append(pos, end);
        self.splice(pos, end, "");
    }

    /// Moves the cursor forward to an exact grapheme position and updates the
    /// viewport. The position given must be *after* the current cursor pos.
    fn move_cursor_forward(&mut self, pos: GraphemePos) {
        self.overwrite_buffer = true;

        let base = self.cursor_row;
        let base_to_top = self.row_diff(self.viewport_top, base);
        let base_pos = self.viewport_top_pos as isize + base_to_top;
        let cursor = self.row_diff(base, pos.row());

        self.cursor_row = pos.row();
        self.cursor_col = self.rows[pos.row()].graphemes[pos.grapheme()].column as usize;

        let prev_top = self.viewport_top_pos as isize - base_pos;
        let prev_bottom = self.viewport_bottom_pos as isize - base_pos;
        self.recompute_viewport(base, base_pos, cursor, prev_top, prev_bottom);
    }

    /// Moves the cursor forward to an exact grapheme position and updates the
    /// viewport. The position given must be *before* the current cursor pos.
    fn move_cursor_backward(&mut self, pos: GraphemePos) {
        self.overwrite_buffer = true;

        let base = self.cursor_row;
        let base_to_top = self.row_diff(self.viewport_top, base);
        let base_pos = self.viewport_top_pos as isize + base_to_top;
        let cursor = -self.row_diff(pos.row(), base);

        self.cursor_row = pos.row();
        self.cursor_col = self.rows[pos.row()].graphemes[pos.grapheme()].column as usize;

        let prev_top = self.viewport_top_pos as isize - base_pos;
        let prev_bottom = self.viewport_bottom_pos as isize - base_pos;
        self.recompute_viewport(base, base_pos, cursor, prev_top, prev_bottom);
    }

    fn grapheme_start(&self) -> GraphemePos {
        GraphemePos(self.first_row(), 0)
    }

    fn grapheme_end(&self) -> GraphemePos {
        GraphemePos(self.head, 0)
    }

    fn last_grapheme(&self) -> GraphemePos {
        GraphemePos(self.last_row(), self.rows[self.last_row()].graphemes.len() - 1)
    }

    fn word_end(&self) -> GraphemePos {
        let mut iter = self.iter_graphemes(self.cursor_pos(), self.grapheme_end());
        (&mut iter).find(|(_, g)| g.is_alphanumeric());
        iter.find(|(_, g)| !g.is_alphanumeric())
            .map(|(pos, _)| pos)
            .unwrap_or(self.last_grapheme())
    }

    fn prev_word_start(&self) -> GraphemePos {
        let iter = self.iter_graphemes(self.grapheme_start(), self.cursor_pos()).rev();
        let prev = iter.take_while(|(_, g)| !g.is_alphanumeric())
            .last()
            .map_or(self.cursor_pos(), |(pos, _)| pos);
        let iter = self.iter_graphemes(self.grapheme_start(), prev).rev();
        iter.take_while(|(_, g)| g.is_alphanumeric())
            .last()
            .map(|(pos, _)| pos)
            .unwrap_or(self.grapheme_start())
    }

    /// Goes to the nearest word end after the cursor.
    pub fn go_to_word_end(&mut self) {
        self.move_cursor_forward(self.word_end());
    }

    /// Goes to the nearest word start before the cursor.
    pub fn go_to_prev_word_start(&mut self) {
        self.move_cursor_backward(self.prev_word_start());
    }

    /// Kills text between the previous word start and the cursor. Appends to
    /// buffer
    pub fn delete_prev_word(&mut self) {
        let start = self.prev_word_start();
        let pos = self.cursor_pos();
        self.buffer_prepend(start, pos);
        self.splice(start, pos, "");
    }

    /// Clears the input and returns its former contents as a command event.
    fn submit(&mut self) -> AppEvent {
        let mut text = self.get_text();
        self.set_text("");
        if text.ends_with('\n') { text.pop(); }
        AppEvent::Command(text)
    }

    /// Handles a single keyboard event, applying the corresponding edit or
    /// movement. Returns `Some(text)` when the input is submitted, in which
    /// case the buffer is cleared. Enter submits special commands (input
    /// beginning with `!` or `/`) and inserts a newline otherwise; Alt+Enter
    /// does the opposite.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<AppEvent> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let mut response = None;
        match (key.code, ctrl, shift, alt) {
            // Ctrl + char
            (KeyCode::Char('a'), true, _, _) => self.go_to_line_start(),
            (KeyCode::Char('e'), true, _, _) => self.go_to_line_end(),
            (KeyCode::Char('u'), true, _, _) => self.delete_to_line_start(),
            (KeyCode::Char('k'), true, _, _) => self.delete_to_line_end(),
            (KeyCode::Char('w'), true, _, _) => self.delete_prev_word(),
            (KeyCode::Char('y'), true, _, _) => self.paste_buffer(),
            (KeyCode::Char('c'), true, _, _) => {
                let start = self.grapheme_start();
                let end = self.last_grapheme();
                if start != end {
                    self.buffer_append(start, end);
                    self.splice(start, end, "");
                } else {
                    response = Some(AppEvent::Interrupt);
                }
            }
            // Alt + char
            (KeyCode::Char('f'), _, _, true) => self.go_to_word_end(),
            (KeyCode::Char('b'), _, _, true) => self.go_to_prev_word_start(),
            (KeyCode::Char('u'), _, _, true) => self.move_up(self.max_height()),
            (KeyCode::Char('d'), _, _, true) => self.move_down(self.max_height()),
            // Other combinations
            | (KeyCode::Char('j'), true, _, _)
            | (KeyCode::Char('j'), _, _, true)
            | (KeyCode::Enter, true, _, _)
            | (KeyCode::Enter, _, true, _) => self.paste("\n"),
            // Ignoring modifiers
            (KeyCode::Char(c), _, _, _) => {
                let mut s = CompactString::with_capacity(1);
                s.push(c);
                self.paste(&s);
            }
            (KeyCode::Enter, _, _, true) => {
                if self.is_special_command() {
                    self.paste("\n");
                } else {
                    response = Some(self.submit());
                }
            }
            (KeyCode::Enter, _, _, _) => {
                if self.is_special_command() {
                    response = Some(self.submit());
                } else {
                    self.paste("\n");
                }
            }
            // XXX: Maybe should expand to spaces when input via keyboard
            (KeyCode::Tab, _, _, _) => self.paste("\t"),
            (KeyCode::Backspace, _, _, _) => self.backspace(),
            (KeyCode::Delete, _, _, _) => self.delete(),
            (KeyCode::Left, _, _, _) => self.move_left(),
            (KeyCode::Right, _, _, _) => self.move_right(),
            (KeyCode::Up, _, _, _) => {
                if self.cursor_row == self.first_row() && self.cursor_col == 0 {
                    response = Some(AppEvent::HistoryPrev);
                } else {
                    self.move_up(1);
                }
            }
            (KeyCode::Down, _, _, _) => {
                let last = self.last_row();
                if self.cursor_row == last && self.cursor_col == self.rows[last].width {
                    response = Some(AppEvent::HistoryNext);
                } else {
                    self.move_down(1);
                }
            }
            (KeyCode::PageUp, _, _, _) => self.move_up(self.max_height()),
            (KeyCode::PageDown, _, _, _) => self.move_down(self.max_height()),
            (KeyCode::Home, _, _, _) => self.go_to_start(),
            (KeyCode::End, _, _, _) => self.go_to_end(),
            _ => {},
        };
        response
    }
}

impl Component for InputBox {
    type Update<'a> = ();
    type Event = Option<AppEvent>;

    fn draw(&self, canvas: Canvas) {
        let prev = self.rows[self.viewport_top].prev;
        for (i, (_, row)) in self.iter_range(prev, self.viewport_bottom).enumerate() {
            let pad_width = canvas[i].width() + self.width;
            for g in &row.graphemes {
                match &g.data[..] {
                    "\t" => canvas[i].push(&SPACES[..g.width as usize], g.width as usize),
                    "\n" => {}
                    _ => canvas[i].push(&g.data, g.width as usize),
                }
            }
            canvas[i].pad_to_width(pad_width);
        }
    }

    fn set_width(&mut self, width: usize) {
        InputBox::set_width(self, width);
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
        let row = self.row_diff(self.viewport_top, self.cursor_row) as usize;
        let cursor_pos = self.cursor_pos();
        let col = self.rows[cursor_pos.row()].graphemes[cursor_pos.grapheme()].column;
        Some((row, col as usize))
    }

    fn handle_input(&mut self, event: Event) -> Self::Event {
        match event {
            Event::Key(key) => self.handle_key(key),
            _ => None,
        }
    }

    fn handle_update<'a>(&mut self, _update: Self::Update<'a>) {
    }
}

/// Exposed fields:
/// - width: number
/// - max_height: number
/// - num_rows: number
/// - num_lines: number
/// - text: string
/// - head: id
/// - viewport_top: id
/// - viewport_top_pos: number
/// - viewport_bottom: id
/// - viewport_bottom_pos: number
/// - cursor_row: id
/// - cursor_col: number
/// - buffer: string
/// - overwrite_buffer: bool
impl DataQuery for InputBox {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "width": self.width,
                "max_height": self.max_height,
                "num_rows": self.num_rows(),
                "num_lines": self.num_lines(),
                "text": self.get_text(),
                "head": self.head.to_json(),
                "viewport_top": self.viewport_top.to_json(),
                "viewport_top_pos": self.viewport_top_pos,
                "viewport_bottom": self.viewport_bottom.to_json(),
                "viewport_bottom_pos": self.viewport_bottom_pos,
                "cursor_row": self.cursor_row.to_json(),
                "cursor_col": self.cursor_col,
                "buffer": self.buffer,
                "overwrite_buffer": self.overwrite_buffer,
            }))),
            "width" => Ok(QueryField::Value(json!(self.width))),
            "max_height" => Ok(QueryField::Value(json!(self.max_height))),
            "num_rows" => Ok(QueryField::Value(json!(self.num_rows()))),
            "num_lines" => Ok(QueryField::Value(json!(self.num_lines()))),
            "text" => Ok(QueryField::Value(json!(self.get_text()))),
            "head" => Ok(QueryField::Value(self.head.to_json())),
            "viewport_top" => Ok(QueryField::Value(self.viewport_top.to_json())),
            "viewport_top_pos" => Ok(QueryField::Value(json!(self.viewport_top_pos))),
            "viewport_bottom" => Ok(QueryField::Value(self.viewport_bottom.to_json())),
            "viewport_bottom_pos" => Ok(QueryField::Value(json!(self.viewport_bottom_pos))),
            "cursor_row" => Ok(QueryField::Value(self.cursor_row.to_json())),
            "cursor_col" => Ok(QueryField::Value(json!(self.cursor_col))),
            "buffer" => Ok(QueryField::Value(json!(self.buffer))),
            "overwrite_buffer" => Ok(QueryField::Value(json!(self.overwrite_buffer))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct InputRowIter<'i> {
    input: &'i InputBox,
    prev: Id<InputRow>,
    last: Id<InputRow>,
}

impl<'i> Iterator for InputRowIter<'i> {
    type Item = (Id<InputRow>, &'i InputRow);

    fn next(&mut self) -> Option<Self::Item> {
        if self.prev == self.last {
            return None;
        }
        let id = self.input.rows[self.prev].next;
        self.prev = id;
        Some((id, &self.input.rows[id]))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        ((self.prev != self.last) as usize, None)
    }
}

impl<'i> DoubleEndedIterator for InputRowIter<'i> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.prev == self.last {
            return None;
        }
        let id = self.last;
        self.last = self.input.rows[self.last].prev;
        Some((id, &self.input.rows[id]))
    }
}

impl<'i> std::iter::FusedIterator for InputRowIter<'i> {}

#[derive(Clone, Copy, Debug)]
struct InputGraphemeIter<'i> {
    input: &'i InputBox,
    start: GraphemePos,
    end: GraphemePos,
}

impl<'i> InputGraphemeIter<'i> {
    fn new(
        input: &'i InputBox,
        start: GraphemePos,
        mut end: GraphemePos,
    ) -> Self {
        // Normalize so that end < graphemes.len(). This fixes some edge cases.
        let end_row = &input.rows[end.row()];
        if end.grapheme() == end_row.graphemes.len() {
            end = GraphemePos(end_row.next, 0);
        }
        Self {
            input,
            start,
            end,
        }
    }
}

impl<'i> Iterator for InputGraphemeIter<'i> {
    type Item = (GraphemePos, &'i InputGrapheme);

    fn next(&mut self) -> Option<Self::Item> {
        if self.start == self.end {
            return None;
        }
        let pos = self.start;
        let grapheme = &self.input.rows[pos.row()].graphemes[pos.grapheme()];
        self.start = GraphemePos(pos.row(), pos.grapheme() + 1);
        if self.start.grapheme() == self.input.rows[pos.row()].graphemes.len() {
            self.start = GraphemePos(self.input.rows[pos.row()].next, 0);
        }
        Some((pos, grapheme))
    }
}

impl<'i> DoubleEndedIterator for InputGraphemeIter<'i> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.start == self.end {
            return None;
        }
        if self.end.grapheme() == 0 {
            let row = self.input.rows[self.end.row()].prev;
            self.end = GraphemePos(row, self.input.rows[row].graphemes.len());
        }
        let pos = GraphemePos(self.end.row(), self.end.grapheme() - 1);
        self.end = pos;
        Some((pos, &self.input.rows[pos.row()].graphemes[pos.grapheme()]))
    }
}

impl<'i> std::iter::FusedIterator for InputGraphemeIter<'i> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::ToJson;
    use crate::ui::text::truncate_line;
    use serde_json::json;

    const SAMPLE: &str = r"Is it for fear to wet a widow's eye,
That thou consum'st thy self in single life?
Ah! if thou issueless shalt hap to die,
The world will wail thee like a makeless wife;
The world will be thy widow and still weep
That thou no form of thee hast left behind,
When every private widow well may keep
By children's eyes, her husband's shape in mind:
Look what an unthrift in the world doth spend
Shifts but his place, for still the world enjoys it;
But beauty's waste hath in the world an end,
And kept unused the user so destroys it.
No love toward others in that bosom sits
That on himself such murd'rous shame commits.
";

    fn row(is_last: bool, text: &str) -> InputRow {
        let mut row = InputRow::from_row(truncate_line(80, text));
        if is_last {
            row.graphemes.push(InputGrapheme {
                data: "\n".into(),
                column: row.width as u16,
                width: 0,
            });
        }
        row
    }

    #[test]
    fn test_grapheme_iter() {
        let mut input = InputBox::new(10, 8);
        input.set_text("abcdefgabcdefgabcdefgabcdefg");
        let start = input.first_row();
        let start_grapheme = 3;
        let end = input.last_row();
        let end_grapheme = 4;
        let start_pos = GraphemePos(start, start_grapheme);
        let end_pos = GraphemePos(end, end_grapheme);

        let s: String = input.iter_graphemes(start_pos, end_pos)
            .map(|(_, g)| &g.data[..])
            .collect();
        assert_eq!(s, "defgabcdefgabcdefgabc");

        let t: String = input.iter_graphemes(start_pos, end_pos)
            .rev()
            .map(|(_, g)| &g.data[..])
            .collect();
        assert_eq!(t, "cbagfedcbagfedcbagfed");
    }

    #[test]
    fn test_grapheme_at_col() {
        let final_row = row(true, "a界c");
        assert_eq!(final_row.grapheme_at_col(0), 0);
        assert_eq!(final_row.grapheme_at_col(1), 1);
        assert_eq!(final_row.grapheme_at_col(2), 1);
        assert_eq!(final_row.grapheme_at_col(3), 2);
        assert_eq!(final_row.grapheme_at_col(4), 3);
        assert_eq!(final_row.grapheme_at_col(usize::MAX), 3);

        let wrapped_row = row(false, "a界c");
        assert_eq!(wrapped_row.grapheme_at_col(4), 2);
        assert_eq!(wrapped_row.grapheme_at_col(usize::MAX), 2);

        assert_eq!(row(true, "").grapheme_at_col(0), 0);
    }

    #[test]
    fn test_insert_text() {
        let mut input = InputBox::new(20, 8);

        assert_eq!(input.num_rows(), 1);
        assert_eq!(input.num_lines(), 1);
        assert_eq!(input.get_text(), "\n");

        input.insert_text(input.head, "first\nsecond");
        assert_eq!(input.num_rows(), 3);
        assert_eq!(input.num_lines(), 3);
        assert_eq!(input.get_text(), "first\nsecond\n\n");

        input.insert_text(input.rows[input.head].next, "middle");
        assert_eq!(input.num_rows(), 4);
        assert_eq!(input.num_lines(), 4);
        assert_eq!(input.get_text(), "first\nmiddle\nsecond\n\n");

        let (id, _) = input.iter_rows().nth(input.num_rows() - 2).unwrap();
        input.insert_text(id, "last");
        assert_eq!(input.num_rows(), 5);
        assert_eq!(input.num_lines(), 5);
        assert_eq!(input.get_text(), "first\nmiddle\nsecond\nlast\n\n");

        let (id, _) = input.iter_rows().nth(input.num_rows() - 2).unwrap();
        input.insert_text(id, "a line that is way too long");
        assert_eq!(input.num_rows(), 7);
        assert_eq!(input.num_lines(), 6);
        assert_eq!(
            input.get_text(),
            "first\nmiddle\nsecond\nlast\na line that is way too long\n\n"
        );
    }

    #[test]
    fn test_splice_delete_newline() {
        let mut input = InputBox::new(80, 8);
        input.set_text("abc\ndef");
        input.set_viewport_top(input.first_row());
        assert_eq!(input.get_text(), "abc\ndef\n");
        assert_eq!(input.num_lines(), 2);
        assert_eq!(input.num_rows(), 2);

        let rows = row_ids(&input);
        // Newline grapheme is the final (zero-width) grapheme of the first row.
        let start = GraphemePos(rows[0], input.rows[rows[0]].graphemes.len() - 1);
        let end = GraphemePos(rows[1], 0);

        input.splice(start, end, "");

        assert_eq!(input.get_text(), "abcdef\n");
        assert_eq!(input.num_lines(), 1);
        assert_eq!(input.num_rows(), 1);
    }

    fn splice(
        input: &mut InputBox,
        start_row: usize,
        start_grapheme: usize,
        end_row: usize,
        end_grapheme: usize,
        inserted_text: &str,
    ) -> String {
        let rows = row_ids(input);
        let start = GraphemePos(rows[start_row], start_grapheme);
        let end = GraphemePos(rows[end_row], end_grapheme);
        input.splice(start, end, inserted_text);
        input.get_text()
    }

    #[test]
    fn test_splice() {
        // Replace within a single row
        let mut input = InputBox::new(20, 8);
        input.set_text("abcdefg");
        assert_eq!(input.get_text(), "abcdefg\n");
        assert_eq!(splice(&mut input, 0, 2, 0, 4, "XY"), "abXYefg\n");
        assert_eq!(input.num_lines(), 1);
        assert_eq!(input.num_rows(), 1);

        // Replace spanning two rows
        let mut input = InputBox::new(10, 8);
        input.set_text("abcdefghijklmnopqrst");
        assert_eq!(input.num_rows(), 2);
        assert_eq!(input.get_text(), "abcdefghijklmnopqrst\n");
        assert_eq!(splice(&mut input, 0, 2, 1, 2, "ZZ"), "abZZmnopqrst\n");
        assert_eq!(input.num_lines(), 1);
        assert_eq!(input.num_rows(), 2);

        // Replace spanning two lines
        let mut input = InputBox::new(80, 8);
        input.set_text("abcd\ndefg");
        assert_eq!(input.get_text(), "abcd\ndefg\n");
        assert_eq!(splice(&mut input, 0, 3, 1, 2, "XY"), "abcXYfg\n");
        assert_eq!(input.num_lines(), 1);
        assert_eq!(input.num_rows(), 1);
    }

    #[test]
    fn test_paste() {
        let mut input = InputBox::new(20, 8);

        input.set_text("abcdef");
        input.set_viewport_top(input.first_row());
        assert_eq!(input.get_text(), "abcdef\n");

        // Paste at the beginning of the line.
        input.cursor_row = input.first_row();
        input.cursor_col = 0;
        input.paste("XY");
        assert_eq!(input.get_text(), "XYabcdef\n");

        // Paste in the middle of the line.
        input.cursor_row = input.first_row();
        input.cursor_col = 4;
        input.paste("XY");
        assert_eq!(input.get_text(), "XYabXYcdef\n");

        // Paste at the end of the line (one column past the last row).
        input.cursor_row = input.first_row();
        input.cursor_col = input.rows[input.first_row()].width;
        input.paste("XY");
        assert_eq!(input.get_text(), "XYabXYcdefXY\n");
    }

    #[test]
    fn test_delete() {
        let mut input = InputBox::new(80, 8);
        input.set_text("abcde");
        input.set_viewport_top(input.first_row());
        assert_eq!(input.get_text(), "abcde\n");

        // Delete a grapheme in the middle of a line.
        input.cursor_row = input.first_row();
        input.cursor_col = 2;
        input.delete();
        assert_eq!(input.get_text(), "abde\n");

        // Delete at the end of the last line does nothing.
        input.cursor_col = input.rows[input.first_row()].width;
        input.delete();
        assert_eq!(input.get_text(), "abde\n");

        // Delete a newline, merging lines.
        let mut input = InputBox::new(80, 8);
        input.set_text("abc\ndef");
        input.set_viewport_top(input.first_row());
        let rows = row_ids(&input);
        input.cursor_row = rows[0];
        input.cursor_col = input.rows[rows[0]].width;
        input.delete();
        assert_eq!(input.get_text(), "abcdef\n");
        assert_eq!(input.num_lines(), 1);
    }

    #[test]
    fn test_backspace() {
        let mut input = InputBox::new(80, 8);
        input.set_text("abcde");
        input.set_viewport_top(input.first_row());
        assert_eq!(input.get_text(), "abcde\n");

        // Backspace a grapheme in the middle of a line.
        input.cursor_row = input.first_row();
        input.cursor_col = 3;
        input.backspace();
        assert_eq!(input.get_text(), "abde\n");

        // Backspace at the start of the first line does nothing.
        input.cursor_col = 0;
        input.backspace();
        assert_eq!(input.get_text(), "abde\n");

        // Backspace at the start of a line merges with the previous line.
        let mut input = InputBox::new(80, 8);
        input.set_text("abc\ndef");
        input.set_viewport_top(input.first_row());
        let rows = row_ids(&input);
        input.cursor_row = rows[1];
        input.cursor_col = 0;
        input.backspace();
        assert_eq!(input.get_text(), "abcdef\n");
        assert_eq!(input.num_lines(), 1);
    }

    #[test]
    fn test_splice_scrolling() {
        // Cursor moves out of viewport
        let mut input = InputBox::new(80, 4);
        input.set_text(&SAMPLE[..SAMPLE.len() - 1]);
        assert_eq!(input.num_rows(), 14);
        let rows = row_ids(&input);
        assert_eq!(input.viewport_bottom, rows[3]);

        input.paste(SAMPLE);
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 28);
        assert_eq!((input.cursor_row, input.cursor_col), (rows[14], 0));
        assert_eq!(input.viewport_top, rows[14]);
        assert_eq!(input.viewport_bottom, rows[17]);
        assert_eq!(input.viewport_top_pos(), 14);
        assert_eq!(input.viewport_bottom_pos(), 17);
        check_positions(&input);

        // Deletion scrolls viewport up
        let mut input = InputBox::new(80, 4);
        input.set_text(&SAMPLE[..SAMPLE.len() - 1]);
        assert_eq!(input.num_rows(), 14);
        let rows = row_ids(&input);
        input.cursor_row = rows[13];
        input.cursor_col = 0;
        input.set_viewport_top(rows[10]);
        assert_eq!(input.viewport_top, rows[10]);
        assert_eq!(input.viewport_bottom, rows[13]);

        splice(&mut input, 6, 0, 13, 0, "");
        assert_eq!(input.num_rows(), 7);
        let rows = row_ids(&input);
        assert_eq!(input.cursor_row, rows[6]);
        assert_eq!(rows.len(), 7);
        assert_eq!(input.viewport_top, rows[3]);
        assert_eq!(input.viewport_bottom, rows[6]);
        assert_eq!(input.viewport_top_pos(), 3);
        assert_eq!(input.viewport_bottom_pos(), 6);
        check_positions(&input);

        // No scrolling
        let mut input = InputBox::new(80, 10);
        input.set_text(&SAMPLE[..SAMPLE.len() - 1]);
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 14);

        input.paste("hello\n");
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 15);
        assert_eq!((input.cursor_row, input.cursor_col), (rows[1], 0));
        assert_eq!(input.viewport_top, rows[0]);
        assert_eq!(input.viewport_bottom, rows[9]);

        splice(&mut input, 0, 0, 1, 0, "");
        assert_eq!(input.num_rows(), 14);
        let rows = row_ids(&input);
        assert_eq!(input.viewport_top, rows[0]);
        assert_eq!(input.viewport_bottom, rows[9]);
        assert_eq!(input.get_text(), SAMPLE);

        // Shrink height
        splice(&mut input, 3, 0, 13, 0, "");
        assert_eq!(input.num_rows(), 4);
        let rows = row_ids(&input);
        assert_eq!(input.viewport_top, rows[0]);
        assert_eq!(input.viewport_bottom, rows[3]);
        assert_eq!(input.viewport_top_pos(), 0);
        assert_eq!(input.viewport_bottom_pos(), 3);
        check_positions(&input);
    }

    fn row_ids(input: &InputBox) -> Vec<Id<InputRow>> {
        input.iter_rows().map(|(id, _)| id).collect()
    }

    /// Verifies the tracked viewport position fields against `row_diff` from
    /// the first row. Catches drift between the cached positions and the
    /// linked-list reality.
    fn check_positions(input: &InputBox) {
        assert_eq!(
            input.viewport_top_pos(),
            input.row_diff(input.first_row(), input.viewport_top) as usize,
            "viewport_top_pos mismatch",
        );
        assert_eq!(
            input.viewport_bottom_pos(),
            input.row_diff(input.first_row(), input.viewport_bottom) as usize,
            "viewport_bottom_pos mismatch",
        );
    }

    #[test]
    fn test_cursor_movement() {
        let mut input = InputBox::new(80, 4);

        input.paste(&SAMPLE[..SAMPLE.len() - 1]);
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 14);

        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 45));
        assert_eq!(input.viewport_top, rows[10]);
        assert_eq!(input.viewport_bottom, rows[13]);

        input.cursor_row = rows[0];
        input.cursor_col = 10;
        input.set_viewport_top(rows[0]);
        input.move_left();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 9));
        input.move_right();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 10));

        // Test first row
        input.cursor_col = 10;
        input.move_up(1);
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        assert_eq!(
            (input.viewport_top, input.viewport_bottom),
            (rows[0], rows[3])
        );
        input.move_up(1);
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        assert_eq!(
            (input.viewport_top, input.viewport_bottom),
            (rows[0], rows[3])
        );
        input.move_left();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        assert_eq!(
            (input.viewport_top, input.viewport_bottom),
            (rows[0], rows[3])
        );

        // Test last row
        input.cursor_row = rows[13];
        input.set_viewport_top(rows[10]);
        input.move_down(1);
        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 45));
        assert_eq!(
            (input.viewport_top, input.viewport_bottom),
            (rows[10], rows[13])
        );
        input.move_down(1);
        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 45));
        assert_eq!(
            (input.viewport_top, input.viewport_bottom),
            (rows[10], rows[13])
        );
        input.move_right();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 45));
        assert_eq!(
            (input.viewport_top, input.viewport_bottom),
            (rows[10], rows[13])
        );

        // Test scrolling
        input.cursor_row = rows[1];
        input.cursor_col = 0;
        input.set_viewport_top(rows[1]);
        input.move_up(1);
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        assert_eq!(
            (input.viewport_top, input.viewport_bottom),
            (rows[0], rows[3])
        );

        input.cursor_row = rows[12];
        input.cursor_col = 0;
        input.set_viewport_top(rows[9]);
        input.move_down(1);
        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 0));
        assert_eq!(
            (input.viewport_top, input.viewport_bottom),
            (rows[10], rows[13])
        );

        // Test line endings
        input.cursor_row = rows[0];
        input.cursor_col = 35;
        input.set_viewport_top(rows[0]);
        input.move_right();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 36));
        input.move_right();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[1], 0));
        input.move_left();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 36));
        input.move_left();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 35));

        // Test with wrapped line
        let mut input = InputBox::new(10, 4);
        input.paste("123456789 123456789");
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 2);
        input.cursor_row = rows[0];
        input.cursor_col = 9;
        input.move_right();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[1], 0));
        input.move_left();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 9));
        check_positions(&input);
    }

    #[test]
    fn test_line_start_end_and_delete() {
        // go_to_line_start / go_to_line_end
        let mut input = InputBox::new(80, 8);
        input.set_text("abcd\nefgh\nijkl");
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 3);

        input.cursor_row = rows[1];
        input.cursor_col = 3;
        input.go_to_line_start();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[1], 0));
        input.go_to_line_end();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[1], 4));

        // Wrapped line: start/end span the whole logical line.
        let mut input = InputBox::new(10, 8);
        input.paste("123456789 123456789");
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 2);
        input.cursor_row = rows[0];
        input.cursor_col = 9;
        input.go_to_line_start();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        input.go_to_line_end();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[1], 9));

        // delete_to_line_start keeps the rest of the line.
        let mut input = InputBox::new(80, 8);
        input.set_text("abcd\nefgh");
        let rows = row_ids(&input);
        input.cursor_row = rows[1];
        input.cursor_col = 3;
        input.delete_to_line_start();
        assert_eq!(input.get_text(), "abcd\nh\n");
        assert_eq!(input.buffer, "efg");

        // delete_to_line_end keeps the newline (and thus the line).
        let mut input = InputBox::new(80, 8);
        input.set_text("abcd\nefgh");
        let rows = row_ids(&input);
        input.cursor_row = rows[1];
        input.cursor_col = 1;
        input.delete_to_line_end();
        assert_eq!(input.get_text(), "abcd\ne\n");
        assert_eq!(input.num_lines(), 2);
        assert_eq!(input.buffer, "fgh");
    }

    #[test]
    fn test_go_to_line_start_and_end() {
        let mut input = InputBox::new(80, 8);
        input.set_text("first\nsecond\nthird");
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 3);

        // Start in the middle of line 2.
        input.cursor_row = rows[1];
        input.cursor_col = 3;

        // First call to go_to_line_start goes to the start of the current line.
        input.go_to_line_start();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[1], 0));

        // Repeated call when already at start goes back one character (end of previous line).
        input.go_to_line_start();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 5));

        // Moving to start of first line.
        input.go_to_line_start();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));

        // Repeated call at the very beginning of the buffer stays at (rows[0], 0).
        input.go_to_line_start();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));

        // go_to_line_end from start goes to end of current line.
        input.go_to_line_end();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 5));

        // Repeated call when already at line end goes forward one character (start of next line).
        input.go_to_line_end();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[1], 0));

        // Go to end of line 2.
        input.go_to_line_end();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[1], 6));

        // Forward to start of line 3.
        input.go_to_line_end();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[2], 0));

        // Go to end of line 3.
        input.go_to_line_end();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[2], 5));

        // Repeated call at the very end of the buffer stays at (rows[2], 5).
        input.go_to_line_end();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[2], 5));

        // Wrapped line test
        let mut input = InputBox::new(10, 8);
        input.paste("123456789 123456789\nabc");
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 3);

        // Cursor at row 1 (the wrapped second row of line 1), col 5.
        input.cursor_row = rows[1];
        input.cursor_col = 5;

        // Line start goes to the start of the entire logical line (row 0, col 0).
        input.go_to_line_start();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));

        // Line end goes to the end of the entire logical line (row 1, col 9).
        input.go_to_line_end();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[1], 9));

        // Line end when already at line end advances to the next line (row 2, col 0).
        input.go_to_line_end();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[2], 0));

        // Line start when at line start moves back to the end of previous line (row 1, col 9).
        input.go_to_line_start();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[1], 9));
    }

    #[test]
    fn test_delete_to_line_boundaries() {
        // delete_to_line_start at the beginning of a line deletes the
        // previous line's ending, merging the two lines.
        let mut input = InputBox::new(80, 8);
        input.set_text("abc\ndef");
        input.set_viewport_top(input.first_row());
        let rows = row_ids(&input);
        input.cursor_row = rows[1];
        input.cursor_col = 0;
        input.delete_to_line_start();
        assert_eq!(input.get_text(), "abcdef\n");
        assert_eq!(input.num_lines(), 1);
        assert_eq!(input.buffer, "\n");

        // ... but does nothing at the beginning of the first line.
        let mut input = InputBox::new(80, 8);
        input.set_text("abc\ndef");
        input.set_viewport_top(input.first_row());
        let rows = row_ids(&input);
        input.cursor_row = rows[0];
        input.cursor_col = 0;
        input.delete_to_line_start();
        assert_eq!(input.get_text(), "abc\ndef\n");
        assert_eq!(input.buffer, "");

        // delete_to_line_end at the end of a line deletes the line's ending,
        // merging the line with the next one.
        let mut input = InputBox::new(80, 8);
        input.set_text("abc\ndef");
        input.set_viewport_top(input.first_row());
        let rows = row_ids(&input);
        input.cursor_row = rows[0];
        input.cursor_col = input.rows[rows[0]].width;
        input.delete_to_line_end();
        assert_eq!(input.get_text(), "abcdef\n");
        assert_eq!(input.num_lines(), 1);
        assert_eq!(input.buffer, "\n");

        // ... but does nothing at the end of the last line.
        let mut input = InputBox::new(80, 8);
        input.set_text("abc\ndef");
        input.set_viewport_top(input.first_row());
        let rows = row_ids(&input);
        input.cursor_row = rows[1];
        input.cursor_col = input.rows[rows[1]].width;
        input.delete_to_line_end();
        assert_eq!(input.get_text(), "abc\ndef\n");
        assert_eq!(input.buffer, "");
    }

    #[test]
    fn test_move_cursor_scrolls_viewport() {
        let mut input = InputBox::new(80, 4);
        input.set_text(&SAMPLE[..SAMPLE.len() - 1]);
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 14);
        assert_eq!(input.viewport_top, rows[0]);
        assert_eq!(input.viewport_bottom, rows[3]);
        assert_eq!(input.viewport_top_pos(), 0);
        assert_eq!(input.viewport_bottom_pos(), 3);

        // Moving to the very start keeps the viewport at the top.
        input.move_cursor_forward(GraphemePos(rows[0], 0));
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        assert_eq!(input.viewport_top, rows[0]);
        assert_eq!(input.viewport_top_pos(), 0);

        // Moving to the end of the last row scrolls the viewport down to keep
        // the cursor visible with a margin.
        let last = rows[13];
        input.move_cursor_forward(GraphemePos(last, input.rows[last].graphemes.len() - 1));
        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 45));
        assert_eq!(input.viewport_top, rows[10]);
        assert_eq!(input.viewport_bottom, rows[13]);
        assert_eq!(input.viewport_top_pos(), 10);
        assert_eq!(input.viewport_bottom_pos(), 13);

        // Moving back to the start scrolls the viewport to the top.
        input.move_cursor_backward(GraphemePos(rows[0], 0));
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        assert_eq!(input.viewport_top, rows[0]);
        assert_eq!(input.viewport_bottom, rows[3]);
        assert_eq!(input.viewport_top_pos(), 0);
        assert_eq!(input.viewport_bottom_pos(), 3);
    }

    #[test]
    fn test_resize_preserves_viewport_position() {
        let mut input = InputBox::new(80, 4);
        input.set_text("one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten");
        assert_eq!(input.num_rows(), 10);
        let rows = row_ids(&input);

        // Window shows rows 4..7, cursor at row 6 (offset 2).
        input.cursor_row = rows[6];
        input.set_viewport_top(rows[4]);
        assert_eq!(input.row_diff(input.viewport_top, input.cursor_row), 2);

        // Re-wrap to a narrower width. Short lines stay on one row each.
        input.set_width(40);
        assert_eq!(input.num_rows(), 10);

        let rows = row_ids(&input);
        assert_eq!(input.cursor_row, rows[6]);
        assert_eq!(input.row_diff(input.viewport_top, input.cursor_row), 2);
        assert_eq!(input.row_diff(input.viewport_top, input.viewport_bottom), 3);

        // Cursor at the top of the viewport stays at the top.
        input.cursor_row = rows[2];
        input.set_viewport_top(rows[0]);
        assert_eq!(input.row_diff(input.viewport_top, input.cursor_row), 2);
        input.set_width(20);
        assert_eq!(input.row_diff(input.viewport_top, input.cursor_row), 2);

        // Shrinking the viewport below the cursor offset clamps to the bottom.
        input.set_max_height(2);
        assert_eq!(
            input.row_diff(input.viewport_top, input.cursor_row),
            input.row_diff(input.viewport_top, input.viewport_bottom)
        );
        check_positions(&input);
    }

    #[test]
    fn test_next_word() {
        let mut input = InputBox::new(80, 4);

        input.set_text("abc def ghi");
        input.cursor_row = input.first_row();
        input.cursor_col = 0;
        input.go_to_word_end();
        assert_eq!(input.cursor_col, 3);
        input.cursor_col = 1;
        input.go_to_word_end();
        assert_eq!(input.cursor_col, 3);
        input.cursor_col = 3;
        input.go_to_word_end();
        assert_eq!(input.cursor_col, 7);

        input.set_text("a b c");
        input.cursor_col = 0;
        input.go_to_word_end();
        assert_eq!(input.cursor_col, 1);
        input.cursor_col = 1;
        input.go_to_word_end();
        assert_eq!(input.cursor_col, 3);
        check_positions(&input);
    }

    #[test]
    fn test_prev_word() {
        let mut input = InputBox::new(80, 4);

        input.set_text("abc def ghi");
        input.cursor_row = input.first_row();
        input.cursor_col = 7;
        input.go_to_prev_word_start();
        assert_eq!(input.cursor_col, 4);
        input.cursor_col = 6;
        input.go_to_prev_word_start();
        assert_eq!(input.cursor_col, 4);
        input.cursor_col = 4;
        input.go_to_prev_word_start();
        assert_eq!(input.cursor_col, 0);

        input.set_text("a b c");
        input.cursor_col = 4;
        input.go_to_prev_word_start();
        assert_eq!(input.cursor_col, 2);
        check_positions(&input);
    }

    #[test]
    fn test_del_prev_word() {
        let mut input = InputBox::new(80, 4);

        input.set_text("abc def ghi");
        input.cursor_row = input.first_row();
        input.cursor_col = 0;
        input.delete_prev_word();
        assert_eq!(input.get_text(), "abc def ghi\n");
        assert_eq!(input.buffer, "");
        input.cursor_col = 8;
        input.delete_prev_word();
        assert_eq!(input.get_text(), "abc ghi\n");
        assert_eq!(input.buffer, "def ");
        input.move_cursor_backward(GraphemePos(input.first_row(), 1));
        input.delete_prev_word();
        assert_eq!(input.get_text(), "bc ghi\n");
        assert_eq!(input.buffer, "a");

        input.set_text("a b");
        input.cursor_col = 3;
        input.delete_prev_word();
        assert_eq!(input.get_text(), "a \n");
        assert_eq!(input.buffer, "b");
    }

    #[test]
    fn test_overwrite_buffer_flag() {
        let mut input = InputBox::new(80, 8);
        input.set_text("hello world");

        // Delete a word to fill the buffer.
        input.cursor_row = input.first_row();
        input.cursor_col = 11; // end of "world"
        input.delete_prev_word();
        assert_eq!(input.buffer, "world");
        assert!(!input.overwrite_buffer);

        // Moving the cursor and deleting again overwrites the buffer instead
        // of appending.
        input.go_to_line_start();
        input.cursor_col = 5; // end of "hello"
        input.delete_prev_word();
        assert_eq!(input.buffer, "hello");
        assert!(!input.overwrite_buffer);
    }

    #[test]
    fn test_home_end() {
        let mut input = InputBox::new(80, 4);
        input.paste(&SAMPLE[..SAMPLE.len() - 1]);
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 14);

        // Cursor starts at the end; viewport follows the newest content.
        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 45));
        assert_eq!(input.viewport_top, rows[10]);
        assert_eq!(input.viewport_bottom, rows[13]);

        // Home moves the cursor to the first grapheme and scrolls to the top.
        input.handle_key(KeyEvent::from(KeyCode::Home));
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        assert_eq!(input.viewport_top, rows[0]);

        // End moves the cursor to the end of the last row and scrolls down.
        input.handle_key(KeyEvent::from(KeyCode::End));
        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 45));
        assert_eq!(input.viewport_bottom, rows[13]);
        assert_eq!(input.viewport_top_pos(), 10);
        assert_eq!(input.viewport_bottom_pos(), 13);
        check_positions(&input);
    }

    #[test]
    fn test_ctrl_c() {
        let mut input = InputBox::new(80, 8);
        input.set_text("draft command");
        input.go_to_end();
        let response = input.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(response, None);
        assert_eq!(input.get_text(), "\n");
        assert_eq!(input.buffer, "draft command");
        assert_eq!(input.num_rows(), 1);
        input.go_to_start();
        input.paste_buffer();
        assert_eq!(input.get_text(), "draft command\n");
        assert_eq!(input.buffer, "");

        let mut input = InputBox::new(80, 8);
        input.set_text("first line\nsecond line");
        input.go_to_end();
        input.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(input.get_text(), "\n");
        assert_eq!(input.buffer, "first line\nsecond line");
        assert_eq!(input.num_lines(), 1);

        // Pressing Ctrl+C twice does not overwrite buffer
        input.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(input.get_text(), "\n");
        assert_eq!(input.buffer, "first line\nsecond line");
        check_positions(&input);
    }

    #[test]
    fn test_enter_key() {
        let mut input = InputBox::new(80, 8);

        // Plain text: Enter inserts a newline, Alt+Enter submits.
        input.set_text("hello");
        input.go_to_end();
        assert!(!input.is_special_command());
        let response = input.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(response, None);
        assert_eq!(input.get_text(), "hello\n\n");
        let response = input.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        assert_eq!(response, Some(AppEvent::Command("hello\n".to_string())));
        assert_eq!(input.get_text(), "\n");

        // Special commands: Enter submits, Alt+Enter inserts a newline.
        input.set_text("/clear");
        assert!(input.is_special_command());
        let response = input.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(response, Some(AppEvent::Command("/clear".to_string())));
        assert_eq!(input.get_text(), "\n");
        input.set_text("!deploy");
        input.go_to_end();
        assert!(input.is_special_command());
        let response = input.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        assert_eq!(response, None);
        assert_eq!(input.get_text(), "!deploy\n\n");

        // A special character not at the start of the input does not count.
        input.set_text("a/b c");
        assert!(!input.is_special_command());

        // Ctrl+Enter and Ctrl/Alt+J always insert a newline.
        input.set_text("multi");
        input.go_to_end();
        input.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL));
        assert_eq!(input.get_text(), "multi\n\n");
        input.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        assert_eq!(input.get_text(), "multi\n\n\n");
        input.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::ALT));
        assert_eq!(input.get_text(), "multi\n\n\n\n");
        check_positions(&input);
    }

    #[test]
    fn test_query() {
        let input = InputBox::new(20, 8);
        let expected = json!({
            "width": 20,
            "max_height": 8,
            "num_rows": 1,
            "num_lines": 1,
            "text": "\n",
            "head": input.head.to_json(),
            "viewport_top": input.viewport_top.to_json(),
            "viewport_top_pos": 0,
            "viewport_bottom": input.viewport_bottom.to_json(),
            "viewport_bottom_pos": 0,
            "cursor_row": input.cursor_row.to_json(),
            "cursor_col": 0,
            "buffer": "",
            "overwrite_buffer": false,
        });
        assert_eq!(input.query("/").unwrap(), expected);
        assert_eq!(input.query("/width").unwrap(), json!(20));
        assert_eq!(input.query("/max_height").unwrap(), json!(8));
        assert_eq!(input.query("/num_rows").unwrap(), json!(1));
        assert_eq!(input.query("/num_lines").unwrap(), json!(1));
        assert_eq!(input.query("/text").unwrap(), json!("\n"));
        assert_eq!(input.query("/head").unwrap(), input.head.to_json());
        assert_eq!(input.query("/viewport_top").unwrap(), input.viewport_top.to_json());
        assert_eq!(input.query("/viewport_top_pos").unwrap(), json!(0));
        assert_eq!(input.query("/viewport_bottom").unwrap(), input.viewport_bottom.to_json());
        assert_eq!(input.query("/viewport_bottom_pos").unwrap(), json!(0));
        assert_eq!(input.query("/cursor_row").unwrap(), input.cursor_row.to_json());
        assert_eq!(input.query("/cursor_col").unwrap(), json!(0));
        assert_eq!(input.query("/buffer").unwrap(), json!(""));
        assert_eq!(input.query("/overwrite_buffer").unwrap(), json!(false));
    }
}
