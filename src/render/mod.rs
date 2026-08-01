mod text;

use crossterm::style::ContentStyle;

pub use self::text::Preformatted;

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
