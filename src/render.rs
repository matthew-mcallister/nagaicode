use std::iter::iter;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::style::{StyleSettings, THEME_DARK};

const TAB_WIDTH: usize = 4;

const STYLE_SETTINGS: StyleSettings = StyleSettings {
    theme: THEME_DARK,
    max_width: 99,
};

/// Returns an iterator over `(grapheme, width)` pairs. Also expands tabs to
/// spaces.
fn iter_graphemes(line: &str) -> impl Iterator<Item = (&str, usize)> + '_ {
    iter!(move || {
        let mut column = 0;
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
    inner: String,
    width: usize,
}

impl Buffer {
    fn flush(&mut self) -> String {
        let capacity = self.inner.capacity();
        let result = std::mem::replace(&mut self.inner, String::with_capacity(capacity));
        self.width = 0;
        result
    }

    fn push(&mut self, s: &str, width: usize) {
        self.inner.push_str(s);
        self.width += width;
    }
}

/// Wraps given text at word boundaries to fit within the maximum allowed
/// width. Returns each resulting line as a string. Tabs will be expanded to
/// spaces.
///
/// Details:
/// - Whitespace is preserved and will cause a line break where it overflows
/// - Overflowing words will be placed on the next line, if they fit
/// - Words too long to fit on one line will be broken exactly where they
///   overflow the margin
pub fn wrap_text(max_width: usize, text: &str) -> impl Iterator<Item = String> + '_ {
    assert!(max_width >= 2);
    iter!(move || {
        for line in text.lines() {
            let mut current_offset = 0;
            let mut buffer = Buffer {
                inner: String::with_capacity(6 * max_width),
                width: 0,
            };
            let mut graphemes = iter_graphemes(line);

            loop {
                let word_offset = current_offset;
                let mut word_width = 0;
                let mut ws: Option<(&str, usize)> = None;

                // Consume a word + next whitespace
                let mut word;
                loop {
                    word = &line[word_offset..current_offset];

                    let Some((g, w)) = graphemes.next() else { break };
                    current_offset += g.len();
                    if g.chars().all(|c| c.is_whitespace()) {
                        ws = Some((g, w));
                        break;
                    }
                    word_width += w;
                }

                if !word.is_empty() {
                    if buffer.width + word_width > max_width {
                        if word_width <= max_width {
                            // Break line at word start
                            yield buffer.flush();
                            buffer.push(word, word_width);
                        } else {
                            // Word is too long to fit on one line. Break the
                            // word at max line width.
                            for (g, w) in iter_graphemes(word) {
                                if buffer.width + w > max_width {
                                    yield buffer.flush();
                                }
                                buffer.push(g, w);
                            }
                        }
                    } else {
                        buffer.push(word, word_width);
                    }
                }

                if let Some((ws_g, ws_w)) = ws {
                    if buffer.width + ws_w > max_width {
                        yield buffer.flush();
                    }
                    buffer.push(ws_g, ws_w);
                } else {
                    break;
                }
            }

            yield buffer.inner;
        }
    })()
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
    }
}
