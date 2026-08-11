// TODO: paste mode, disables all horizontal padding

use std::fmt;
use std::iter::iter;

use crossterm::Command;
use crossterm::style::{Color, ResetColor, SetBackgroundColor};

use crate::ui::write_spaces;
use crate::ui::Component;

/// Adds padding around a UI component. Also styles the background.
#[derive(Debug)]
pub struct Padded<C> {
    pub h_padding: usize,
    pub v_padding: usize,
    pub background_color: Option<Color>,
    pub inner: C,
}

impl<C> Padded<C> {
    pub fn new(
        inner: C,
        h_padding: usize,
        v_padding: usize,
        background_color: Option<Color>,
    ) -> Self {
        Self {
            h_padding,
            v_padding,
            background_color,
            inner,
        }
    }

    pub fn inner(&self) -> &C {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut C {
        &mut self.inner
    }

    pub fn into_inner(self) -> C {
        self.inner
    }
}

#[derive(Debug)]
pub enum PaddedRow<R> {
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
                if let Some(bg) = background {
                    SetBackgroundColor(*bg).write_ansi(f)?;
                }
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
    type EventReponse = C::EventReponse;

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        let mut inner = self.inner.drawable_rows();
        let background = self.background_color;
        let width = self.inner.width() + 2 * self.h_padding;
        let h_padding = self.h_padding;
        let v_padding = self.v_padding;
        Box::new(iter!(move || {
            for _ in 0..v_padding {
                yield PaddedRow::Fill { background, width };
            }
            for row in &mut inner {
                yield PaddedRow::Inner {
                    background,
                    left: h_padding,
                    right: h_padding,
                    inner: row,
                };
            }
            for _ in 0..v_padding {
                yield PaddedRow::Fill { background, width };
            }
        })())
    }

    fn set_width(&mut self, width: usize) {
        self.inner.set_width(width.saturating_sub(2 * self.h_padding));
    }

    fn set_height(&mut self, height: usize) {
        self.inner.set_height(height.saturating_sub(2 * self.v_padding));
    }

    fn width(&self) -> usize {
        self.inner.width() + 2 * self.h_padding
    }

    fn height(&self) -> usize {
        self.inner.height() + 2 * self.v_padding
    }

    fn cursor_pos(&self) -> (usize, usize) {
        let (row, col) = self.inner.cursor_pos();
        (row + self.v_padding, col + self.h_padding)
    }

    fn handle_event(&mut self, event: crossterm::event::Event) -> Self::EventReponse {
        self.inner.handle_event(event)
    }
}
