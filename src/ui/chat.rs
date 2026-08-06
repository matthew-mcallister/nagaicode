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
use crate::ui::input_box::{DrawInputBox, InputBox};

const TEXT_INPUT_MAX_HEIGHT: u16 = 24;

struct Chat {
    width: u16,
    height: u16,
    theme: &'static Theme,
    input: InputBox,
}

impl Chat {
    fn new(w: u16, h: u16, theme: &'static Theme) -> Self {
        Self {
            width: w,
            height: h,
            theme,
            input: InputBox::new(w as usize - 8, TEXT_INPUT_MAX_HEIGHT as _),
        }
    }

    fn resize(&mut self, w: u16, _h: u16) {
        self.width = w;
        self.height = w;
        self.input.set_width(w as usize - 8);
    }

    /// Handles a key event. Returns true if the app should quit.
    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'c' => true,
            KeyCode::Char(c) => {
                let mut s = CompactString::with_capacity(1);
                s.push(c);
                self.input.paste(&s);
                false
            }
            KeyCode::Enter => {
                self.input.paste("\n");
                false
            }
            KeyCode::Tab => {
                self.input.paste("\t");
                false
            }
            KeyCode::Backspace => {
                self.input.backspace();
                false
            }
            KeyCode::Delete => {
                self.input.delete();
                false
            }
            KeyCode::Left => {
                self.input.move_left();
                false
            }
            KeyCode::Right => {
                self.input.move_right();
                false
            }
            KeyCode::Up => {
                self.input.move_up(1);
                false
            }
            KeyCode::Down => {
                self.input.move_down(1);
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
            background_color: Some(
                self.theme.bg_base,
            ),
            ..Default::default()
        });

        let x_0 = 2;
        let x_1 = self.width - 2;
        let y_1 = self.height - 1;
        let y_0 = y_1 - self.input.height() as u16 - 2;
        let style = ContentStyle {
            background_color: Some(self.theme.bg_input_box),
            ..Default::default()
        };
        canvas.clear_rect(x_0, y_0, x_1 - x_0, y_1 - y_0, style);
        let draw_input = DrawInputBox {
            input: &self.input,
            x: x_0 + 2,
            y: y_0 + 1,
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
