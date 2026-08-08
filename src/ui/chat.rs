use std::io::Write;

use compact_str::CompactString;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::queue;
use crossterm::execute;
use crossterm::style::{ResetColor, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{
    DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode, size,
};

use crate::error::AnyResult;
use crate::style::{THEME_DARK, Theme};
use crate::ui::padded::Padded;
use crate::ui::stacked_view::StackedView;
use crate::ui::Component;

const TEXT_INPUT_MAX_HEIGHT: u16 = 24;

struct Chat {
    theme: &'static Theme,
    stacked: Padded<StackedView>,
}

impl Chat {
    fn new(w: u16, h: u16, theme: &'static Theme) -> Self {
        Self {
            theme,
            stacked: Padded::new(
                StackedView::new(
                    w as usize - 4,
                    h as usize - 2,
                    TEXT_INPUT_MAX_HEIGHT.min(h.saturating_sub(2)) as usize,
                    theme,
                ),
                2,
                1,
                Some(theme.bg_base),
            ),
        }
    }

    fn resize(&mut self, w: u16, h: u16) {
        self.stacked.set_width(w as usize);
        self.stacked.set_height(h as usize);
    }

    /// Handles a key event. Returns true if the app should quit.
    fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let input = self.stacked.inner_mut().input_mut();
        match (key.code, ctrl, shift, alt) {
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
                input.set_text("");
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
        }
    }

    // TODO: cap redraw frequency
    fn draw(&self, stdout: &mut impl Write) -> AnyResult<()> {
        let text_style = self.theme.text_base;
        let bg = self.theme.bg_base;
        queue!(stdout, SetForegroundColor(text_style.foreground_color))?;
        queue!(stdout, SetBackgroundColor(bg))?;
        for (y, row) in self.stacked.drawable_rows().enumerate() {
            queue!(stdout, row, MoveTo(0, y as u16))?;
        }
        queue!(stdout, ResetColor)?;
        stdout.flush()?;
        Ok(())
    }
}

/// Runs the terminal app.
pub fn run() -> AnyResult<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, DisableLineWrap, Hide)?;

    let (w, h) = size()?;
    let mut chat = Chat::new(w, h, &THEME_DARK);
    chat.draw(&mut stdout)?;

    let mut quit = false;
    while !quit {
        match event::read()? {
            Event::Key(key) => {
                quit = chat.handle_key(key);
                chat.stacked.inner_mut().resize();
                chat.draw(&mut stdout)?;
            }
            Event::Resize(w, h) => {
                chat.resize(w, h);
                chat.draw(&mut stdout)?;
            }
            _ => {}
        }
    }

    execute!(stdout, EnableLineWrap, Show, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}
