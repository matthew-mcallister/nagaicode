use serde_json::json;

use crate::query::{DataQuery, QueryError, QueryField};

/// Command history scroll direction, used to set cursor position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Forward,
    Backward,
}

#[derive(Debug, Default)]
pub struct CommandHistory {
    /// Submitted commands, most recent last. Only records newly sent commands
    /// when different from the previously sent command.
    command_history: Vec<String>,
    /// Command history cursor. The current/buffered command is represented as
    /// `command_history.len()`.
    command_history_pos: usize,
    /// Current unsent command from the input editor.
    buffered_command: String,
}

impl CommandHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a submitted command, unless it is blank or a repeat of the
    /// previous command.
    pub fn push(&mut self, command: impl AsRef<str> + Into<String>) {
        if command.as_ref().trim_end_matches('\n').is_empty() {
            return;
        }
        if self.command_history.last().map(String::as_str) != Some(command.as_ref()) {
            self.command_history.push(command.into());
        }
        self.command_history_pos = self.command_history.len();
        self.buffered_command.clear();
    }

    /// Moves the cursor to the previous command and returns its text.
    ///
    /// The `get_current` callback may be used to save the state of the current
    /// command being edited.
    // TODO: Decide how to handle when previously submitted commands are
    // edited. Currently changes are discarded. Should they be saved
    // temporarily?
    pub fn prev_command(&mut self, get_current: impl FnOnce() -> String) -> Option<&str> {
        if self.command_history_pos == 0 {
            return None;
        }
        if self.command_history_pos == self.command_history.len() {
            self.buffered_command = get_current();
        }
        self.command_history_pos -= 1;
        Some(&self.command_history[self.command_history_pos])
    }

    /// Moves the cursor to the next command and returns the text to display.
    ///
    /// The `get_current` callback may be used to save the state of the current
    /// command being edited.
    pub fn next_command(&mut self, get_current: impl FnOnce() -> String) -> Option<&str> {
        let _ = get_current;
        if self.command_history_pos == self.command_history.len() {
            return None;
        }
        self.command_history_pos += 1;
        if self.command_history_pos == self.command_history.len() {
            Some(&self.buffered_command)
        } else {
            Some(&self.command_history[self.command_history_pos])
        }
    }

    pub fn commands(&self) -> &[String] {
        &self.command_history
    }

    pub fn pos(&self) -> usize {
        self.command_history_pos
    }

    pub fn buffered(&self) -> &str {
        &self.buffered_command
    }
}

impl DataQuery for CommandHistory {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "command_history": self.query("/command_history")?,
                "command_history_pos": self.query("/command_history_pos")?,
                "buffered_command": self.query("/buffered_command")?,
            }))),
            "command_history" => Ok(QueryField::Value(json!(self.command_history))),
            "command_history_pos" => Ok(QueryField::Value(json!(self.command_history_pos))),
            "buffered_command" => Ok(QueryField::Value(json!(self.buffered_command))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_skips_blank_and_repeated_commands() {
        let mut history = CommandHistory::new();
        assert!(history.commands().is_empty());
        assert_eq!(history.pos(), 0);

        history.push("foo");
        history.push("foo");
        history.push("\n");
        history.push("\n\n");
        assert_eq!(history.commands(), ["foo"]);

        history.push("bar");
        assert_eq!(history.commands(), ["foo", "bar"]);
        assert_eq!(history.pos(), 2);
        assert_eq!(history.buffered(), "");
    }

    #[test]
    fn test_prev_and_next_restore_buffered_command() {
        let mut history = CommandHistory::new();

        let get_current = || "unsent".to_owned();
        assert_eq!(history.prev_command(get_current), None);
        assert_eq!(history.next_command(get_current), None);

        history.push("foo");
        history.push("bar");

        assert_eq!(history.prev_command(get_current), Some("bar"));
        assert_eq!(history.buffered(), "unsent");
        let get_current = || "ignored".to_owned();
        assert_eq!(history.prev_command(get_current), Some("foo"));
        assert_eq!(history.prev_command(get_current), None);
        assert_eq!(history.pos(), 0);

        assert_eq!(history.next_command(get_current), Some("bar"));
        assert_eq!(history.next_command(get_current), Some("unsent"));
        assert_eq!(history.pos(), 2);
        assert_eq!(history.next_command(get_current), None);
    }

    #[test]
    fn test_push_while_recalling_resets_cursor() {
        let mut history = CommandHistory::new();
        let get_current = || "unsent".to_owned();
        history.push("foo");
        history.push("bar");
        assert_eq!(history.prev_command(get_current), Some("bar"));

        history.push("baz");
        assert_eq!(history.commands(), ["foo", "bar", "baz"]);
        assert_eq!(history.pos(), 3);
        assert_eq!(history.buffered(), "");
        assert_eq!(history.next_command(get_current), None);
        assert_eq!(history.prev_command(get_current), Some("baz"));
    }
}
