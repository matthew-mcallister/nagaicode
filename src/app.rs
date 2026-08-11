use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode, size,
};

use crate::error::AnyResult;
use crate::ui::chat::Chat;
use crate::ui::style::THEME_DARK;

/// Runs the terminal app.
pub fn run() -> AnyResult<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, DisableLineWrap)?;

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

    execute!(stdout, EnableLineWrap, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}
