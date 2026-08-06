use std::slice::SliceIndex;

use compact_str::CompactString;
use crossterm::style::ContentStyle;

use crate::arena::{Arena, Id};
use crate::canvas::Canvas;
use crate::text::{Row, strip_cr, wrap_line};

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
    /// Length of line in bytes. Good for reserving string capacity.
    len_bytes: usize,
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

        let len_bytes = s.len();
        let line = lines.insert(Self {
            first_row: Id::null(),
            last_row: Id::null(),
            num_rows: 0,
            len_bytes,
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

    fn data<I>(&self, range: I) -> impl Iterator<Item = &'_ str> + '_
    where
        I: SliceIndex<[InputGrapheme], Output = [InputGrapheme]>,
    {
        self.graphemes[range].iter().map(|g| &g.data[..])
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
    viewport_bottom: Id<InputRow>,
    cursor_row: Id<InputRow>,
    cursor_col: usize,
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
            len_bytes: 0,
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
            cursor_row: first,
            cursor_col: 0,
        }
    }

    pub fn num_rows(&self) -> usize {
        // Subtract header node
        self.rows.len() - 1
    }

    pub fn num_lines(&self) -> usize {
        self.lines.len()
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

    /// Attempts to set the viewport region based on first row.
    fn set_viewport_top(&mut self, viewport_top: Id<InputRow>) {
        let prev = self.rows[viewport_top].prev;
        self.viewport_top = viewport_top;
        if let Some(row) = self
            .iter_range(prev, self.last_row())
            .nth(self.max_height - 1)
            .map(|(id, _)| id)
        {
            self.viewport_bottom = row;
        } else if self.viewport_top == self.first_row() {
            // Viewport covers entire text
            self.viewport_bottom = self.last_row()
        } else {
            self.set_viewport_bottom(self.last_row());
        }
    }

    /// Attempts to set the viewport region based on last row.
    fn set_viewport_bottom(&mut self, viewport_bottom: Id<InputRow>) {
        self.viewport_bottom = viewport_bottom;
        if let Some(row) = self
            .iter_range(self.head, self.viewport_bottom)
            .rev()
            .nth(self.max_height - 1)
            .map(|(id, _)| id)
        {
            self.viewport_top = row
        } else if self.viewport_bottom == self.last_row() {
            // Viewport covers entire text
            self.viewport_top = self.first_row();
        } else {
            self.set_viewport_top(self.first_row());
        }
    }

    /// Recomputes the viewport after moving the cursor. Previous viewport
    /// bounds must be computed in advance. Tries to keep `MARGIN_ROWS` rows
    /// between the cursor and the viewport edges.
    fn recompute_viewport(
        &mut self,
        base: Id<InputRow>,
        prev_top: isize,
        prev_bottom: isize,
    ) {
        let margin = Self::MARGIN_ROWS;
        let cursor = self.row_diff(base, self.cursor_row);
        if cursor > prev_top + margin {
            let bottom = self.row_offset(self.cursor_row, margin).unwrap_or(self.last_row());
            self.set_viewport_bottom(bottom);
        } else if cursor < prev_bottom - margin {
            let top = self.row_offset(self.cursor_row, -margin).unwrap_or(self.first_row());
            self.set_viewport_top(top);
        } else {
            let bottom = self.row_offset(base, prev_bottom)
                .unwrap_or(self.last_row());
            self.set_viewport_bottom(bottom);
        }
    }

    pub fn height(&self) -> usize {
        std::cmp::min(self.max_height, self.num_rows())
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

    fn set_text(&mut self, text: &str) {
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
        self.set_viewport_top(self.first_row());
        self.cursor_row = self.first_row();
        self.cursor_col = 0;
    }

    /// Updates the wrapping width, re-wrapping all existing text. The cursor
    /// is restored to the same byte offset.
    pub fn set_width(&mut self, width: usize) {
        if width == self.width {
            return;
        }

        let text = self.get_text();
        // The text is unchanged (only the wrap width differs), so the grapheme
        // index of the cursor is stable across the re-wrap.
        let cursor_index = self
            .iter_graphemes(GraphemePos(self.first_row(), 0), self.cursor_pos())
            .count();
        // Offset of the cursor within the viewport, preserved across the
        // re-wrap when possible.
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

    /// Updates the maximum viewport size
    pub fn set_max_height(&mut self, max_height: usize) {
        if max_height == 0 {
            return;
        }
        self.max_height = max_height;
        self.fit_viewport_on_resize(self.row_diff(self.viewport_top, self.cursor_row) as usize);
    }

    /// Repositions the viewport to keep the cursor in the desired row, as long
    /// as it is possible to do so.
    fn fit_viewport_on_resize(&mut self, cursor_row: usize) {
        let k = cursor_row.min(self.max_height - 1);

        // Scan up k rows and check for the top
        let first = match self.row_offset(self.cursor_row, -(k as isize)) {
            Some(first) => first,
            None => {
                self.set_viewport_top(self.first_row());
                return;
            }
        };

        // Scan down max_height - k rows and check for the bottom
        let down = self.max_height - 1 - k;
        if self.row_offset(self.cursor_row, down as isize).is_none() {
            self.set_viewport_bottom(self.last_row());
            return;
        }

        self.set_viewport_top(first);
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

        // Assume splice start point comes before viewport bottom
        let viewport_bottom_offset = self.row_diff(prev, self.viewport_bottom);

        // Cursor byte offset relative to end of last line.
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

        // Recompute view viewport
        self.recompute_viewport(
            prev,
            viewport_bottom_offset - self.max_height as isize,
            viewport_bottom_offset,
        );
    }

    /// Pastes raw text at the cursor position.
    pub fn paste(&mut self, pasted_text: &str) {
        if pasted_text.is_empty() { return; }
        let pos = self.cursor_pos();
        self.splice(pos, pos, pasted_text);
    }

    // FIXME: Make this configurable, and handle 0 specifically
    const MARGIN_ROWS: isize = 3;

    /// Moves the cursor up one row. Preserves the column of the cursor. If
    /// already at the very first row, moves to the start of the line. Scrolls
    /// the viewport if at the top. Tries to keep some rows between the cursor
    /// and viewport edge.
    pub fn move_up(&mut self, rows: usize) {
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
            let new_top = self.row_offset(self.viewport_top, margin - Self::MARGIN_ROWS)
                .unwrap_or(self.first_row());
            self.set_viewport_top(new_top);
        }
    }

    /// Moves the cursor down one row. Preserves the column of the cursor. If
    /// already at the very last row, moves to the end of the line. Scrolls the
    /// viewport if at the bottom. Tries to keep some rows between the cursor
    /// and viewport edge.
    pub fn move_down(&mut self, rows: usize) {
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
            let new_bottom = self.row_offset(self.viewport_bottom, Self::MARGIN_ROWS - margin)
                .unwrap_or(self.last_row());
            self.set_viewport_bottom(new_bottom);
        }
    }

    /// Moves the cursor left by one grapheme. If at the start of a row, goes
    /// to the end of the previous row.
    pub fn move_left(&mut self) {
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

    /// Moves the cursor to the beginning of the current logical line.
    pub fn go_to_line_start(&mut self) {
        let line = self.rows[self.cursor_row].line;
        self.cursor_row = self.lines[line].first_row;
        self.cursor_col = 0;
    }

    /// Moves the cursor to the end of the current logical line.
    pub fn go_to_line_end(&mut self) {
        let line = self.rows[self.cursor_row].line;
        let last_row = self.lines[line].last_row;
        self.cursor_row = last_row;
        self.cursor_col = self.rows[last_row].width;
    }



    /// Deletes all graphemes from the beginning of the current logical line up
    /// to the cursor.
    pub fn delete_to_line_start(&mut self) {
        let line = self.rows[self.cursor_row].line;
        let start = GraphemePos(self.lines[line].first_row, 0);
        let pos = self.cursor_pos();
        if start != pos {
            self.splice(start, pos, "");
        }
    }

    /// Deletes all graphemes from the cursor up to the end of the current
    /// logical line, keeping the trailing newline so the line remains.
    pub fn delete_to_line_end(&mut self) {
        let line = self.rows[self.cursor_row].line;
        let last_row = self.lines[line].last_row;
        // The final grapheme is the zero-width newline; stop before it.
        let end = GraphemePos(last_row, self.rows[last_row].graphemes.len() - 1);
        let pos = self.cursor_pos();
        if pos != end {
            self.splice(pos, end, "");
        }
    }

    /// Moves the cursor to an exact grapheme position and updates the
    /// viewport.
    fn move_cursor_to(&mut self, pos: GraphemePos) {
        let base = self.first_row();
        let prev_top = self.row_diff(base, self.viewport_top);
        let prev_bottom = self.row_diff(base, self.viewport_bottom);

        self.cursor_row = pos.row();
        self.cursor_col = self.rows[pos.row()].graphemes[pos.grapheme()].column as usize;

        self.recompute_viewport(base, prev_top, prev_bottom);
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
        self.move_cursor_to(self.word_end());
    }

    /// Goes to the nearest word start before the cursor.
    pub fn go_to_prev_word_start(&mut self) {
        self.move_cursor_to(self.prev_word_start());
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
    fn peek(&self) -> Option<(GraphemePos, &'i InputGrapheme)> {
        self.clone().next()
    }

    fn peek_back(&self) -> Option<(GraphemePos, &'i InputGrapheme)> {
        self.clone().next_back()
    }
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

/// Draws the viewport into a canvas, along with the cursor block.
#[derive(Debug)]
pub struct DrawInputBox<'i> {
    pub input: &'i InputBox,
    pub x: u16,
    pub y: u16,
    pub style: ContentStyle,
}

impl<'i> DrawInputBox<'i> {
    pub fn draw_to(&self, canvas: &mut Canvas) {
        // Draw text
        let prev = self.input.rows[self.input.viewport_top].prev;
        let top_y = self.y;
        let mut cursor_row = u16::MAX;
        for (offset, (row_id, row)) in self.input
            .iter_range(prev, self.input.viewport_bottom)
            .enumerate()
        {
            let row_y = top_y + offset as u16;
            canvas.write_str(self.x, row_y, &row.preformatted, self.style);
            if row_id == self.input.cursor_row {
                cursor_row = row_y;
            }
        }

        // Draw cursor
        let row = &self.input.rows[self.input.cursor_row];
        let g = row.grapheme_at_col(self.input.cursor_col);
        let column = row.graphemes[g].column as u16;
        canvas.set_cursor_pos(self.x + column, cursor_row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::truncate_line;

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
    }

    fn row_ids(input: &InputBox) -> Vec<Id<InputRow>> {
        input.iter_rows().map(|(id, _)| id).collect()
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

        // delete_to_line_end keeps the newline (and thus the line).
        let mut input = InputBox::new(80, 8);
        input.set_text("abcd\nefgh");
        let rows = row_ids(&input);
        input.cursor_row = rows[1];
        input.cursor_col = 1;
        input.delete_to_line_end();
        assert_eq!(input.get_text(), "abcd\ne\n");
        assert_eq!(input.num_lines(), 2);
    }

    #[test]
    fn test_move_cursor_to_scrolls_viewport() {
        let mut input = InputBox::new(80, 4);
        input.set_text(&SAMPLE[..SAMPLE.len() - 1]);
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 14);
        assert_eq!(input.viewport_top, rows[0]);
        assert_eq!(input.viewport_bottom, rows[3]);

        // Moving to the very start keeps the viewport at the top.
        input.move_cursor_to(GraphemePos(rows[0], 0));
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        assert_eq!(input.viewport_top, rows[0]);

        // Moving to the end of the last row scrolls the viewport down to keep
        // the cursor visible with a margin.
        let last = rows[13];
        input.move_cursor_to(GraphemePos(last, input.rows[last].graphemes.len() - 1));
        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 45));
        assert_eq!(input.viewport_top, rows[10]);
        assert_eq!(input.viewport_bottom, rows[13]);

        // Moving back to the start scrolls the viewport to the top.
        input.move_cursor_to(GraphemePos(rows[0], 0));
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        assert_eq!(input.viewport_top, rows[0]);
        assert_eq!(input.viewport_bottom, rows[3]);
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
    }
}
