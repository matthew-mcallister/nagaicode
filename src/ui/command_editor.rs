use std::fmt;

use crossterm::Command;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

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
    /// If true, overwrite the last command in history.
    has_temp_cmd: bool,
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
            has_temp_cmd: false,
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
    type InEvent = Event;
    type OutEvent = Option<AppEvent>;

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

    fn set_focus(&mut self, focused: bool) {
        self.input.set_focus(focused);
        self.scroll_bar.set_focus(focused);
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

    fn handle_event(&mut self, event: Self::InEvent) -> Self::OutEvent {
        if let Event::Key(KeyEvent { code: KeyCode::Char('c'), modifiers, .. }) = event
            && modifiers.contains(KeyModifiers::CONTROL)
        {
            let mut text = self.input.inner().get_text();
            if text.ends_with('\n') {
                text.pop();
            }
            if self.has_temp_cmd {
                self.command_history.pop();
            }
            self.command_history.push(text);
            self.has_temp_cmd = true;
            self.command_history_pos = self.command_history.len();
            self.buffered_command.clear();
            self.input.inner_mut().set_text("");
            self.sync_scroll_bar();
            return None;
        }

        let response = self.input.handle_event(event);
        let response = match response {
            Some(AppEvent::Command(text)) => {
                if self.has_temp_cmd {
                    self.command_history.pop();
                    self.has_temp_cmd = false;
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::style::THEME_DARK;

    // Warning: Large blob of AI-generated tests
    #[test]
    fn test_ctrl_c() {
        let mut editor = CommandEditor::new(80, 8, &THEME_DARK);
        editor.input_mut().set_text("draft command");

        let event = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        let response = editor.handle_event(event);
        assert_eq!(response, None);
        assert_eq!(editor.input.inner().get_text(), "\n");

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Up)));
        assert_eq!(editor.input.inner().get_text(), "draft command\n");

        let mut editor = CommandEditor::new(80, 8, &THEME_DARK);
        editor.input_mut().set_text("temporary");
        editor.handle_event(Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert_eq!(editor.input.inner().get_text(), "\n");

        editor.input_mut().set_text("submitted");
        let response = editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Enter)));
        assert_eq!(response, Some(AppEvent::Command("submitted".to_string())));

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Up)));
        assert_eq!(editor.input.inner().get_text(), "submitted\n");

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Up)));
        assert_eq!(editor.input.inner().get_text(), "submitted\n");

        let mut editor = CommandEditor::new(80, 8, &THEME_DARK);
        editor.input_mut().set_text("temp1");
        editor.handle_event(Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));

        editor.input_mut().set_text("temp2");
        editor.handle_event(Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        assert_eq!(editor.input.inner().get_text(), "\n");

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Up)));
        assert_eq!(editor.input.inner().get_text(), "temp2\n");

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Up)));
        assert_eq!(editor.input.inner().get_text(), "temp2\n");

        let mut editor = CommandEditor::new(80, 8, &THEME_DARK);
        editor.input_mut().set_text("cmd1");
        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Enter)));

        editor.input_mut().set_text("cmd2");
        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Enter)));

        editor.input_mut().set_text("cmd3_draft");
        editor.handle_event(Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)));

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Up)));
        assert_eq!(editor.input.inner().get_text(), "cmd3_draft\n");

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Up)));
        assert_eq!(editor.input.inner().get_text(), "cmd2\n");

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Up)));
        assert_eq!(editor.input.inner().get_text(), "cmd1\n");

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Down)));
        assert_eq!(editor.input.inner().get_text(), "cmd1\n");

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Down)));
        assert_eq!(editor.input.inner().get_text(), "cmd2\n");

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Down)));
        assert_eq!(editor.input.inner().get_text(), "cmd3_draft\n");

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Down)));
        assert_eq!(editor.input.inner().get_text(), "\n");

        editor.input_mut().set_text("cmd3_final");
        let response = editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Enter)));
        assert_eq!(response, Some(AppEvent::Command("cmd3_final".to_string())));

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Up)));
        assert_eq!(editor.input.inner().get_text(), "cmd3_final\n");

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Up)));
        assert_eq!(editor.input.inner().get_text(), "cmd2\n");

        editor.handle_event(Event::Key(KeyEvent::from(KeyCode::Up)));
        assert_eq!(editor.input.inner().get_text(), "cmd1\n");
    }
}
