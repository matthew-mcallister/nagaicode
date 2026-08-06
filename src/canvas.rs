use compact_str::CompactString;
use crossterm::Command;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::style::{ContentStyle, Print, SetStyle};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// A single terminal cell.
// XXX: Optimize me!
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cell {
    pub grapheme: CompactString,
    pub style: ContentStyle,
}

impl Cell {
    fn space() -> Self {
        Self {
            grapheme: CompactString::new(" "),
            style: ContentStyle::default(),
        }
    }
}

/// Paintable virtual terminal. Solves overdraw and terminal flickering. Some
/// terminal behavior is not implemented exactly (e.g. partially overwriting
/// wide characters), so the caller should take care when painting and not
/// rely on such minutiae.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Canvas {
    width: u16,
    height: u16,
    cursor_pos: Option<(u16, u16)>,
    /// Grapheme data. A `None` cell is occupied by a wide grapheme to its left
    data: Vec<Option<Cell>>,
}

impl Canvas {
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            width,
            height,
            cursor_pos: None,
            data: vec![Some(Cell::space()); width as usize * height as usize],
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    fn get(&self, x: u16, y: u16) -> Option<&Cell> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.data[self.index(x, y)].as_ref()
    }

    fn index(&self, x: u16, y: u16) -> usize {
        y as usize * self.width as usize + x as usize
    }

    pub fn set_cursor_pos(&mut self, x: u16, y: u16) {
        self.cursor_pos = Some((x, y));
    }

    /// Writes a grapheme to the terminal.
    pub fn write(
        &mut self,
        x: u16,
        y: u16,
        grapheme: &str,
        width: u16,
        style: ContentStyle,
    ) {
        assert!(x + width <= self.width);
        let i = self.index(x, y);
        self.data[i] = Some(Cell {
            grapheme: grapheme.into(),
            style,
        });
        for i in 1..width {
            let j = self.index(x + i, y);
            self.data[j] = None;
        }
    }

    /// Writes a horizontal line of text to the terminal. The text will be
    /// parsed automatically. It must not contain tabs or line breaks.
    pub fn write_str(
        &mut self,
        mut x: u16,
        y: u16,
        text: &str,
        style: ContentStyle,
    ) {
        for grapheme in text.graphemes(true) {
            let width = grapheme.width() as u16;
            self.write(x, y, grapheme, width, style);
            x += width;
        }
    }

    /// Clears the entire terminal to a solid color.
    pub fn clear_all(&mut self, style: ContentStyle) {
        self.clear_rect(0, 0, self.width, self.height, style);
    }

    /// Clears a rectangle.
    pub fn clear_rect(&mut self, x: u16, y: u16, w: u16, h: u16, style: ContentStyle) {
        assert!(x + w <= self.width);
        assert!(y + h <= self.height);
        for i in y..y + h {
            for j in x..x + w {
                let idx = self.index(j, i);
                self.data[idx] = Some(Cell {
                    grapheme: " ".into(),
                    style,
                });
            }
        }
    }
}

impl Command for Canvas {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        let mut style = ContentStyle::default();
        Hide.write_ansi(f)?;
        for y in 0..self.height {
            MoveTo(0, y).write_ansi(f)?;
            for x in 0..self.width {
                let Some(cell) = self.data[self.index(x, y)].as_ref() else {
                    continue;
                };
                if cell.style != style {
                    SetStyle(cell.style).write_ansi(f)?;
                    style = cell.style;
                }
                Print(cell.grapheme.as_str()).write_ansi(f)?;
            }
        }
        if let Some((x, y)) = self.cursor_pos {
            MoveTo(x, y).write_ansi(f)?;
            Show.write_ansi(f)?;
        }
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crossterm::style::Color;

    #[test]
    fn test_paint() {
        let mut canvas = Canvas::new(20, 5);
        let bg = ContentStyle {
            background_color: Some(Color::White),
            ..Default::default()
        };
        let rect = ContentStyle {
            background_color: Some(Color::Blue),
            ..Default::default()
        };
        let text = ContentStyle {
            foreground_color: Some(Color::Green),
            ..Default::default()
        };
        canvas.clear_all(bg);
        canvas.clear_rect(1, 1, 18, 3, rect);
        canvas.write_str(2, 2, "Hello world", text);
        let mut out = String::new();
        canvas.write_ansi(&mut out).unwrap();
        let expected = concat!(
            "\x1b[?25l",
            "\x1b[1;1H\x1b[48;5;15m",
            "                    ",
            "\x1b[2;1H",
            " ",
            "\x1b[48;5;12m",
            "                  ",
            "\x1b[48;5;15m",
            " ",
            "\x1b[3;1H",
            " ",
            "\x1b[48;5;12m",
            " ",
            "\x1b[38;5;10mHello world",
            "\x1b[48;5;12m",
            "      ",
            "\x1b[48;5;15m",
            " ",
            "\x1b[4;1H",
            " ",
            "\x1b[48;5;12m",
            "                  ",
            "\x1b[48;5;15m",
            " ",
            "\x1b[5;1H",
            "                    ",
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn test_cursor() {
        let mut canvas = Canvas::new(20, 5);
        canvas.set_cursor_pos(3, 2);
        let mut out = String::new();
        canvas.write_ansi(&mut out).unwrap();
        assert_eq!(
            out,
            concat!(
                "\x1b[?25l",
                "\x1b[1;1H",
                "                    ",
                "\x1b[2;1H",
                "                    ",
                "\x1b[3;1H",
                "                    ",
                "\x1b[4;1H",
                "                    ",
                "\x1b[5;1H",
                "                    ",
                "\x1b[3;4H\x1b[?25h",
            )
        );
    }
}
