use std::fmt;
use std::iter;

use crossterm::Command;
use crossterm::style::{Color, ResetColor, SetBackgroundColor};

use crate::ui::write_spaces;
use crate::ui::Component;

/// Renders a solid color.
#[derive(Debug)]
pub struct Empty {
    width: usize,
    height: usize,
    background: Option<Color>,
}

#[derive(Debug, Clone)]
pub struct EmptyRow {
    width: usize,
    background: Option<Color>,
}

impl Empty {
    pub fn new(width: usize, height: usize, background: Option<Color>) -> Self {
        Self {
            width,
            height,
            background,
        }
    }

    pub fn height(&self) -> usize {
        self.height
    }
}

impl Command for EmptyRow {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        if let Some(bg) = self.background {
            SetBackgroundColor(bg).write_ansi(f)?;
        }
        write_spaces(f, self.width)?;
        ResetColor.write_ansi(f)?;
        Ok(())
    }
}

impl Component for Empty {
    type Row<'a> = EmptyRow where Self: 'a;
    type RowIter<'a> = iter::Repeat<Self::Row<'a>> where Self: 'a;

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        iter::repeat(EmptyRow {
            width: self.width,
            background: self.background,
        })
    }

    fn set_width(&mut self, width: usize) {
        self.width = width;
    }

    fn set_height(&mut self, height: usize) {
        self.height = height;
    }

    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }
}
