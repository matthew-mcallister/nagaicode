use crossterm::event::Event;

use crate::app::AppEvent;
use crate::ui::input_box::{InputBox, InputBoxRow};
use crate::ui::padded::{Padded, PaddedRow};
use crate::ui::style::Theme;
use crate::ui::Component;

/// A single drawable row of the command editor.
pub type CommandEditorRow<'a> = PaddedRow<InputBoxRow<'a>>;

/// Command input editor. A thin wrapper around `Padded<InputBox>` that also
/// owns the command history navigation logic. The underlying `InputBox` emits
/// `AppEvent::HistoryPrev`/`HistoryNext` when the cursor is at the boundaries
/// of the input; this component intercepts those events and swaps in the
/// appropriate historical command, so they never propagate to the parent.
#[derive(Debug)]
pub struct CommandEditor {
    input: Padded<InputBox>,
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
        Self {
            input: Padded::new(
                InputBox::new(width.saturating_sub(4), max_height.saturating_sub(2)),
                2,
                1,
                Some(theme.bg_input_box),
            ),
            command_history: Vec::new(),
            command_history_pos: 0,
            buffered_command: String::new(),
        }
    }

    pub fn input_mut(&mut self) -> &mut InputBox {
        self.input.inner_mut()
    }
}

impl Component for CommandEditor {
    type Row<'a> = PaddedRow<InputBoxRow<'a>> where Self: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where Self: 'a;
    type EventReponse = Option<AppEvent>;

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        self.input.drawable_rows()
    }

    fn set_width(&mut self, width: usize) {
        self.input.set_width(width);
    }

    fn set_height(&mut self, height: usize) {
        self.input.set_height(height);
    }

    fn width(&self) -> usize {
        self.input.width()
    }

    fn height(&self) -> usize {
        self.input.height()
    }

    fn cursor_pos(&self) -> (usize, usize) {
        self.input.cursor_pos()
    }

    fn handle_event(&mut self, event: Event) -> Self::EventReponse {
        let response = self.input.handle_event(event);
        match &response {
            Some(AppEvent::Command(text)) => {
                if self.command_history.last() != Some(text) {
                    self.command_history.push(text.clone());
                }
                self.command_history_pos = self.command_history.len();
                self.buffered_command.clear();
                response
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
        }
    }
}
