use std::fmt;
use std::iter::iter;

use crossterm::Command;
use crossterm::style::{Color, ResetColor, SetBackgroundColor};

use crate::ui::write_spaces;
use crate::ui::Component;

/// Adds padding around a UI component. Also styles the background.
#[derive(Debug)]
struct Padded<C> {
    pub width: usize,
    pub height: usize,
    pub h_padding: usize,
    pub v_padding: usize,
    pub background_color: Option<Color>,
    pub inner: C,
}

impl<C> Padded<C> {
    pub fn inner(&self) -> &C {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &C {
        &self.inner
    }

    pub fn into_inner(self) -> C {
        self.inner
    }
}

enum PaddedRow<R> {
    Fill { background: Option<Color>, width: usize },
    Inner {
        background: Option<Color>,
        left: usize,
        right: usize,
        inner: R,
    },
}

impl<R: Command> Command for PaddedRow<R> {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        match self {
            PaddedRow::Fill { background, width } => {
                if let Some(bg) = background {
                    SetBackgroundColor(*bg).write_ansi(f)?;
                }
                write_spaces(f, *width)?;
                ResetColor.write_ansi(f)?;
            }
            PaddedRow::Inner {
                background,
                left,
                right,
                inner,
            } => {
                if let Some(bg) = background {
                    SetBackgroundColor(*bg).write_ansi(f)?;
                }
                write_spaces(f, *left)?;
                inner.write_ansi(f)?;
                write_spaces(f, *right)?;
                ResetColor.write_ansi(f)?;
            }
        }
        Ok(())
    }
}

impl<C: Component> Component for Padded<C> {
    type Row<'a> = PaddedRow<C::Row<'a>> where C: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where C: 'a;

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        let mut inner = self.inner.drawable_rows();
        let background = self.background_color;
        let width = self.width;
        let h_padding = self.h_padding;
        let height = self.height;
        Box::new(iter!(move || {
            for _ in 0..self.v_padding {
                yield PaddedRow::Fill { background, width };
            }
            for _ in 2..height {
                if let Some(row) = inner.next() {
                    yield PaddedRow::Inner {
                        background,
                        left: h_padding,
                        right: h_padding,
                        inner: row,
                    };
                } else {
                    yield PaddedRow::Fill { background, width };
                }
            }
            for _ in 0..self.v_padding {
                yield PaddedRow::Fill { background, width };
            }
        })())
    }

    fn set_width(&mut self, width: usize) {
        self.width = width;
        self.inner.set_width(width - 2 * self.h_padding);
    }

    fn set_height(&mut self, height: usize) {
        self.height = height;
        self.inner.set_height(height - 2 * self.v_padding);
    }

    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }
}
