use compact_str::CompactString;

use crate::arena::{Arena, Id};
use crate::text::{wrap_line, Row};

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

    /// Returns the index into the grapheme array of the final position of a
    /// row. The return value depends on whether or not the row is the last one
    /// in the line. For the last row, the final position is one past the final
    /// column in the row. For all other rows, the final position is at the
    /// final column in the row.
    fn final_pos(&self) -> usize {
        if self.is_last {
            self.graphemes.len()
        } else {
            self.graphemes.len().saturating_sub(1)
        }
    }

    /// Finds the grapheme index at a given column within a row. If the column
    /// is past the end, then returns the final position in the row.
    fn grapheme_at_col(&self, col: usize) -> usize {
        self.graphemes
            .iter()
            .position(|grapheme| col < grapheme.column as usize + grapheme.width as usize)
            .unwrap_or_else(|| self.final_pos())
    }
}

#[derive(Debug)]
struct InputBox {
    lines: Arena<InputLine>,
    rows: Arena<InputRow>,
    width: usize,
    max_height: usize,
    /// Head of circularly linked list. Contains no real data.
    head: Id<InputRow>,
    first_visible_row: Id<InputRow>,
    last_visible_row: Id<InputRow>,
    cursor_row: Id<InputRow>,
    cursor_col: usize,
}

impl InputBox {
    fn new(
        width: usize,
        max_height: usize,
    ) -> Self {
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

    fn iter_rows<'a>(&'a self) -> InputRowIter<'a> {
        let start = self.rows[self.head].next;
        let end = self.head;
        InputRowIter { input: self, start, end }
    }

    fn iter_line<'a>(&'a self, line: Id<InputLine>) -> InputRowIter<'a> {
        let line = &self.lines[line];
        let start = line.first_row;
        let end = self.rows[line.last_row].next;
        InputRowIter { input: self, start, end }
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
        debug_assert_eq!(cur, next);

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
    /// have line breaks; each source line will become its own input line.
    fn insert_text(&mut self, prev: Id<InputRow>, text: &str) {
        let mut prev = prev;
        for line_text in text.split('\n') {
            let line = InputLine::from_str(&mut self.lines, &mut self.rows, self.width, line_text);
            self.link_line(prev, line);
            prev = self.lines[line].last_row;
        }
    }

    /// Pastes raw text at the cursor position.
    fn paste(&mut self, pasted_text: &str) {
        let line = self.rows[self.cursor_row].line;
        let line_first = self.lines[line].first_row;
        let line_last = self.lines[line].last_row;
        let prev = self.rows[line_first].prev;

        // Buffer the graphemes of the original line, splicing in the pasted
        // text when the cursor position is reached.
        let mut out = String::new();
        let mut cur = line_first;
        loop {
            if cur == self.cursor_row {
                let index = self.rows[cur].grapheme_at_col(self.cursor_col);
                for g in &self.rows[cur].graphemes[..index] {
                    out.push_str(&g.data);
                }
                out.push_str(pasted_text);
                for g in &self.rows[cur].graphemes[index..] {
                    out.push_str(&g.data);
                }
            } else {
                for g in &self.rows[cur].graphemes {
                    out.push_str(&g.data);
                }
            }
            if cur == line_last {
                break;
            }
            cur = self.rows[cur].next;
        }

        self.remove_line(line);
        self.insert_text(prev, &out);
    }

    /// Moves the cursor up one row. Preserves the column of the cursor. If
    /// already at the very first row, moves to the start of the line. Scrolls
    /// the visible window if at the top.
    fn move_up(&mut self) {
        todo!()
    }

    /// Moves the cursor down one row. Preserves the column of the cursor. If
    /// already at the very last row, moves to the end of the line. Scrolls the
    /// visible window if at the bottom.
    fn move_down(&mut self) {
        todo!()
    }

    /// Moves the cursor left by one grapheme. If at the start of a row, goes
    /// to the end of the previous row.
    fn move_left(&mut self) {
        todo!()
    }

    /// Moves the cursor right by one grapheme. If at the end of a row, goes to
    /// the start of the next row.
    fn move_right(&mut self) {
        todo!()
    }
}

#[derive(Clone, Copy, Debug)]
struct InputRowIter<'i> {
    input: &'i InputBox,
    start: Id<InputRow>,
    end: Id<InputRow>,
}

impl<'i> Iterator for InputRowIter<'i> {
    type Item = (Id<InputRow>, &'i InputRow);

    fn next(&mut self) -> Option<Self::Item> {
        if self.start == self.end {
            return None;
        }
        let id = self.start;
        let row = &self.input.rows[id];
        self.start = row.next;
        Some((id, row))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        ((self.start != self.end) as usize, None)
    }
}

impl<'i> DoubleEndedIterator for InputRowIter<'i> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.start == self.end {
            return None;
        }
        let id = self.input.rows[self.end].prev;
        self.end = id;
        Some((id, &self.input.rows[id]))
    }
}

impl<'i> std::iter::FusedIterator for InputRowIter<'i> {}

impl<'i> std::iter::ExactSizeIterator for InputRowIter<'i> {}

#[cfg(test)]
mod tests {
    use crate::text::truncate_line;
    use super::*;

    fn row(is_last: bool, text: &str) -> InputRow {
        let mut row = InputRow::from_row(truncate_line(80, text));
        row.is_last = is_last;
        row
    }

    #[test]
    fn test_final_pos() {
        assert_eq!(row(true, "").final_pos(), 0);
        assert_eq!(row(false, "").final_pos(), 0);
        assert_eq!(row(true, "ab").final_pos(), 2);
        assert_eq!(row(false, "ab").final_pos(), 1);
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
        assert_eq!(row(false, "").grapheme_at_col(0), 0);
    }

    fn row_ids(input: &InputBox) -> Vec<Id<InputRow>> {
        input.iter_rows().map(|(id, _)| id).collect()
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
        assert_eq!(input.get_text(), "first\nmiddle\nsecond\nlast\na line that is way too long\n\n");
    }

    #[test]
    fn test_paste() {
        let mut input = InputBox::new(20, 8);

        input.insert_text(input.head, "abcdef");
        let first = input.rows[input.head].next;

        // Paste at the beginning of the line.
        input.cursor_row = first;
        input.cursor_col = 0;
        input.paste("XY");
        assert_eq!(input.get_text(), "XYabcdef\n\n");

        // Paste in the middle of the line.
        let first = input.rows[input.head].next;
        input.cursor_row = first;
        input.cursor_col = 4;
        input.paste("XY");
        assert_eq!(input.get_text(), "XYabXYcdef\n\n");

        // Paste at the end of the line (one column past the last row).
        let first = input.rows[input.head].next;
        input.cursor_row = first;
        input.cursor_col = input.rows[first].width;
        input.paste("XY");
        assert_eq!(input.get_text(), "XYabXYcdefXY\n\n");
    }
}
