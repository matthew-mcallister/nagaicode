use crossterm::cursor::MoveTo;
use crossterm::style::{ContentStyle, PrintStyledContent, StyledContent};

use crate::text::Row;

#[derive(Debug)]
pub struct DrawRectangle {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub style: ContentStyle,
}

impl crossterm::Command for DrawRectangle {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        use crossterm::cursor::MoveTo;
        use crossterm::style::{PrintStyledContent, StyledContent};

        let fill = " ".repeat(self.width as usize);
        for dy in 0..self.height {
            crossterm::Command::write_ansi(&MoveTo(self.x, self.y + dy), f)?;
            let sc = StyledContent::new(self.style, fill.as_str());
            crossterm::Command::write_ansi(&PrintStyledContent(sc), f)?;
        }
        Ok(())
    }
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
    fn from_rows(
        rows: impl Iterator<Item = Row>,
        style: ContentStyle,
    ) -> Self {
        let mut lines = Vec::new();
        let mut width = 0;

        for row in rows {
            let mut line: String = String::with_capacity(2 * row.graphemes.len());
            line.extend(row.graphemes.iter().map(|g| g.formatted()));
            let line_width = row.graphemes.iter().map(|g| g.width as usize).sum();
            width = width.max(line_width);
            lines.push(vec![StyledContent::new(style, line)]);
        }

        Self { lines, width }
    }

    pub fn wrapped(
        max_width: usize,
        text: &str,
        style: ContentStyle,
    ) -> Self {
        Self::from_rows(
            text.lines()
                .flat_map(|line| crate::text::wrap_line(max_width, line)),
            style,
        )
    }

    pub fn truncated(
        max_width: usize,
        text: &str,
        style: ContentStyle,
    ) -> Self {
        Self::from_rows(
            text.lines()
                .map(|line| crate::text::truncate_line(max_width, line)),
            style,
        )
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
