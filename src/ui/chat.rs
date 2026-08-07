use std::io::Write;

use compact_str::CompactString;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::style::ContentStyle;
use crossterm::terminal::{
    DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode, size,
};

use crate::canvas::Canvas;
use crate::error::AnyResult;
use crate::style::{THEME_DARK, Theme};
use crate::ui::history::{DrawHistory, History};
use crate::ui::input_box::{DrawInputBox, InputBox};

const TEXT_INPUT_MAX_HEIGHT: u16 = 24;
/// Horizontal padding around the history and input box, in columns.
const H_PADDING: u16 = 2;
/// Vertical padding above/below the history, in rows.
const V_PADDING: u16 = 1;

struct Chat {
    width: u16,
    height: u16,
    theme: &'static Theme,
    input: InputBox,
    history: History,
}

impl Chat {
    fn new(w: u16, h: u16, theme: &'static Theme) -> Self {
        let mut chat = Self {
            width: w,
            height: h,
            theme,
            input: InputBox::new(w.saturating_sub(2 * H_PADDING + 4) as usize, TEXT_INPUT_MAX_HEIGHT as _),
            history: History::new(w.saturating_sub(2 * H_PADDING) as usize, 1),
        };
        chat.update_history_max_height();
        chat
    }

    fn resize(&mut self, w: u16, h: u16) {
        self.width = w;
        self.height = h;
        self.input
            .set_width(w.saturating_sub(2 * H_PADDING + 4) as usize);
        self.history.set_width(w.saturating_sub(2 * H_PADDING) as usize);
        self.update_history_max_height();
    }

    /// Computes the input box rectangle `(x, y, w, h)`.
    fn input_box_rect(&self) -> (u16, u16, u16, u16) {
        let x_0 = H_PADDING;
        let x_1 = self.width - H_PADDING;
        let y_1 = self.height - V_PADDING;
        let y_0 = y_1 - self.input.height() as u16 - 2;
        (x_0, y_0, x_1 - x_0, y_1 - y_0)
    }

    /// Computes the history rectangle `(x, y, w, h)`, filling the screen from
    /// the top down to the top of the input box.
    fn history_rect(&self) -> (u16, u16, u16, u16) {
        let (x_0, y_0, _, _) = self.input_box_rect();
        (
            x_0,
            V_PADDING,
            self.width.saturating_sub(2 * H_PADDING),
            y_0.saturating_sub(2 * V_PADDING),
        )
    }

    /// Keeps the history viewport height in sync with the input box height.
    fn update_history_max_height(&mut self) {
        let (_, _, _, h) = self.history_rect();
        let height = h as usize;
        if height != self.history.max_height() {
            self.history.set_max_height(height);
        }
    }

    /// Handles a key event. Returns true if the app should quit.
    fn handle_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match (key.code, ctrl, shift, alt) {
            // Ctrl + char
            (KeyCode::Char('c'), true, _, _) => true,
            (KeyCode::Char('a'), true, _, _) => {
                self.input.go_to_line_start();
                false
            }
            (KeyCode::Char('e'), true, _, _) => {
                self.input.go_to_line_end();
                false
            }
            (KeyCode::Char('u'), true, _, _) => {
                self.input.delete_to_line_start();
                false
            }
            (KeyCode::Char('k'), true, _, _) => {
                self.input.delete_to_line_end();
                false
            }
            (KeyCode::Char('w'), true, _, _) => {
                self.input.delete_prev_word();
                false
            }
            (KeyCode::Char('y'), true, _, _) => {
                self.input.paste_buffer();
                false
            }
            // Alt + char
            (KeyCode::Char('f'), _, _, true) => {
                self.input.go_to_word_end();
                false
            }
            (KeyCode::Char('b'), _, _, true) => {
                self.input.go_to_prev_word_start();
                false
            }
            // Other combinations
            | (KeyCode::Char('j'), true, _, _)
            | (KeyCode::Char('j'), _, _, true)
            | (KeyCode::Enter, true, _, _)
            | (KeyCode::Enter, _, true, _)
            | (KeyCode::Enter, _, _, true) => {
                self.input.paste("\n");
                false
            }
            // Ignoring modifiers
            (KeyCode::Char(c), _, _, _) => {
                let mut s = CompactString::with_capacity(1);
                s.push(c);
                self.input.paste(&s);
                false
            }
            (KeyCode::Enter, _, _, _) => {
                let text = self.input.get_text();
                self.input.set_text("");
                let text = text.trim_end_matches('\n');
                if !text.is_empty() {
                    self.history.system_message(text);
                }
                false
            }
            (KeyCode::Tab, _, _, _) => {
                // XXX: Maybe should expand to spaces when input via keyboard
                self.input.paste("\t");
                false
            }
            (KeyCode::Backspace, _, _, _) => {
                self.input.backspace();
                false
            }
            (KeyCode::Delete, _, _, _) => {
                self.input.delete();
                false
            }
            (KeyCode::Left, _, _, _) => {
                self.input.move_left();
                false
            }
            (KeyCode::Right, _, _, _) => {
                self.input.move_right();
                false
            }
            (KeyCode::Up, _, _, _) => {
                self.input.move_up(1);
                false
            }
            (KeyCode::Down, _, _, _) => {
                self.input.move_down(1);
                false
            }
            (KeyCode::PageUp, _, _, _) => {
                self.input.move_up(self.input.max_height());
                false
            }
            (KeyCode::PageDown, _, _, _) => {
                self.input.move_down(self.input.max_height());
                false
            }
            _ => false,
        }
    }

    // TODO: cap redraw frequency
    fn draw(&self, stdout: &mut impl Write) -> AnyResult<()> {
        let mut canvas = Canvas::new(self.width, self.height);

        // Clear screen
        canvas.clear_all(ContentStyle {
            background_color: Some(self.theme.bg_base),
            ..Default::default()
        });

        // Draw chat history
        let (history_x, history_y, history_w, history_h) = self.history_rect();
        let base = ContentStyle {
            background_color: Some(self.theme.bg_base),
            ..Default::default()
        };
        canvas.clear_rect(history_x, history_y, history_w, history_h, base);
        let draw_history = DrawHistory {
            theme: self.theme,
            history: &self.history,
            x: history_x,
            y: history_y,
        };
        draw_history.draw_to(&mut canvas);

        // Draw input box
        let (input_x, input_y, input_w, input_h) = self.input_box_rect();
        let style = ContentStyle {
            background_color: Some(self.theme.bg_input_box),
            ..Default::default()
        };
        canvas.clear_rect(input_x, input_y, input_w, input_h, style);
        let draw_input = DrawInputBox {
            input: &self.input,
            x: input_x + 2,
            y: input_y + 1,
            style: self.theme.text_base,
        };
        draw_input.draw_to(&mut canvas);

        execute!(stdout, canvas)?;
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
                chat.update_history_max_height();
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
