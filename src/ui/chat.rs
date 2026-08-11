use std::io::Write;

use compact_str::CompactString;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::queue;
use crossterm::style::{ResetColor, SetBackgroundColor, SetForegroundColor};
use dedent::dedent;

use crate::error::AnyResult;
use crate::ui::history::HistoryItemContent;
use crate::ui::style::Theme;
use crate::ui::padded::Padded;
use crate::ui::stacked_view::StackedView;
use crate::ui::Component;

const TEXT_INPUT_MAX_HEIGHT: u16 = 24;

#[derive(Debug)]
pub struct Chat {
    theme: &'static Theme,
    stacked: Padded<StackedView>,
}

impl Chat {
    pub fn new(w: u16, h: u16, theme: &'static Theme) -> Self {
        // Minimum dimensions are 80x24. If the terminal is smaller the UI will
        // just overflow the screen. This helps avoid crashes or bizarre bugs
        // caused by pathologically tiny terminals.
        let w = w.max(20);
        let h = h.max(16);

        let mut stacked = StackedView::new(
            w as usize - 4,
            h as usize - 2,
            TEXT_INPUT_MAX_HEIGHT.min(h.saturating_sub(2)) as usize,
            theme,
        );
        stacked.history_mut().add_item(HistoryItemContent::Help(dedent!("
            Welcome to NagaiCode!

            Type /help for a list of commands."
        ).into()));

        Self {
            theme,
            stacked: Padded::new(stacked, 2, 1, Some(theme.bg_base)),
        }
    }

    pub fn resize(&mut self, w: u16, h: u16) {
        self.stacked.set_width(w as usize);
        self.stacked.set_height(h as usize);
    }

    /// Handles a key event. Returns true if the app should quit.
    // TODO: pass input events down to components, bubble up generated events
    // to parents.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let input = self.stacked.inner_mut().input_mut();
        let res = match (key.code, ctrl, shift, alt) {
            // Ctrl + char
            (KeyCode::Char('c'), true, _, _) => true,
            (KeyCode::Char('a'), true, _, _) => {
                input.go_to_line_start();
                false
            }
            (KeyCode::Char('e'), true, _, _) => {
                input.go_to_line_end();
                false
            }
            (KeyCode::Char('u'), true, _, _) => {
                input.delete_to_line_start();
                false
            }
            (KeyCode::Char('k'), true, _, _) => {
                input.delete_to_line_end();
                false
            }
            (KeyCode::Char('w'), true, _, _) => {
                input.delete_prev_word();
                false
            }
            (KeyCode::Char('y'), true, _, _) => {
                input.paste_buffer();
                false
            }
            // Alt + char
            (KeyCode::Char('f'), _, _, true) => {
                input.go_to_word_end();
                false
            }
            (KeyCode::Char('b'), _, _, true) => {
                input.go_to_prev_word_start();
                false
            }
            // Other combinations
            | (KeyCode::Char('j'), true, _, _)
            | (KeyCode::Char('j'), _, _, true)
            | (KeyCode::Enter, true, _, _)
            | (KeyCode::Enter, _, true, _)
            | (KeyCode::Enter, _, _, true) => {
                input.paste("\n");
                false
            }
            // Ignoring modifiers
            (KeyCode::Char(c), _, _, _) => {
                let mut s = CompactString::with_capacity(1);
                s.push(c);
                input.paste(&s);
                false
            }
            (KeyCode::Enter, _, _, _) => {
                let text = input.get_text();
                input.set_text("");
                let text = text.strip_suffix('\n').unwrap_or(&text);
                self.process_command(&text);
                false
            }
            (KeyCode::Tab, _, _, _) => {
                // XXX: Maybe should expand to spaces when input via keyboard
                input.paste("\t");
                false
            }
            (KeyCode::Backspace, _, _, _) => {
                input.backspace();
                false
            }
            (KeyCode::Delete, _, _, _) => {
                input.delete();
                false
            }
            (KeyCode::Left, _, _, _) => {
                input.move_left();
                false
            }
            (KeyCode::Right, _, _, _) => {
                input.move_right();
                false
            }
            (KeyCode::Up, _, _, _) => {
                input.move_up(1);
                false
            }
            (KeyCode::Down, _, _, _) => {
                input.move_down(1);
                false
            }
            (KeyCode::PageUp, _, _, _) => {
                input.move_up(input.max_height());
                false
            }
            (KeyCode::PageDown, _, _, _) => {
                input.move_down(input.max_height());
                false
            }
            _ => false,
        };
        self.stacked.inner_mut().resize();
        res
    }

    fn process_command(&mut self, command: &str) {
        if command.trim().is_empty() {
            return;
        }

        let history = self.stacked.inner_mut().history_mut();
        if !command.contains('\n') {
            let slash_command = command.starts_with('/');
            let bang_command = command.starts_with('!');
            if slash_command || bang_command {
                let command = &command[1..];
                if slash_command {
                    match crate::command::run_command(&command) {
                        Ok(output) => {
                            if !output.trim().is_empty() {
                                history.add_item(HistoryItemContent::Help(output));
                            }
                        }
                        Err(e) => {
                            history.add_item(HistoryItemContent::Error(e.to_string()));
                        }
                    }
                } else {
                    todo!("call system()")
                };
                return;
            }
        }
        history.add_item(HistoryItemContent::Markdown(command.into()));
    }

    // TODO: cap redraw frequency
    pub fn draw(&self, stdout: &mut impl Write) -> AnyResult<()> {
        let text_style = self.theme.text_base;
        let bg = self.theme.bg_base;
        queue!(stdout,
            Hide,
            SetForegroundColor(text_style.fg_color),
            SetBackgroundColor(bg),
        )?;
        for (y, row) in self.stacked.drawable_rows().enumerate() {
            queue!(stdout, MoveTo(0, y as u16), row)?;
        }
        let (row, col) = self.stacked.cursor_pos();
        queue!(stdout, ResetColor, MoveTo(col as u16, row as u16), Show)?;
        stdout.flush()?;
        Ok(())
    }
}
