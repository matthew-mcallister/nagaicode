use crossterm::Command;

use crate::ui::style::Theme;
use crate::ui::history::{History, HistoryRowRef};
use crate::ui::input_box::{InputBox, InputBoxRow};
use crate::ui::padded::{Padded, PaddedRow};
use crate::ui::{write_spaces, Component};

/// Stacks components vertically. The input box is anchored to the bottom and
/// grows upward; the history fills the remaining space.
#[derive(Debug)]
pub struct StackedView {
    width: usize,
    height: usize,
    history: History,
    input: Padded<InputBox>,
}

impl StackedView {
    pub fn new(
        width: usize,
        height: usize,
        input_max_height: usize,
        theme: &'static Theme,
    ) -> Self {
        let mut this = Self {
            width,
            height,
            history: History::new(width, 0),
            input: Padded::new(
                InputBox::new(width.saturating_sub(4), input_max_height.saturating_sub(2)),
                2,
                1,
                Some(theme.bg_input_box),
            ),
        };
        this.resize();  // Compute history height
        this
    }

    pub fn input_mut(&mut self) -> &mut InputBox {
        self.input.inner_mut()
    }

    pub fn history_mut(&mut self) -> &mut History {
        &mut self.history
    }

    /// Recomputes the history region's height after the input box changes size.
    pub fn resize(&mut self) {
        let history_height = self.height.saturating_sub(self.input.height());
        if self.history.max_height() != history_height {
            self.history.set_height(history_height);
        }
    }
}

#[derive(Debug)]
pub enum StackedRow<'a> {
    Empty { width: usize },
    History(HistoryRowRef<'a>),
    Input(PaddedRow<InputBoxRow<'a>>),
}

impl Command for StackedRow<'_> {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        match self {
            StackedRow::Empty { width } => write_spaces(f, *width),
            StackedRow::History(row) => row.write_ansi(f),
            StackedRow::Input(row) => row.write_ansi(f),
        }
    }
}

impl Component for StackedView {
    type Row<'a> = StackedRow<'a> where Self: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where Self: 'a;

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        let empty_rows = self
            .height
            .saturating_sub(self.history.height())
            .saturating_sub(self.input.height());
        let width = self.width;
        let empty = (0..empty_rows).map(move |_| StackedRow::Empty { width });
        let history = self.history.drawable_rows().map(StackedRow::History);
        let input = self.input.drawable_rows().map(StackedRow::Input);
        Box::new(empty.chain(history).chain(input))
    }

    fn set_width(&mut self, width: usize) {
        self.width = width;
        self.history.set_width(width);
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

    fn cursor_pos(&self) -> (usize, usize) {
        let (row, col) = self.input.cursor_pos();
        (self.height.saturating_sub(self.input.height()) + row, col)
    }
}
