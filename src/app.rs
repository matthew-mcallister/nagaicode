use std::error::Error;

use crossterm::event;
use crossterm::execute;
use crossterm::terminal::{
    DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode, size,
};

use crate::command::Command;
use crate::error::AnyResult;
use crate::ui::chat::Chat;
use crate::ui::history::HistoryItemContent;
use crate::ui::style::THEME_DARK;

#[derive(Debug)]
pub enum AppEvent {
    Command(String),
    /// Navigate to the previous entry in the command history.
    HistoryPrev,
    /// Navigate to the next entry in the command history.
    HistoryNext,
}

#[derive(Debug)]
pub struct App {
    chat: Chat,
    quit: bool,
}

impl App {
    pub fn new() -> AnyResult<Self> {
        let (w, h) = size()?;
        let chat = Chat::new(w, h, &THEME_DARK);
        Ok(Self {
            chat,
            quit: false,
        })
    }

    pub fn run(&mut self) -> AnyResult<()> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen, DisableLineWrap)?;

        while !self.quit {
            self.chat.draw(&mut stdout)?;
            let event = self.chat.handle_event(event::read()?);
            if let Some(event) = event {
                self.process_event(event);
            }
        }

        execute!(stdout, EnableLineWrap, LeaveAlternateScreen)?;
        disable_raw_mode()?;

        Ok(())
    }

    fn process_slash_command(&mut self, command: &str) -> Result<String, Box<dyn Error>> {
        let command = match crate::command::parse_command(command) {
            Ok(x) => x,
            Err(e) => return Ok(e.to_string()),
        };
        match command {
            Command::Provider(cmd) => crate::command::run_provider_command(cmd),
            Command::Quit => {
                self.quit = true;
                Ok(String::new())
            },
        }
    }

    fn process_command(&mut self, command: &str) {
        if command.trim().is_empty() {
            return;
        }

        let slash_command = command.starts_with('/');
        let bang_command = command.starts_with('!');
        if slash_command || bang_command {
            let command = &command[1..];
            if slash_command {
                match self.process_slash_command(command) {
                    Ok(output) => {
                        if !output.trim().is_empty() {
                            self.chat.add_item(HistoryItemContent::Help(output));
                        }
                    }
                    Err(e) => {
                        self.chat.add_item(HistoryItemContent::Error(e.to_string()));
                    }
                }
            } else {
                todo!("call system()")
            };
            return;
        }

        self.chat.add_item(HistoryItemContent::Markdown(command.into()));
    }

    fn process_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Command(cmd) => self.process_command(&cmd),
            // Consumed by the StackedView; should never reach the App.
            AppEvent::HistoryPrev | AppEvent::HistoryNext => {},
        }
    }
}

pub fn run() -> AnyResult<()> {
    App::new()?.run()
}
