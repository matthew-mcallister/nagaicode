use std::iter::iter;

use crossterm::cursor::{MoveTo, MoveToNextLine};
use crossterm::style::{ContentStyle, PrintStyledContent, StyledContent};
use crossterm::{QueueableCommand};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const TAB_WIDTH: usize = 4;

fn expand_tabs(start_col: usize, line: &str) -> impl Iterator<Item = (&str, usize)> + '_ {
    iter!(move || {
        let mut column = start_col;
        for grapheme in line.graphemes(true) {
            if grapheme == "\t" {
                let spaces = TAB_WIDTH - column % TAB_WIDTH;
                for _ in 0..spaces {
                    column += 1;
                    yield (" ", 1);
                }
            } else {
                let width = grapheme.width();
                column += width;
                yield (grapheme, width);
            }
        }
    })()
}

#[derive(Debug)]
struct Buffer {
    max_width: usize,
    width: usize,
    output: Vec<String>,
    buffer: String,
}

impl Buffer {
    fn flush(&mut self) {
        let capacity = self.buffer.capacity();
        let buffer = std::mem::replace(&mut self.buffer, String::with_capacity(capacity));
        self.output.push(buffer);
        self.width = 0;
    }

    fn push(&mut self, s: &str, width: usize) {
        if self.width + width > self.max_width {
            self.flush();
        }
        self.buffer.push_str(s);
        self.width += width;
    }

    fn push_word<'a>(&'a mut self, word: &str, width: usize) {
        debug_assert!(self.width + width <= self.max_width);
        self.buffer.push_str(word);
        self.width += width;
    }
}

/// Returns true if the text is larger than `columns` in width.
fn overflows(text: &str, columns: usize) -> bool {
    let mut width = 0;
    for g in text.graphemes(true) {
        width += g.width();
        if width > columns { return true; }
    }
    false
}

/// Wraps given text at word boundaries to fit within the maximum allowed
/// width. Returns each resulting line as a string. Tabs will be expanded to
/// spaces.
///
/// Details:
/// - Whitespace is preserved and will cause a line break where it overflows
/// - Tabs are expanded to spaces
/// - Overflowing words will be placed on the next line
/// - Words too long to fit on one line will be broken where they overflow the
///   margin
fn wrap_text(max_width: usize, text: &str) -> Vec<String> {
    assert!(max_width >= 2);

    let mut buffer = Buffer {
        max_width,
        width: 0,
        output: vec![],
        buffer: String::with_capacity(2 * max_width),
    };

    for mut line in text.lines() {
        let mut src_col = 0;

        while !line.is_empty() {
            // Process a word
            let end = line.char_indices()
                .find(|(_, c)| c.is_whitespace())
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            let word = &line[..end];
            if !word.is_empty() {
                line = &line[end..];

                if buffer.width > 0 && overflows(&word, max_width - buffer.width) {
                    buffer.flush();
                }

                for g in word.graphemes(true) {
                    let w = g.width();
                    buffer.push(g, w);
                    src_col += w;
                }
            }

            // Process whitespace
            let end = line.char_indices()
                .find(|(_, c)| !c.is_whitespace())
                .map(|(i, _)| i)
                .unwrap_or(line.len());
            let whitespace = &line[..end];
            if !whitespace.is_empty() {
                line = &line[end..];
                for (g, w) in expand_tabs(src_col, whitespace) {
                    buffer.push(g, w);
                    src_col += w;
                }
            }
        }

        buffer.flush();
    }

    buffer.output
}

/// Truncates a single line of text at the given column width.
fn truncate_text(max_width: usize, line: &str) -> String {
    let mut out = String::with_capacity(2 * line.len());
    let mut width = 0;
    out.extend(
        expand_tabs(0, line)
            .take_while(move |(_, w)| {
                width += w;
                width <= max_width
            })
            .map(|(g, _)| g)
    );
    out
}

/// Preformatted text, suitable for rendering. Preformatted text may be
/// translated or clipped to a vertical window, but resizing horizontally
/// or changing styles requires a recomputation.
#[derive(Debug)]
pub struct Preformatted {
    lines: Vec<Vec<StyledContent<String>>>,
    width: usize,
}

impl Preformatted {
    pub fn wrapped(
        max_width: usize,
        text: &str,
        style: ContentStyle,
    ) -> Self {
        let lines = wrap_text(max_width, text)
            .into_iter()
            .map(|line| vec![StyledContent::new(style, line)])
            .collect();
        Self {
            lines,
            width: max_width,
        }
    }

    pub fn truncated(
        max_width: usize,
        text: &str,
        style: ContentStyle,
    ) -> Self {
        let lines = text.lines()
            .map(|line| vec![StyledContent::new(style, truncate_text(max_width, line))])
            .collect();
        Self {
            lines,
            width: max_width,
        }
    }

    pub fn height(&self) -> usize {
        self.lines.len()
    }

    pub fn width(&self) -> usize {
        self.width
    }
}

#[derive(Debug)]
pub struct DrawPreformatted<'p> {
    pub pre: &'p Preformatted,
    pub x: u16,
    pub y: u16,
    pub start_line: usize,
    pub end_line: usize,
}

impl<'p> crossterm::Command for DrawPreformatted<'p> {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        let x = self.x;
        let mut y = self.y;
        for line in self.pre.lines[self.start_line..self.end_line].iter() {
            crossterm::Command::write_ansi(&MoveTo(x, y), f)?;
            for styled in line.iter() {
                let sc = StyledContent::new(*styled.style(), styled.content().as_str());
                crossterm::Command::write_ansi(&PrintStyledContent(sc), f)?;
            }
            y += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap(width: usize, text: &str) -> String {
        let mut s = String::new();
        for line in wrap_text(width, text) {
            s.push_str(&line);
            s.push('\n');
        }
        s
    }

    #[test]
    fn test_cases() {
        assert_eq!(wrap(10, ""), "");
        assert_eq!(wrap(10, "hello"), "hello\n");
        assert_eq!(wrap(20, "hello world "), "hello world \n");
        assert_eq!(wrap(10, "hello world "), "hello \nworld \n");
        assert_eq!(wrap(5, "hello word"), "hello\n word\n");
        assert_eq!(wrap(6, "hello word"), "hello \nword\n");
        assert_eq!(wrap(3, "abcdef"), "abc\ndef\n");
        assert_eq!(wrap(2, "abcdef"), "ab\ncd\nef\n");
        assert_eq!(
            wrap(10, "hello aabbaabbaabbaabbaabbaabb"),
            "hello \naabbaabbaa\nbbaabbaabb\naabb\n",
        );
        assert_eq!(wrap(5, "hello"), "hello\n");
        assert_eq!(wrap(10, "      "), "      \n");
        assert_eq!(wrap(3, "      "), "   \n   \n");
        assert_eq!(wrap(10, "hello\nworld"), "hello\nworld\n");
        assert_eq!(wrap(10, "hello\n\nworld"), "hello\n\nworld\n");
        assert_eq!(wrap(10, "hello world foo"), "hello \nworld foo\n");
        assert_eq!(wrap(10, "\n"), "\n");
        assert_eq!(wrap(10, "\n\n"), "\n\n");
        assert_eq!(wrap(8, "asdf\t"), "asdf    \n");
        assert_eq!(wrap(6, "asdf\t"), "asdf  \n  \n");
    }
}
