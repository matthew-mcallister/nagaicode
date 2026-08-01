use std::iter::iter;

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

/// Wraps given text at word boundaries to fit within the maximum allowed
/// width. Returns each resulting line as a string. Tabs will be expanded to
/// spaces.
///
/// Details:
/// - Whitespace is preserved and will cause a line break where it overflows
/// - Tabs are expanded to spaces
/// - Overflowing words will be placed on the next line, if they fit
/// - Words too long to fit on one line will be broken exactly where they
///   overflow the margin
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
                let word_width = word.width();
                line = &line[end..];
                src_col += word_width;

                if word_width <= max_width {
                    buffer.push(word, word_width);
                } else {
                    // Word is too long to fit on one line
                    for g in word.graphemes(true) {
                        buffer.push(g, g.width());
                    }
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
/// or changing text styles requires a recomputation.
#[derive(Debug)]
pub struct Preformatted {
    lines: Vec<String>,
    width: usize,
    height: usize,
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
    fn test_empty_input() {
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
            "hello aabb\naabbaabbaa\nbbaabbaabb\n",
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
