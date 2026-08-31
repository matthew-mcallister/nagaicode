use std::collections::VecDeque;
use std::num::NonZeroUsize;

use compact_str::CompactString;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const TAB_WIDTH: usize = 4;
pub const SPACES: &str = const {
    let Ok(s) = std::str::from_utf8(&[b' '; 2048]) else { panic!() };
    s
};

/// Removes trailing '\r' from a string.
pub fn strip_cr(s: &str) -> &str {
    s.strip_suffix('\r').unwrap_or(s)
}

/// Single grapheme; generally corresponds to a unicode grapheme cluster. Tabs
/// are treated as variable width graphemes and rendered as spaces
#[derive(Debug)]
pub struct Grapheme {
    /// One or more codepoints of data
    pub data: CompactString,
    /// Width in columns
    pub width: u8,
}

impl Grapheme {
    /// Zero-width newline grapheme. Marks the end of a line.
    pub fn newline() -> Self {
        Self {
            data: CompactString::new("\n"),
            width: 0,
        }
    }

    pub fn formatted(&self) -> &str {
        match &self.data[..] {
            "\t" => &SPACES[..self.width as usize],
            "\n" => "",
            _ => &self.data,
        }
    }
}

#[derive(Debug, Default)]
pub struct Row {
    pub graphemes: Vec<Grapheme>,
    pub width: usize,
}

impl Row {
    /// Concatenates all graphemes into a string and pads with spaces to fill
    /// the desired width.
    pub fn to_padded_string(&self, padded_width: usize) -> String {
        let mut s = String::with_capacity(2 * padded_width);
        for grapheme in &self.graphemes {
            // Omit the trailing zero-width newline marker, if present.
            if grapheme.data == "\n" {
                continue;
            }
            s.push_str(grapheme.formatted());
        }
        if self.width < padded_width {
            s.push_str(&SPACES[..padded_width - self.width]);
        }
        s
    }
}

impl std::fmt::Display for Row {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for grapheme in &self.graphemes {
            f.write_str(grapheme.formatted())?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
struct Breakpoint {
    offset: usize,
    set: bool,
}

#[derive(Debug, Default)]
pub struct Analysis {
    max_width: usize,
    tab_width: usize,
    /// Queue of upcoming potential break points
    break_points: VecDeque<Breakpoint>,
    /// `(grapheme_index, column)` for currently set breakpoint
    break_point: Option<(NonZeroUsize, NonZeroUsize)>,
    /// Previous rows
    rows: Vec<Row>,
    /// Current row
    row: Row,
    src_col: usize,
    src_offset: usize,
}

impl Analysis {
    fn new(
        max_width: usize,
        tab_width: usize,
        break_points: VecDeque<Breakpoint>,
    ) -> Self {
        assert!(tab_width < 128, "max tab width is 128");
        Self {
            max_width,
            tab_width,
            break_points,
            ..Default::default()
        }
    }

    fn cur_column(&self) -> usize {
        self.row.width
    }

    fn end_row(&mut self) {
        let mut old_row = std::mem::replace(&mut self.row, Row {
            graphemes: Vec::with_capacity(self.max_width),
            width: 0,
        });
        if let Some((index, column)) = self.break_point.take() {
            self.row.graphemes = old_row.graphemes.split_off(index.get());
            self.row.width = old_row.width - column.get();
            old_row.width = column.get();
        } else {
            self.row.width = 0;
        }
        self.rows.push(old_row);
    }

    // Pops any waiting break point
    fn flush_break_point(&mut self) {
        if let Some(Breakpoint { set, .. }) = self.break_points
            .pop_front_if(|bp| bp.offset == self.src_offset)
        {
            if set {
                let index = NonZeroUsize::new(self.row.graphemes.len()).unwrap();
                let width = NonZeroUsize::new(self.cur_column()).unwrap();
                self.break_point = Some((index, width));
            } else {
                self.break_point = None;
            }
        }
    }

    fn push(&mut self, grapheme: &str) {
        assert_ne!(grapheme, "\n");
        self.flush_break_point();
        let width = if grapheme == "\t" {
            self.tab_width - self.src_col % self.tab_width
        } else {
            grapheme.width()
        };
        if self.cur_column() + width > self.max_width {
            self.end_row();
        }
        self.row.graphemes.push(Grapheme {
            data: CompactString::new(grapheme),
            width: width as u8,
        });
        self.row.width += width;
        self.src_col += width;
        self.src_offset += grapheme.len();
    }

