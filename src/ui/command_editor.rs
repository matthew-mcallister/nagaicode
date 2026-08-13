use std::fmt;

use crossterm::Command;
use crossterm::event::Event;

use crate::app::AppEvent;
use crate::ui::input_box::{InputBox, InputBoxRow};
use crate::ui::padded::{Padded, PaddedRow};
use crate::ui::scroll_bar::{ScrollBar, ScrollBarRow};
use crate::ui::style::Theme;
use crate::ui::Component;

/// A single drawable row of the command editor. The scroll bar is rendered to
/// the right of the padded input box.
#[derive(Debug)]
pub struct CommandEditorRow<'a> {
    input: PaddedRow<InputBoxRow<'a>>,
    bar: ScrollBarRow<'a>,
}

impl Command for CommandEditorRow<'_> {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        self.input.write_ansi(f)?;
        self.bar.write_ansi(f)
    }
}

/// Command input editor, wrapper around InputBox
#[derive(Debug)]
pub struct CommandEditor {
    input: Padded<InputBox>,
    scroll_bar: ScrollBar,
    /// Submitted commands, most recent last. Only records newly sent commands
    /// when different from the previously sent command.
    command_history: Vec<String>,
    /// Command history cursor. The current/buffered command is represented as
    /// `command_history.len()`.
    command_history_pos: usize,
    /// Current unsent command from the input editor.
    buffered_command: String,
}

impl CommandEditor {
    pub fn new(width: usize, max_height: usize, theme: &'static Theme) -> Self {
        let mut this = Self {
            input: Padded::new(
                // Reserve one column for the scroll bar
                InputBox::new(width.saturating_sub(5), max_height.saturating_sub(2)),
                2,
                1,
                Some(theme.bg_input_box),
            ),
            scroll_bar: ScrollBar::new(theme),
            command_history: Vec::new(),
            command_history_pos: 0,
            buffered_command: String::new(),
        };
        this.scroll_bar.set_width(1);
        this.sync_scroll_bar();
        this
    }

    pub fn input_mut(&mut self) -> &mut InputBox {
        self.input.inner_mut()
    }

    /// Syncs the scroll bar with the current state of the input box.
    // XXX: This is kind of a kludge to handle the way StackedView allows the
    // input box to set its own height
    fn sync_scroll_bar(&mut self) {
        let input = self.input.inner();
        self.scroll_bar.set_num_rows(input.num_rows());
        self.scroll_bar.set_viewport(input.viewport_top_pos(), input.viewport_bottom_pos());
        self.scroll_bar.set_height(self.input.height());
    }
}

impl Component for CommandEditor {
    type Row<'a> = CommandEditorRow<'a> where Self: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where Self: 'a;
    type EventReponse = Option<AppEvent>;

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        Box::new(
            self.input
                .drawable_rows()
                .zip(self.scroll_bar.drawable_rows())
                .map(|(input, bar)| CommandEditorRow { input, bar }),
        )
    }

    fn set_width(&mut self, width: usize) {
        self.input.set_width(width.saturating_sub(1));
        self.scroll_bar.set_width(1);
        self.sync_scroll_bar();
    }

    fn set_height(&mut self, height: usize) {
        self.input.set_height(height);
        self.sync_scroll_bar();
    }

    fn width(&self) -> usize {
        self.input.width() + 1
    }

    fn height(&self) -> usize {
        self.input.height()
    }

    fn cursor(&self) -> Option<(usize, usize)> {
        self.input.cursor()
    }

    fn handle_event(&mut self, event: Event) -> Self::EventReponse {
        let response = self.input.handle_event(event);
        let response = match response {
            Some(AppEvent::Command(text)) => {
                if self.command_history.last() != Some(&text) {
                    self.command_history.push(text.clone());
                }
                self.command_history_pos = self.command_history.len();
                self.buffered_command.clear();
                Some(AppEvent::Command(text))
            }
            Some(AppEvent::HistoryPrev) => {
                if self.command_history_pos > 0 {
                    let len = self.command_history.len();
                    if self.command_history_pos == len {
                        self.buffered_command = self.input.inner().get_text();
                    }
                    self.command_history_pos -= 1;
                    let text = self.command_history[self.command_history_pos].clone();
                    self.input.inner_mut().set_text(&text);
                }
                None
            }
            Some(AppEvent::HistoryNext) => {
                let len = self.command_history.len();
                if self.command_history_pos < len {
                    self.command_history_pos += 1;
                    let input = self.input.inner_mut();
                    if self.command_history_pos == len {
                        // Restore unsent command from buffer
                        input.set_text(&self.buffered_command);
                        input.go_to_end();
                    } else {
                        input.set_text(&self.command_history[self.command_history_pos]);
                        input.go_to_end();
                    }
                }
                None
            }
            None => None,
        };
        self.sync_scroll_bar();
        response
    }
}
