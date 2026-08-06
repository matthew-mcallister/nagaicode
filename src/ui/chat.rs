use std::io::{self, Write};

use compact_str::CompactString;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::{execute, queue};
use crossterm::style::ContentStyle;
use crossterm::terminal::{
    size, Clear, ClearType, DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
    LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use crate::error::AnyResult;
use crate::render::DrawRectangle;
use crate::style::{BackgroundColorName, THEME_DARK};
use crate::ui::input_box::{Anchor, DrawInputBox, InputBox};

/// Rows/columns of blank space around the outer grey box.
const MARGIN: u16 = 2;
/// Rows/columns of interior padding inside the grey box.
const PADDING: u16 = 1;
const MAX_HEIGHT: u16 = 24;

/// Width of the input content: terminal minus margins and padding on each side.
fn content_width(w: u16) -> usize {
    (w.saturating_sub(2 * (MARGIN + PADDING)) as usize).max(4)
}

/// Maximum number of visible input rows: content fits inside the box, which
/// fits inside the terminal minus top/bottom margins.
fn content_height(h: u16) -> usize {
    (h.saturating_sub(2 * (MARGIN + PADDING)) as usize).max(1)
}

struct Chat {
    input: InputBox,
}

impl Chat {
    fn new(w: u16, h: u16) -> Self {
        Self {
            input: InputBox::new(content_width(w), h as usize),
        }
    }

    fn resize(&mut self, w: u16, h: u16) {
        self.input.set_width(content_width(w));
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
                self.input.move_up();
                false
            }
            KeyCode::Down => {
                self.input.move_down();
                false
            }
            KeyCode::PageUp => {
                self.input.scroll_up();
                false
            }
            KeyCode::PageDown => {
                self.input.scroll_down();
                false
            }
            _ => false,
        }
    }

    fn draw(&self, stdout: &mut impl Write) -> AnyResult<()> {
        let (w, h) = size()?;

        // Clear everything, then repaint the whole screen with the base
        // background color before drawing the input box region.
        queue!(stdout, Clear(ClearType::All))?;

        let base = DrawRectangle {
            x: 0,
            y: 0,
            width: w,
            height: h,
            style: ContentStyle {
                background_color: Some(
                    THEME_DARK.get_background_color(BackgroundColorName::Base),
                ),
                ..Default::default()
            },
        };
        queue!(stdout, base)?;

        // The grey box is inset by MARGIN rows/columns from the terminal edges.
        // Its content sits PADDING rows/columns inside the box, anchored to the
        // bottom.
        let content_height = self.input.height() as u16;
        let content_x: u16 = MARGIN + PADDING;
        let content_y = h.saturating_sub(MARGIN + PADDING + 1);
        let content_top = content_y.saturating_sub(content_height - 1);

        let rect = DrawRectangle {
            x: MARGIN,
            y: content_top.saturating_sub(PADDING),
            width: w.saturating_sub(2 * MARGIN),
            height: content_height + 2 * PADDING,
            style: ContentStyle {
                background_color: Some(
                    THEME_DARK.get_background_color(BackgroundColorName::InputBox),
                ),
                ..Default::default()
            },
        };
        queue!(stdout, rect)?;

        let input = DrawInputBox {
            input: &self.input,
            x: content_x,
            y: content_y,
            anchor: Anchor::Bottom,
        };
        queue!(stdout, input)?;

        stdout.flush()?;
        Ok(())
    }
}

/// Runs the terminal app.
pub fn run() -> AnyResult<()> {
    enable_raw_mode()?;
    // Use a huge buffer to avoid flicker
    let mut stdout = io::BufWriter::with_capacity(1024 * 1024, io::stdout());
    execute!(stdout, EnterAlternateScreen, DisableLineWrap, Hide)?;

    let (w, _) = size()?;
    let mut chat = Chat::new(w, MAX_HEIGHT);
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
