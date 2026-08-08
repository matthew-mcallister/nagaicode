use crossterm::Command;

use crate::ui::empty::{Empty, EmptyRow};
use crate::ui::input_box::{InputBox, InputBoxRow};
use crate::ui::Component;

/// Stacks components vertically. The input box is anchored to the bottom and
/// grows upward; the empty component fills the remaining space.
#[derive(Debug)]
pub struct StackedView {
    width: usize,
    height: usize,
    empty: Empty,
    input: InputBox,
}

impl StackedView {
    pub fn new(
        width: usize,
        height: usize,
        input_max_height: usize,
    ) -> Self {
        let mut this = Self {
            width,
            height,
            empty: Empty::new(width, 0, None),
            input: InputBox::new(width, input_max_height),
        };
        this.resize();  // Compute empty height
        this
    }

    pub fn input_mut(&mut self) -> &mut InputBox {
        &mut self.input
    }

    /// Recomputes the empty region's height after the input box changes size.
    pub fn resize(&mut self) {
        let empty_height = self.height.saturating_sub(self.input.height());
        if self.empty.height() != empty_height {
            self.empty.set_height(empty_height);
        }
    }
}

#[derive(Debug)]
pub enum StackedRow<'a> {
    Empty(EmptyRow),
    Input(InputBoxRow<'a>),
}

impl Command for StackedRow<'_> {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        match self {
            StackedRow::Empty(row) => row.write_ansi(f),
            StackedRow::Input(row) => row.write_ansi(f),
        }
    }
}

impl Component for StackedView {
    type Row<'a> = StackedRow<'a> where Self: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where Self: 'a;

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        let empty_height = self.empty.height();
        let empty = self.empty.drawable_rows().take(empty_height);
        let input = self.input.drawable_rows();
        Box::new(
            empty.map(StackedRow::Empty)
                .chain(input.map(StackedRow::Input)),
        )
    }

    fn set_width(&mut self, width: usize) {
        self.width = width;
        self.empty.set_width(width);
        self.input.set_width(width);
        self.resize();
    }

    fn set_height(&mut self, height: usize) {
        self.height = height;
        self.resize();
    }

    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }
}
