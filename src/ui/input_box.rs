use std::slice::SliceIndex;

use compact_str::CompactString;

use crate::arena::{Arena, Id};
use crate::text::{Row, strip_cr, wrap_line};

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
    column: u32,
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
            graphemes: vec![],
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
        let mut column: u32 = 0;
        let mut preformatted = String::new();
        for g in row.graphemes {
            preformatted.push_str(g.formatted());
            let width = g.width;
            graphemes.push(InputGrapheme {
                data: g.data,
                width,
                column,
            });
            column += width as u32;
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
        self.graphemes
            .iter()
            .position(|grapheme| col < grapheme.column as usize + grapheme.width as usize)
            .unwrap_or_else(|| self.graphemes.len() - 1)
    }
}

#[derive(Debug)]
struct InputBox {
    lines: Arena<InputLine>,
    rows: Arena<InputRow>,
    width: usize,
    /// Maximum number of visible rows
    max_height: usize,
    /// Head of circularly linked list. Contains no real data.
    head: Id<InputRow>,
    first_visible_row: Id<InputRow>,
    last_visible_row: Id<InputRow>,
    cursor_row: Id<InputRow>,
    cursor_col: usize,
}

impl InputBox {
    fn new(width: usize, max_height: usize) -> Self {
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
            graphemes: vec![InputGrapheme { data: "\n".into(), column: 0, width: 0 }],
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
            first_visible_row: first,
            last_visible_row: first,
            cursor_row: first,
            cursor_col: 0,
        }
    }

    fn num_rows(&self) -> usize {
        // Subtract header node
        self.rows.len() - 1
    }

    fn num_lines(&self) -> usize {
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

    /// Iterates over graphemes between two arbitrary points. `end_grapheme` is
    /// not included in the iterator range.
    fn iter_graphemes<'a>(
        &'a self,
        start_row: Id<InputRow>,
        start_grapheme: usize,
        end_row: Id<InputRow>,
        end_grapheme: usize,
    ) -> InputGraphemeIter<'a> {
        InputGraphemeIter { input: self, start_row, start_grapheme, end_row, end_grapheme }
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

    fn set_first_visible_row(&mut self, first_visible_row: Id<InputRow>) {
        let prev = self.rows[first_visible_row].prev;
        self.first_visible_row = first_visible_row;
        self.last_visible_row = self
            .iter_range(prev, self.last_row())
            .nth(self.max_height - 1)
            .map(|(id, _)| id)
            .unwrap_or(self.last_row());
    }

    fn set_last_visible_row(&mut self, last_visible_row: Id<InputRow>) {
        self.last_visible_row = last_visible_row;
        self.first_visible_row = self
            .iter_range(self.head, self.last_visible_row)
            .rev()
            .nth(self.max_height - 1)
            .map(|(id, _)| id)
            .unwrap_or(self.first_row());
    }

    fn height(&self) -> usize {
        std::cmp::min(self.max_height, self.num_rows())
    }

    /// Computes text of all lines.
    fn get_text(&self) -> String {
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
        self.remove_line(self.rows[self.first_row()].line);
        self.insert_text(self.head, text);
        self.set_first_visible_row(self.first_row());
        self.cursor_row = self.first_row();
        self.cursor_col = 0;
    }

    /// Deletes a range of graphemes and replaces them with new text. If
    /// `inserted_text` is empty, graphemes will still be deleted without
    /// inserting any new text. `start_grapheme` is inclusive while
    /// `end_grapheme` is not: `end_grapheme` may equal `graphemes.len()`. If
    /// the deletion range contains a trailing newline, the newline will be
    /// deleted and consecutive lines merged.
    ///
    /// The cursor will be placed after the inserted/deleted text and the
    /// scroll window updated appropriately.
    // XXX: Don't think this handles case end_grapheme == graphemes.len() correctly
    fn splice(
        &mut self,
        start_row: Id<InputRow>,
        start_grapheme: usize,
        end_row: Id<InputRow>,
        end_grapheme: usize,
        inserted_text: &str,
    ) {
        let start_line = self.rows[start_row].line;
        let end_line = self.rows[end_row].line;
        let prev = self.rows[self.lines[start_line].first_row].prev;
        let next = self.rows[self.lines[end_line].last_row].next;

        // Assume splice start point comes before last visible row
        let last_visible_row_offset = self.row_diff(prev, self.last_visible_row);

        // Cursor byte offset relative to end of last line.
        // Why byte offset? In some cases, pasted codepoints will alter the
        // grapheme segmentation of the rest of the line. This ensures the
        // cursor still points to the same codepoint it did previously.
        let cursor_offset: usize = self.iter_graphemes(end_row, end_grapheme, next, 0)
            .map(|(_, _, g)| g.data.len())
            .sum();

        // Splice strings
        let mut out = String::new();
        out.extend(
            self.iter_graphemes(self.lines[start_line].first_row, 0, start_row, start_grapheme)
                .map(|(_, _, g)| &g.data[..])
        );
        out.push_str(inserted_text);
        out.extend(
            self.iter_graphemes(end_row, end_grapheme, next, 0)
                .map(|(_, _, g)| &g.data[..])
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
        let (row, _, g) = self.iter_graphemes(self.first_row(), 0, next, 0)
            .rev()
            .find(|(_, _, g)| {
                bytes += g.data.len();
                bytes >= cursor_offset
            })
            .unwrap();
        self.cursor_col = g.column as _;
        self.cursor_row = row;

        // Recompute view window
        let cursor_row_offset = self.row_diff(prev, self.cursor_row);
        let last_visible_row_offset = cursor_row_offset.max(last_visible_row_offset);
        let row = self.row_offset(prev, last_visible_row_offset).unwrap_or(self.last_row());
        self.set_last_visible_row(row);
    }

    /// Pastes raw text at the cursor position.
    fn paste(&mut self, pasted_text: &str) {
        if pasted_text.is_empty() { return; }
        let index = self.rows[self.cursor_row].grapheme_at_col(self.cursor_col);
        self.splice(self.cursor_row, index, self.cursor_row, index, pasted_text);
    }

    /// Scrolls the visible window up one row. If the cursor is at the final
    /// visible row, move the cursor up one row. Does nothing if already at the
    /// first overall row.
    fn scroll_up(&mut self) {
        if self.first_visible_row == self.first_row() {
            return;
        }

        let last_visible = self.last_visible_row;
        if self.cursor_row == last_visible {
            self.cursor_row = self.rows[self.cursor_row].prev;
        }
        self.first_visible_row = self.rows[self.cursor_row].prev;
        self.last_visible_row = self.rows[last_visible].prev;
    }

    /// Scrolls the visible window up one row. If the cursor is at the first
    /// visible row, move the cursor down one row. Does nothing if already at
    /// the last overall row.
    fn scroll_down(&mut self) {
        let last_visible = self.last_visible_row;
        if last_visible == self.last_row() {
            return;
        }

        if self.cursor_row == self.first_visible_row {
            self.cursor_row = self.rows[self.cursor_row].next;
        }
        self.first_visible_row = self.rows[self.first_visible_row].next;
        self.last_visible_row = self.rows[last_visible].next;
    }

    /// Moves the cursor up one row. Preserves the column of the cursor. If
    /// already at the very first row, moves to the start of the line. Scrolls
    /// the visible window if at the top.
    fn move_up(&mut self) {
        if self.cursor_row == self.first_row() {
            self.cursor_col = 0;
            return;
        }

        if self.cursor_row == self.first_visible_row {
            let last_visible = self.last_visible_row;
            self.first_visible_row = self.rows[self.first_visible_row].prev;
            self.last_visible_row = self.rows[last_visible].prev;
        }
        self.cursor_row = self.rows[self.cursor_row].prev;
    }

    /// Moves the cursor down one row. Preserves the column of the cursor. If
    /// already at the very last row, moves to the end of the line. Scrolls the
    /// visible window if at the bottom.
    fn move_down(&mut self) {
        if self.cursor_row == self.last_row() {
            self.cursor_col = self.rows[self.cursor_row].width;
            return;
        }

        let cursor_row = self.cursor_row;
        let last_visible = self.last_visible_row;
        if cursor_row == last_visible {
            self.first_visible_row = self.rows[self.first_visible_row].next;
            self.last_visible_row = self.rows[last_visible].next;
        }
        self.cursor_row = self.rows[cursor_row].next;
    }

    /// Moves the cursor left by one grapheme. If at the start of a row, goes
    /// to the end of the previous row.
    fn move_left(&mut self) {
        let index = self.rows[self.cursor_row].grapheme_at_col(self.cursor_col);
        if index == 0 {
            if self.cursor_row == self.first_row() {
                return;
            }
            self.cursor_row = self.rows[self.cursor_row].prev;
            self.cursor_col = self.rows[self.cursor_row].graphemes.len() - 1;
        } else {
            self.cursor_col = self.rows[self.cursor_row].graphemes[index - 1].column as usize;
        }
    }

    /// Moves the cursor right by one grapheme. If at the end of a row, goes to
    /// the start of the next row.
    fn move_right(&mut self) {
        let row = &self.rows[self.cursor_row];
        let index = row.grapheme_at_col(self.cursor_col);
        if index == row.graphemes.len() - 1 {
            if self.cursor_row == self.last_row() {
                return;
            }
            self.cursor_row = row.next;
            self.cursor_col = 0;
            return;
        } else {
            self.cursor_col = self.rows[self.cursor_row].graphemes[index + 1].column as usize;
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
    start_row: Id<InputRow>,
    start_grapheme: usize,
    end_row: Id<InputRow>,
    end_grapheme: usize,
}

impl<'i> Iterator for InputGraphemeIter<'i> {
    type Item = (Id<InputRow>, usize, &'i InputGrapheme);

    fn next(&mut self) -> Option<Self::Item> {
        if self.start_row == self.end_row && self.start_grapheme >= self.end_grapheme {
            return None;
        }
        let id = self.start_row;
        let index = self.start_grapheme;
        let grapheme = &self.input.rows[id].graphemes[index];
        self.start_grapheme += 1;
        if self.start_grapheme == self.input.rows[id].graphemes.len() {
            self.start_row = self.input.rows[id].next;
            self.start_grapheme = 0;
        }
        Some((id, index, grapheme))
    }
}

impl<'i> DoubleEndedIterator for InputGraphemeIter<'i> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.start_row == self.end_row && self.start_grapheme >= self.end_grapheme {
            return None;
        }
        if self.end_grapheme == 0 {
            self.end_row = self.input.rows[self.end_row].prev;
            self.end_grapheme = self.input.rows[self.end_row].graphemes.len();
        }
        let id = self.end_row;
        self.end_grapheme -= 1;
        Some((id, self.end_grapheme, &self.input.rows[id].graphemes[self.end_grapheme]))
    }
}

impl<'i> std::iter::FusedIterator for InputGraphemeIter<'i> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::truncate_line;

    const SAMPLE: &'static str = r"Is it for fear to wet a widow's eye,
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
                column: row.width as u32,
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

        let s: String = input.iter_graphemes(start, start_grapheme, end, end_grapheme)
            .map(|(_, _, g)| &g.data[..])
            .collect();
        assert_eq!(s, "defgabcdefgabcdefgabc");

        let t: String = input.iter_graphemes(start, start_grapheme, end, end_grapheme)
            .rev()
            .map(|(_, _, g)| &g.data[..])
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
        input.set_first_visible_row(input.first_row());
        assert_eq!(input.get_text(), "abc\ndef\n");
        assert_eq!(input.num_lines(), 2);
        assert_eq!(input.num_rows(), 2);

        let rows = row_ids(&input);
        // Newline grapheme is the final (zero-width) grapheme of the first row.
        let start_row = rows[0];
        let start_grapheme = input.rows[start_row].graphemes.len() - 1;
        let end_row = rows[1];
        let end_grapheme = 0;

        input.splice(start_row, start_grapheme, end_row, end_grapheme, "");

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
        input.splice(
            rows[start_row],
            start_grapheme,
            rows[end_row],
            end_grapheme,
            inserted_text,
        );
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
        input.set_first_visible_row(input.first_row());
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
    fn test_splice_scrolling() {
        // Cursor moves out of window
        let mut input = InputBox::new(80, 4);
        input.set_text(&SAMPLE[..SAMPLE.len() - 1]);
        assert_eq!(input.num_rows(), 14);
        let rows = row_ids(&input);
        assert_eq!(input.last_visible_row, rows[3]);

        input.paste(SAMPLE);
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 28);
        assert_eq!((input.cursor_row, input.cursor_col), (rows[14], 0));
        assert_eq!(input.first_visible_row, rows[11]);
        assert_eq!(input.last_visible_row, rows[14]);

        // Deletion scrolls window up
        let mut input = InputBox::new(80, 4);
        input.set_text(&SAMPLE[..SAMPLE.len() - 1]);
        assert_eq!(input.num_rows(), 14);
        let rows = row_ids(&input);
        input.cursor_row = rows[13];
        input.cursor_col = 0;
        input.set_first_visible_row(rows[10]);
        assert_eq!(input.first_visible_row, rows[10]);
        assert_eq!(input.last_visible_row, rows[13]);

        splice(&mut input, 6, 0, 13, 0, "");
        assert_eq!(input.num_rows(), 7);
        let rows = row_ids(&input);
        assert_eq!(input.cursor_row, rows[6]);
        assert_eq!(rows.len(), 7);
        assert_eq!(input.first_visible_row, rows[3]);
        assert_eq!(input.last_visible_row, rows[6]);

        // No scrolling
        let mut input = InputBox::new(80, 10);
        input.set_text(&SAMPLE[..SAMPLE.len() - 1]);
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 14);

        input.paste("hello\n");
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 15);
        assert_eq!((input.cursor_row, input.cursor_col), (rows[1], 0));
        assert_eq!(input.first_visible_row, rows[0]);
        assert_eq!(input.last_visible_row, rows[9]);

        splice(&mut input, 0, 0, 1, 0, "");
        assert_eq!(input.num_rows(), 14);
        let rows = row_ids(&input);
        assert_eq!(input.first_visible_row, rows[0]);
        assert_eq!(input.last_visible_row, rows[9]);
        assert_eq!(input.get_text(), SAMPLE);

        // Shrink height
        splice(&mut input, 3, 0, 13, 0, "");
        assert_eq!(input.num_rows(), 4);
        let rows = row_ids(&input);
        assert_eq!(input.first_visible_row, rows[0]);
        assert_eq!(input.last_visible_row, rows[3]);
    }

    fn row_ids(input: &InputBox) -> Vec<Id<InputRow>> {
        input.iter_rows().map(|(id, _)| id).collect()
    }

    #[test]
    fn test_scroll() {
        let mut input = InputBox::new(80, 2);
        input.set_text("one\ntwo\nthree\nfour");
        input.set_first_visible_row(input.first_row());

        let rows = row_ids(&input);
        input.cursor_row = rows[0];
        assert_eq!(input.last_visible_row, rows[1]);

        input.scroll_down();
        assert_eq!(input.first_visible_row, rows[1]);
        assert_eq!(input.last_visible_row, rows[2]);
        assert_eq!(input.cursor_row, rows[1]);

        input.cursor_row = rows[2];
        input.scroll_up();
        assert_eq!(input.first_visible_row, rows[0]);
        assert_eq!(input.last_visible_row, rows[1]);
        assert_eq!(input.cursor_row, rows[1]);

        input.scroll_up();
        assert_eq!(input.first_visible_row, rows[0]);
        assert_eq!(input.cursor_row, rows[1]);
    }

    #[test]
    fn test_cursor_movement() {
        let mut input = InputBox::new(80, 4);

        input.paste(&SAMPLE[..SAMPLE.len() - 1]);
        let rows = row_ids(&input);
        assert_eq!(rows.len(), 14);

        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 45));
        assert_eq!(input.first_visible_row, rows[10]);
        assert_eq!(input.last_visible_row, rows[13]);

        input.cursor_row = rows[0];
        input.cursor_col = 10;
        input.set_first_visible_row(rows[0]);
        input.move_left();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 9));
        input.move_right();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 10));

        // Test first row
        input.cursor_col = 10;
        input.move_up();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        assert_eq!(
            (input.first_visible_row, input.last_visible_row),
            (rows[0], rows[3])
        );
        input.move_up();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        assert_eq!(
            (input.first_visible_row, input.last_visible_row),
            (rows[0], rows[3])
        );
        input.move_left();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        assert_eq!(
            (input.first_visible_row, input.last_visible_row),
            (rows[0], rows[3])
        );

        // Test last row
        input.cursor_row = rows[13];
        input.set_first_visible_row(rows[10]);
        input.move_down();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 45));
        assert_eq!(
            (input.first_visible_row, input.last_visible_row),
            (rows[10], rows[13])
        );
        input.move_down();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 45));
        assert_eq!(
            (input.first_visible_row, input.last_visible_row),
            (rows[10], rows[13])
        );
        input.move_right();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 45));
        assert_eq!(
            (input.first_visible_row, input.last_visible_row),
            (rows[10], rows[13])
        );

        // Test scrolling
        input.cursor_row = rows[1];
        input.cursor_col = 0;
        input.set_first_visible_row(rows[1]);
        input.move_up();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[0], 0));
        assert_eq!(
            (input.first_visible_row, input.last_visible_row),
            (rows[0], rows[3])
        );

        input.cursor_row = rows[12];
        input.cursor_col = 0;
        input.set_first_visible_row(rows[9]);
        input.move_down();
        assert_eq!((input.cursor_row, input.cursor_col), (rows[13], 0));
        assert_eq!(
            (input.first_visible_row, input.last_visible_row),
            (rows[10], rows[13])
        );

        // Test line endings
        input.cursor_row = rows[0];
        input.cursor_col = 35;
        input.set_first_visible_row(rows[0]);
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
}
