use crossterm::event::Event;

use crate::app::AppEvent;
use crate::ui::canvas::Canvas;
use crate::ui::input_box::InputBox;
use crate::ui::padded::Padded;
use crate::ui::scroll_bar::ScrollBar;
use crate::ui::style::Theme;
use crate::ui::Component;

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
    type Update<'a> = ();
    type Event = Option<AppEvent>;

    fn draw(&self, canvas: Canvas) {
        self.input.draw(&mut *canvas);
        self.scroll_bar.draw(canvas);
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

    fn handle_input(&mut self, event: Event) -> Self::Event {
        let response = self.input.handle_input(event);
        let response = match response {
            Some(AppEvent::Command(text)) => {
                if text.trim_end_matches('\n').is_empty() {
                    return None;
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
            Some(AppEvent::Interrupt) => Some(AppEvent::Interrupt),
            _ => None,
        };
        self.sync_scroll_bar();
        response
    }

    fn handle_update<'a>(&mut self, _update: Self::Update<'a>) {
        self.scroll_bar.handle_update(());
    }
}

#[cfg(test)]
mod tests {
    use crossterm::Command;
    use crossterm::style::{ContentStyle, SetStyle};

    use super::*;
    use crate::ui::style::{Style, THEME_DARK, UpdateStyle};
    use crate::ui::styled_string::StyledString;

    fn render(editor: &CommandEditor) -> String {
        let mut rows: Vec<StyledString> = (0..editor.height())
            .map(|_| StyledString::new(THEME_DARK.base_style(), editor.width()))
            .collect();
        editor.draw(&mut rows);
        rows.iter()
            .map(|row| {
                let mut out = String::new();
                row.write_ansi(&mut out).unwrap();
                out
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn input_prefix() -> String {
        let mut out = String::new();
        let mut content: ContentStyle = THEME_DARK.text_base.into();
        content.background_color = Some(THEME_DARK.bg_input_box);
        SetStyle(content).write_ansi(&mut out).unwrap();
        out
    }

    fn bar_suffix(focused: bool) -> String {
        let old_style = Style::new(THEME_DARK.text_base, THEME_DARK.bg_input_box);
        let text_style = if focused {
            THEME_DARK.text_scroll_bar_focused
        } else {
            THEME_DARK.text_scroll_bar_unfocused
        };
        let new_style = Style::new(text_style, THEME_DARK.bg_base);
        let mut out = String::new();
        UpdateStyle(old_style, new_style).write_ansi(&mut out).unwrap();
        out.push('▐');
        out
    }

    fn track_suffix() -> String {
        let old_style = Style::new(THEME_DARK.text_base, THEME_DARK.bg_input_box);
        let new_style = Style::new(THEME_DARK.text_scroll_bar_track, THEME_DARK.bg_base);
        let mut out = String::new();
        UpdateStyle(old_style, new_style).write_ansi(&mut out).unwrap();
        out.push('▐');
        out
    }

    #[test]
    fn test_render() {
        let mut editor = CommandEditor::new(10, 5, &THEME_DARK);
        let pfx = input_prefix();
        let bar = bar_suffix(false);
        let focused_bar = bar_suffix(true);
        let track = track_suffix();

        assert_eq!(editor.width(), 10);
        assert_eq!(editor.height(), 3);
        assert_eq!(editor.cursor(), Some((1, 2)));
        assert_eq!(
            render(&editor),
            format!("{pfx}         {bar}\n{pfx}         {bar}\n{pfx}         {bar}"),
        );

        editor.set_focus(true);
        assert_eq!(
            render(&editor),
            format!("{pfx}         {focused_bar}\n{pfx}         {focused_bar}\n{pfx}         {focused_bar}"),
        );

        editor.set_focus(false);
        editor.input_mut().set_text("foo");
        assert_eq!(editor.cursor(), Some((1, 2)));
        editor.input_mut().go_to_end();
        assert_eq!(editor.cursor(), Some((1, 5)));
        assert_eq!(
            render(&editor),
            format!("{pfx}         {bar}\n{pfx}  foo    {bar}\n{pfx}         {bar}"),
        );

        let mut editor = CommandEditor::new(10, 6, &THEME_DARK);
        editor.input_mut().set_text("1\n2\n3\n4\n5\n6\n7\n8");
        editor.sync_scroll_bar();
        assert_eq!(editor.width(), 10);
        assert_eq!(editor.height(), 6);
        assert_eq!(
            render(&editor),
            format!(
                "{pfx}         {bar}\n{pfx}  1      {bar}\n{pfx}  2      {bar}\n{pfx}  3      {track}\n{pfx}  4      {track}\n{pfx}         {track}",
            ),
        );

        editor.set_width(12);
        assert_eq!(editor.width(), 12);
    }
}