    fn finish(mut self) -> Vec<Row> {
        self.rows.push(self.row);
        self.rows
    }
}

fn find_break_points(line: &str) -> VecDeque<Breakpoint> {
    let mut chars = line.char_indices();
    let Some((_, mut prev)) = chars.next() else { return Default::default() };
    let mut break_points = VecDeque::new();
    for (offset, c) in chars {
        match (prev.is_whitespace(), c.is_whitespace()) {
            (true, false) => {
                break_points.push_back(Breakpoint { offset, set: true });
            }
            (false, true) => {
                break_points.push_back(Breakpoint { offset, set: false });
            }
            _ => {}
        }
        prev = c;
    }
    break_points
}

/// Text wrapping routine. Used for rendering content as well as for driving
/// the input box.
///
/// Details:
/// - Whitespace is preserved and will cause a line break where it overflows
/// - Tabs are treated as a single, variable-width grapheme and rendered as
///   spaces
/// - Overflowing words will be placed on the next line, unless they are
///   already at the beginning of the line
/// - Words too long to fit on one line will be broken where they overflow
/// - The final row ends with a zero-width newline grapheme
pub fn wrap_line(max_width: usize, line: &str) -> Vec<Row> {
    assert!(max_width >= TAB_WIDTH);
    let break_points = find_break_points(line);
    let mut seg = Analysis::new(max_width, TAB_WIDTH, break_points);
    seg.max_width = max_width;
    for g in line.graphemes(true) {
        seg.push(g);
    }
    let mut rows = seg.finish();
    rows.last_mut().unwrap().graphemes.push(Grapheme::newline());
    rows
}

/// Naive line wrapping: text is wrapped exactly where it overflows the line.
/// This is used for preformatted/code rendering.
pub fn wrap_line_naive(max_width: usize, line: &str) -> Vec<Row> {
    assert!(max_width >= TAB_WIDTH);
    let mut rows = Vec::new();
    let mut row = Row::default();
    let mut width = 0;

    for grapheme in line.graphemes(true) {
        assert_ne!(grapheme, "\n");
        let grapheme_width = if grapheme == "\t" {
            TAB_WIDTH - width % TAB_WIDTH
        } else {
            grapheme.width()
        };
        if width + grapheme_width > max_width {
            row.width = width;
            rows.push(row);
            row = Row::default();
            width = 0;
        }
        row.graphemes.push(Grapheme {
            data: CompactString::new(grapheme),
            width: grapheme_width as u8,
        });
        width += grapheme_width;
    }

    row.width = width;
    rows.push(row);
    rows.last_mut().unwrap().graphemes.push(Grapheme::newline());
    rows
}

/// Naive line wrapping in reverse direction: rows are filled starting from
/// the end of the line. Returns rows in reverse order, so the first row
/// contains the end of the line, filled to the full width wherever possible.
/// Tab widths are computed in forward direction from the start of the line.
pub fn wrap_line_reverse(max_width: usize, line: &str) -> Vec<Row> {
    assert!(max_width >= TAB_WIDTH);
    let graphemes = truncate_line(usize::MAX, line).graphemes;

    let mut rows = Vec::new();
    let mut row = Row::default();
    let mut width = 0;
    for grapheme in graphemes.into_iter().rev() {
        let grapheme_width = grapheme.width as usize;
        if width + grapheme_width > max_width {
            row.graphemes.reverse();
            row.width = width;
            rows.push(std::mem::take(&mut row));
            width = 0;
        }
        row.graphemes.push(grapheme);
        width += grapheme_width;
    }
    row.graphemes.reverse();
    row.width = width;
    rows.push(row);
    rows
}

/// Wraps text to rows of `max_width` columns, ellipsizing it if it exceeds
/// `max_rows` rows. Returns the head rows and, if the text overflowed, tail
/// rows to be displayed after an ellipsis marker. Any line in the tail which
/// does not fit fully in the output will be reverse-wrapped so it fills the
/// final row.
pub fn ellipsize(max_width: usize, max_rows: usize, text: &str) -> (Vec<Row>, Option<Vec<Row>>) {
    let half = max_rows / 2;
    // This is not a hard upper bound, but it is a generous limit.
    let limit = 6 * (max_rows + 1) * max_width;
    let head_end = text.floor_char_boundary(limit);
    let mut rows: Vec<Row> = text[..head_end]
        .lines()
        .flat_map(|line| wrap_line_naive(max_width, line))
        .collect();
    let tail_rows = if rows.len() > max_rows {
        rows.truncate(half);
        let tail_start = text.floor_char_boundary(text.len().saturating_sub(limit));
        let mut tail_rows: Vec<Row> = Vec::new();
        for line in text[tail_start..].lines().rev() {
            let wrapped = wrap_line_naive(max_width, line);
            if tail_rows.len() + wrapped.len() > half {
                tail_rows.extend(wrap_line_reverse(max_width, line));
            } else {
                tail_rows.extend(wrapped);
            }
            if tail_rows.len() > half {
                break;
            }
        }
        tail_rows.truncate(half);
        tail_rows.reverse();
        Some(tail_rows)
    } else {
        None
    };
    (rows, tail_rows)
}

/// Truncates a single line to fit within the maximum width. Also expands tabs.
pub fn truncate_line(max_width: usize, line: &str) -> Row {
    let mut row = Row::default();
    let mut width = 0;

    for grapheme in line.graphemes(true) {
        assert_ne!(grapheme, "\n");
        let grapheme_width = if grapheme == "\t" {
            TAB_WIDTH - width % TAB_WIDTH
        } else {
            grapheme.width()
        };
        if width + grapheme_width > max_width {
            break;
        }
        row.graphemes.push(Grapheme {
            data: CompactString::new(grapheme),
            width: grapheme_width as u8,
        });
        width += grapheme_width;
    }

    row.width = width;
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap(width: usize, text: &str) -> String {
        let mut s = String::with_capacity(2 * text.len());
        for line in text.lines() {
            let rows = wrap_line(width, line);
            for row in rows {
                for g in row.graphemes {
                    s.push_str(g.formatted());
                }
                s.push('\n');
            }
        }
        s
    }

    #[test]
    fn test_wrapping() {
        assert_eq!(wrap(10, ""), "");
        assert_eq!(wrap(10, "hello"), "hello\n");
        assert_eq!(wrap(20, "hello world "), "hello world \n");
        assert_eq!(wrap(10, "hello world "), "hello \nworld \n");
        assert_eq!(wrap(5, "hello word"), "hello\n word\n");
        assert_eq!(wrap(6, "hello word"), "hello \nword\n");
        assert_eq!(wrap(4, "abcdefgh"), "abcd\nefgh\n");
        assert_eq!(wrap(6, "abcdefgh"), "abcdef\ngh\n");
        assert_eq!(
            wrap(10, "hello aabbaabbaabbaabbaabbaabb"),
            "hello \naabbaabbaa\nbbaabbaabb\naabb\n",
        );
        assert_eq!(wrap(5, "hello"), "hello\n");
        assert_eq!(wrap(10, "      "), "      \n");
        assert_eq!(wrap(4, "        "), "    \n    \n");
        assert_eq!(wrap(10, "hello\nworld"), "hello\nworld\n");
        assert_eq!(wrap(10, "hello\n\nworld"), "hello\n\nworld\n");
        assert_eq!(wrap(10, "hello world foo"), "hello \nworld foo\n");
        assert_eq!(wrap(10, "\n"), "\n");
        assert_eq!(wrap(10, "\n\n"), "\n\n");
        assert_eq!(wrap(8, "asdf\t"), "asdf    \n");
        assert_eq!(wrap(6, "asdf\t"), "asdf\n    \n");
    }

    fn wrap_naive(width: usize, text: &str) -> String {
        let mut s = String::with_capacity(2 * text.len());
        for line in text.lines() {
            let rows = wrap_line_naive(width, line);
            for row in rows {
                for g in row.graphemes {
                    s.push_str(g.formatted());
                }
                s.push('\n');
            }
        }
        s
    }

    #[test]
    fn test_wrap_naive() {
        assert_eq!(wrap_naive(10, ""), "");
        assert_eq!(wrap_naive(10, "hello"), "hello\n");
        assert_eq!(wrap_naive(10, "hello world foo"), "hello worl\nd foo\n");
        assert_eq!(wrap_naive(10, "      "), "      \n");
        assert_eq!(wrap_naive(4, "        "), "    \n    \n");
        assert_eq!(wrap_naive(10, "asdf\t"), "asdf    \n");
        assert_eq!(wrap_naive(6, "asdf\t"), "asdf\n    \n");
    }

    fn wrap_rev(width: usize, text: &str) -> Vec<String> {
        wrap_line_reverse(width, text)
            .into_iter()
            .map(|row| row.to_string())
            .collect()
    }

    #[test]
    fn test_wrap_reverse() {
        assert_eq!(wrap_rev(10, ""), [""]);
        assert_eq!(wrap_rev(10, "hello"), ["hello"]);
        assert_eq!(wrap_rev(10, "hello world foo"), [" world foo", "hello"]);
        assert_eq!(wrap_rev(4, "abcdefgh"), ["efgh", "abcd"]);
        assert_eq!(wrap_rev(4, "abcdefg"), ["defg", "abc"]);
        assert_eq!(wrap_rev(10, "abcdefghij0123456789"), ["0123456789", "abcdefghij"]);
        assert_eq!(wrap_rev(4, "ab界cdef界"), ["ef界", "界cd", "ab"]);
        assert_eq!(wrap_rev(10, "a\tb"), ["a   b"]);
        assert_eq!(wrap_rev(4, "ab\tcd"), ["  cd", "ab"]);
    }

    fn ellipsized(text: &str) -> (Vec<String>, Option<Vec<String>>) {
        let (head, tail) = ellipsize(10, 7, text);
        (
            head.into_iter().map(|row| row.to_string()).collect(),
            tail.map(|rows| rows.into_iter().map(|row| row.to_string()).collect()),
        )
    }

    #[test]
    fn test_ellipsize() {
        // Exactly max_rows rows
        let (head, tail) = ellipsized("line0\nline1\nline2\nline3\nline4\nline5\nline6");
        assert_eq!(head, ["line0", "line1", "line2", "line3", "line4", "line5", "line6"]);
        assert!(tail.is_none());

        // More than max_rows rows across separate lines
        let (head, tail) = ellipsized("line0\nline1\nline2\nline3\nline4\nline5\nline6\nline7\nline8");
        assert_eq!(head, ["line0", "line1", "line2"]);
        assert_eq!(tail.unwrap(), ["line6", "line7", "line8"]);

        // A single very long line fills the final row to the full width
        let long = "aaaaaaaaaabbbbbbbbbbccccccccccddddddddddeeeeeeeeeeffffffffffgggggggggg123";
        let (head, tail) = ellipsized(long);
        assert_eq!(head, ["aaaaaaaaaa", "bbbbbbbbbb", "cccccccccc"]);
        assert_eq!(tail.unwrap(), ["eeeeeeefff", "fffffffggg", "ggggggg123"]);
    }

    fn trunc(width: usize, text: &str) -> String {
        truncate_line(width, text)
            .graphemes
            .iter()
            .map(Grapheme::formatted)
            .collect()
    }

    #[test]
    fn test_truncate() {
        assert_eq!(trunc(10, ""), "");
        assert_eq!(trunc(10, "hello"), "hello");
        assert_eq!(trunc(5, "hello"), "hello");
        assert_eq!(trunc(4, "hello"), "hell");
        assert_eq!(trunc(10, "hello world "), "hello worl");
        assert_eq!(trunc(4, "abcdefgh"), "abcd");

        assert_eq!(trunc(4, "ab界c"), "ab界");
        assert_eq!(trunc(3, "ab界c"), "ab");
        assert_eq!(trunc(2, "ab界c"), "ab");
        assert_eq!(trunc(1, "ab"), "a");

        assert_eq!(trunc(8, "asdf\t"), "asdf    ");
        assert_eq!(trunc(6, "asdf\t"), "asdf");
        assert_eq!(trunc(4, "\tab"), "    ");
        assert_eq!(trunc(8, "a\tb"), "a   b");
        assert_eq!(trunc(4, "a\tb"), "a   ");
        assert_eq!(trunc(2, "\t"), "");
    }
}
