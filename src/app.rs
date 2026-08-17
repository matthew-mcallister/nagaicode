use std::error::Error;
use std::io::Write;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event;
use crossterm::execute;
use crossterm::queue;
use crossterm::style::{ResetColor, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{
    DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode, size,
};

use crate::command::Command;
use crate::error::AnyResult;
use crate::model::Model;
use crate::ui::chat::Chat;
use crate::ui::history::HistoryItemContent;
use crate::ui::style::{Theme, THEME_DARK};
use crate::ui::Component;

#[derive(Clone, Debug, Eq, PartialEq)]
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
    selected_model: Option<Model>,
    quit: bool,
    theme: &'static Theme,
}

impl App {
    pub fn new() -> AnyResult<Self> {
        let (w, h) = size()?;
        let theme = &THEME_DARK;
        let chat = Chat::new(w, h, theme);
        Ok(Self {
            chat,
            selected_model: None,
            quit: false,
            theme,
        })
    }

    pub fn selected_model(&self) -> Option<&Model> {
        self.selected_model.as_ref()
    }

    pub fn switch_model(&mut self, model: Model) {
        self.selected_model = Some(model);
    }

    pub fn draw(&self, stdout: &mut impl Write) -> AnyResult<()> {
        let text_style = self.theme.text_base;
        let bg = self.theme.bg_base;
        queue!(stdout,
            Hide,
            SetForegroundColor(text_style.fg_color),
            SetBackgroundColor(bg),
        )?;
        for (y, row) in self.chat.drawable_rows().enumerate() {
            queue!(stdout, MoveTo(0, y as u16), row)?;
        }
        if let Some((row, col)) = self.chat.cursor() {
            queue!(stdout, ResetColor, MoveTo(col as u16, row as u16), Show)?;
        }
        stdout.flush()?;
        Ok(())
    }

    pub fn run(&mut self) -> AnyResult<()> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen, DisableLineWrap)?;

        while !self.quit {
            self.draw(&mut stdout)?;
            let event = self.chat.handle_event(event::read()?.into());
            if let Some(event) = event {
                self.process_event(event);
            }
        }

        execute!(stdout, EnableLineWrap, LeaveAlternateScreen)?;
        disable_raw_mode()?;

        Ok(())
    }

    fn process_slash_command(&mut self, command: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
        let command = match crate::command::parse_command(command) {
            Ok(x) => x,
            Err(e) => return Ok(e.to_string()),
        };
        match command {
            Command::Provider(cmd) => crate::command::run_provider_command(cmd),
            Command::Model(cmd) => crate::command::run_model_command(self, cmd),
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
