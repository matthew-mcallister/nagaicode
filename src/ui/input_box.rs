use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Position within a subline. May point to either column of a character
/// which spans multiple columns. May point one past the end of the final
/// subline of a line.
#[derive(Debug, Clone, Copy)]
struct SublinePos {
    /// Index into character array
    character: usize,
    // Column within subline
    column: usize,
}

/// Character encoded as a grapheme cluster
#[derive(Debug)]
struct Character {
    /// Position in UTF-8 stream of this subline
    offset: u16,
    /// Length in bytes (equal to chars[i + 1].offset - chars[i].offset)
    len: u8,
    /// Width in columns (XXX: should always be 1 or 2; combining and
    /// zero-width characters should be rendered as at least one column wide).
    width: u8,
}

/// Subline of a line as determined by the text wrapping routine.
#[derive(Debug)]
struct Subline {
    /// Number of subline within line
    num: usize,
    /// True if this is the last subline of a line
    last: bool,
    /// Offset of first codepoint
    unicode_start: usize,
    /// Offset one past last codepoint
    unicode_end: usize,
    /// Character segmentation
    characters: Vec<Character>,
    /// Total width in columns
    columns: usize,
    /// Unicode stream of line, present on the first subline of a line
    unicode: Option<String>,
}

impl Subline {
    /// Returns the final position of a subline. The return value depends on
    /// whether or not the subline is the last subline in the line. For the
    /// last subline, the final position is one past the final column in the
    /// subline. For all other sublines, the final position is at the final
    /// column in the subline.
    fn final_pos(&self) -> SublinePos {
        if self.last {
            SublinePos {
                character: self.characters.len(),
                column: self.columns,
            }
        } else {
            SublinePos {
                character: self.characters.len().saturating_sub(1),
                column: self.columns.saturating_sub(1),
            }
        }
    }

    /// Finds the character index at a given column within a subline, or the
    /// final position in the subline if the column is past the end.
    fn char_at_col(&self, col: usize) -> usize {
        if col >= self.columns {
            return self.final_pos().character;
        }
        let mut col_so_far = 0;
        for (i, ch) in self.characters.iter().enumerate() {
            col_so_far += ch.width as usize;
            if col < col_so_far {
                return i;
            }
        }
        self.final_pos().character
    }
}

#[derive(Debug)]
struct InputBox {
    width: usize,
    max_height: usize,
    sublines: Vec<Subline>,
    // First visible subline
    first_subline: usize,
    // Total number of sublines; if greater than max_height, then text will be
    // truncated vertically.
    num_sublines: usize,
    cursor_line: usize,
    cursor_subline: usize,
    /// Cursor position. Column may actually point past the end of the subline.
    /// Moving up/down preserves the column but left/right does not.
    cursor_pos: SublinePos,
}

impl InputBox {
    /// Last visible subline
    fn last_subline(&self) -> usize {
        std::cmp::min(self.num_sublines - 1, self.first_subline + self.max_height)
    }

    /// Moves the cursor up one subline. Preserves the column of the cursor.
    /// If already at the very first subline, moves to the start of the line.
    /// Scrolls the visible window if at the top.
    fn move_up(&mut self) {
        todo!()
    }

    /// Moves the cursor down one subline. Preserves the column of the cursor.
    /// If already at the very last subline, moves to the end of the line.
    /// Scrolls the visible window if at the bottom.
    fn move_down(&mut self) {
        todo!()
    }

    /// Moves the cursor left by one character. If at the start of a subline,
    /// goes to the end of the previous subline.
    fn move_left(&mut self) {
        todo!()
    }

    /// Moves the cursor right by one character. If at the end of a subline,
    /// goes to the start of the next subline.
    fn move_right(&mut self) {
        todo!()
    }

    /// Gets the unicode data for a subline
    fn get_unicode(&self, subline: usize) -> &str {
        let sl = &self.sublines[subline];
        let i = subline - sl.num;
        let unicode = self.sublines[i].unicode.as_ref().unwrap();
        &unicode[sl.unicode_start..sl.unicode_end]
    }

    /// Recomputes the line containing the given subline from its unicode
    /// stream.
    fn recompute_line(&mut self, subline: usize) {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn character(width: u8) -> Character {
        Character {
            offset: 0,
            len: 1,
            width,
        }
    }

    fn subline(line: &str, last: bool) -> Subline {
        Subline {
            num: 0,
            last,
            unicode_start: 0,
            unicode_end: line.len(),
            characters: line
                .graphemes(true)
                .map(|g| character(g.width() as u8))
                .collect(),
            columns: line.width(),
            unicode: Some(line.to_string()),
        }
    }

    #[test]
    fn char_at_col_basic() {
        let sl = subline("abc", true);
        assert_eq!(sl.char_at_col(0), 0);
        assert_eq!(sl.char_at_col(1), 1);
        assert_eq!(sl.char_at_col(2), 2);
        assert_eq!(sl.char_at_col(3), 3);
        assert_eq!(sl.char_at_col(4), 3);

        let sl = subline("a界b", true);
        assert_eq!(sl.char_at_col(0), 0);
        assert_eq!(sl.char_at_col(1), 1);
        assert_eq!(sl.char_at_col(2), 1);
        assert_eq!(sl.char_at_col(3), 2);
        assert_eq!(sl.char_at_col(4), 3);

        let sl = subline("a장b", true);
        assert_eq!(sl.char_at_col(0), 0);
        assert_eq!(sl.char_at_col(1), 1);
        assert_eq!(sl.char_at_col(2), 1);
        assert_eq!(sl.char_at_col(3), 2);
        assert_eq!(sl.char_at_col(4), 3);

        let sl = subline("q\u{308}x", true);
        assert_eq!(sl.char_at_col(0), 0);
        assert_eq!(sl.char_at_col(1), 1);
        assert_eq!(sl.char_at_col(2), 2);

        let sl = subline("ab", false);
        assert_eq!(sl.char_at_col(0), 0);
        assert_eq!(sl.char_at_col(1), 1);
        assert_eq!(sl.char_at_col(2), 1);

        let sl = subline("", true);
        assert_eq!(sl.char_at_col(0), 0);
    }
}