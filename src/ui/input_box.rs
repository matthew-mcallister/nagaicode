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

        rows[last_row].is_last = true;
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
    /// Graphemes
    graphemes: Vec<InputGrapheme>,
    /// Width in columns
    width: usize,
    /// Text for rendering
    preformatted: String,
    /// True if row is last in its line
    is_last: bool,
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
            is_last: false,
        }
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
            is_last: false,
        }
    }

    /// Returns the index of the final grapheme in the row which is visitable
    /// with the movement keys. For the last row in a line, this is one past
    /// the last grapheme in the row. Else, it is the last grapheme in the row.
    fn final_visitable_grapheme(&self) -> usize {
        if self.is_last {
            self.graphemes.len()
        } else {
            // Only last row can be empty
            self.graphemes.len() - 1
        }
    }

    /// Returns the final column in the row which is visitable with the
    /// movement keys. For the last row in a line, this will be the column
    /// after the last grapheme. For other rows, this is the first column of
    /// the last grapheme.
    fn final_visitable_column(&self) -> usize {
        if self.is_last {
            self.width
        } else {
            // Only last row can be empty
            self.graphemes.last().unwrap().column as usize
        }
    }

    /// Finds the index of the grapheme at a given column within a row. If the
    /// column is past the end, then the final grapheme in the row. For the
    /// final row in the line, this returns one past the end of the grapheme
    /// array, which is the logical index of the newline.
    fn grapheme_at_col(&self, col: usize) -> usize {
        self.graphemes
            .iter()
            .position(|grapheme| col < grapheme.column as usize + grapheme.width as usize)
            .unwrap_or_else(|| self.final_visitable_grapheme())
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
            is_last: true,
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

    fn first_row(&self) -> Id<InputRow> {
        self.rows[self.head].next
    }

    fn last_row(&self) -> Id<InputRow> {
        self.rows[self.head].prev
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
        std::cmp::min(self.max_height, self.rows.len())
    }

    /// Computes text of all lines.
    fn get_text(&self) -> String {
        let mut out = String::new();
        for (_, row) in self.iter_rows() {
            for g in &row.graphemes {
                out.push_str(&g.data);
            }
            if row.is_last {
                out.push('\n');
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

    /// Pastes raw text at the cursor position.
    fn paste(&mut self, pasted_text: &str) {
        if pasted_text.is_empty() { return; }

        let line = self.rows[self.cursor_row].line;
        let prev = self.rows[self.lines[line].first_row].prev;
        let last_visible_row_offset = self.iter_range(prev, self.last_visible_row).count() - 1;

        // Buffer the graphemes of the original line, splicing in the pasted
        // text when the cursor position is reached.
        let mut out = String::new();
        let (mut cursor_row_offset, mut cursor_byte) = (usize::MAX, usize::MAX);
        for (i, (cur, cur_row)) in self.iter_line(line).enumerate() {
            if cur == self.cursor_row {
                let index = cur_row.grapheme_at_col(self.cursor_col);
                for g in &cur_row.graphemes[..index] {
                    out.push_str(&g.data);
                }

                out.push_str(pasted_text);

                // Calculate # added lines and cursor offset
                let (added_lines, last_line) = pasted_text.split('\n').enumerate().last().unwrap();
                cursor_byte = strip_cr(last_line).len();
                cursor_row_offset = i + added_lines;

                for g in &cur_row.graphemes[index..] {
                    out.push_str(&g.data);
                }
            } else {
                for g in &cur_row.graphemes {
                    out.push_str(&g.data);
                }
            }
        }

        self.remove_line(line);
        self.insert_text(prev, &out);

        // Recompute cursor row/column
        self.cursor_row = self.iter_range(prev, self.last_row())
            .nth(cursor_row_offset)
            .map(|(id, _)| id)
            .unwrap();
        let mut byte = 0;
        self.cursor_col = self.rows[self.cursor_row].graphemes.iter()
            .map(|g| {
                byte += g.data.len();
                (byte, g.column as usize)
            })
            .find(|&(byte, _)| byte > cursor_byte)
            .map_or(self.rows[self.cursor_row].width, |(_, column)| column);

        // Recompute view window
        let last_visible_row_offset = last_visible_row_offset.max(cursor_row_offset);
        let last_visible_row = self.iter_range(prev, self.last_row())
            .nth(last_visible_row_offset)
            .map(|(id, _)| id)
            .unwrap();
        self.set_last_visible_row(last_visible_row);
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
            self.cursor_col = self.rows[self.cursor_row].final_visitable_column();
        } else {
            self.cursor_col = self.rows[self.cursor_row].graphemes[index - 1].column as usize;
        }
    }

    /// Moves the cursor right by one grapheme. If at the end of a row, goes to
    /// the start of the next row.
    fn move_right(&mut self) {
        let row = &self.rows[self.cursor_row];
        let index = row.grapheme_at_col(self.cursor_col);
        if index == row.final_visitable_grapheme() {
            if self.cursor_row == self.last_row() {
                return;
            }
            self.cursor_row = row.next;
            self.cursor_col = 0;
            return;
        } else {
            // Handles case of moving one past end of final row
            self.cursor_col = self.rows[self.cursor_row]
                .graphemes
                .get(index + 1)
                .map_or(self.rows[self.cursor_row].width, |g| g.column as usize);
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

impl<'i> std::iter::ExactSizeIterator for InputRowIter<'i> {}

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
        row.is_last = is_last;
        row
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
    fn test_paste() {
        let mut input = InputBox::new(20, 8);

        input.insert_text(input.head, "abcdef");
        input.set_first_visible_row(input.first_row());
        assert_eq!(input.get_text(), "abcdef\n\n");

        // Paste at the beginning of the line.
        input.cursor_row = input.first_row();
        input.cursor_col = 0;
        input.paste("XY");
        assert_eq!(input.get_text(), "XYabcdef\n\n");

        // Paste in the middle of the line.
        input.cursor_row = input.first_row();
        input.cursor_col = 4;
        input.paste("XY");
        assert_eq!(input.get_text(), "XYabXYcdef\n\n");

        // Paste at the end of the line (one column past the last row).
        input.cursor_row = input.first_row();
        input.cursor_col = input.rows[input.first_row()].width;
        input.paste("XY");
        assert_eq!(input.get_text(), "XYabXYcdefXY\n\n");
    }

    #[test]
    fn test_paste_visible_lines() {
        let mut input = InputBox::new(80, 8);
        input.paste(&SAMPLE[..SAMPLE.len() - 1]);
        assert_eq!(input.num_rows(), 14);
        let rows = row_ids(&input);
        assert_eq!(input.first_visible_row, rows[6]);
        assert_eq!(input.last_visible_row, rows[13]);

        input.cursor_row = rows[7];
        input.cursor_col = 0;
        input.set_first_visible_row(rows[2]);
        assert_eq!(input.last_visible_row, rows[9]);
        input.paste(SAMPLE);
        assert_eq!(input.num_rows(), 28);
        let rows = row_ids(&input);
        assert_eq!((input.cursor_row, input.cursor_col), (rows[21], 0));
        assert_eq!(input.first_visible_row, rows[14]);
        assert_eq!(input.last_visible_row, rows[21]);
    }

    fn row_ids(input: &InputBox) -> Vec<Id<InputRow>> {
        input.iter_rows().map(|(id, _)| id).collect()
    }

    #[test]
    fn test_scroll() {
        let mut input = InputBox::new(80, 2);
        input.insert_text(input.head, "one\ntwo\nthree\nfour");
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
