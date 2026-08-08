use std::fmt;
use std::iter::iter;

use crossterm::Command;
use crossterm::style::{Color, ResetColor, SetBackgroundColor};

use crate::text::SPACES;
use crate::ui::Component;

/// Adds padding around a UI component. Also styles the background.
#[derive(Debug)]
struct Padded<C> {
    width: usize,
    height: usize,
    h_padding: usize,
    v_padding: usize,
    background_color: Color,
    inner: C,
}

enum PaddedRow<R> {
    Fill { background: Color, width: usize },
    Inner {
        background: Color,
        left: usize,
        right: usize,
        inner: R,
    },
}

impl<R: Command> Command for PaddedRow<R> {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        match self {
            PaddedRow::Fill { background, width } => {
                SetBackgroundColor(*background).write_ansi(f)?;
                write_spaces(f, *width)?;
                ResetColor.write_ansi(f)?;
            }
            PaddedRow::Inner {
                background,
                left,
                right,
                inner,
            } => {
                SetBackgroundColor(*background).write_ansi(f)?;
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
}

fn write_spaces(f: &mut impl fmt::Write, count: usize) -> fmt::Result {
    let mut remaining = count;
    while remaining != 0 {
        let n = remaining.min(SPACES.len());
        f.write_str(&SPACES[..n])?;
        remaining -= n;
    }
    Ok(())
}
