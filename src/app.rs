use std::error::Error;
use std::io::Write;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::execute;
use crossterm::queue;
use crossterm::style::{ResetColor, SetBackgroundColor, SetForegroundColor};
use crossterm::terminal::{
    DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen,
};
use diesel::SqliteConnection;
use futures::StreamExt;

use crate::command::Command;
use crate::error::AnyResult;
use crate::model::Model;
use crate::session::{Content, ContentType, Item, ItemType, Session};
use crate::terminal::{DefaultTerminal, Terminal};
use crate::ui::canvas::Canvas;
use crate::ui::chat::{Chat, Update};
use crate::ui::style::{THEME_DARK, Theme};
use crate::ui::Component;
use crate::ui::styled_string::StyledString;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEvent {
    Command(String),
    /// Navigate to the previous entry in the command history.
    HistoryPrev,
    /// Navigate to the next entry in the command history.
    HistoryNext,
}

pub struct App {
    terminal: DefaultTerminal,
    chat: Chat,
    selected_model: Option<Model>,
    quit: bool,
    // XXX replace most uses of &'static Theme with Arc<Theme> or Rc<Theme>
    theme: &'static Theme,
    conn: SqliteConnection,
    session: Option<Session>,
}

impl App {
    pub fn new(terminal: DefaultTerminal) -> AnyResult<Self> {
        let (w, h) = terminal.size()?;
        let theme = &THEME_DARK;
        let chat = Chat::new(w, h, theme);
        Ok(Self {
            terminal,
            chat,
            selected_model: None,
            quit: false,
            theme,
            conn: crate::db::open()?,
            session: None,
        })
    }

    pub fn selected_model(&self) -> Option<&Model> {
        self.selected_model.as_ref()
    }

    pub fn switch_model(&mut self, model: Model) {
        self.selected_model = Some(model);
    }

    fn make_canvas(&self) -> Vec<StyledString> {
        let style = self.theme.base_style();
        (0..self.chat.height())
            .map(|_| StyledString::new(style, 2 * self.chat.width()))
            .collect()
    }

    pub fn draw(&self, canvas: Canvas) {
        self.chat.draw(canvas);
    }

    pub fn render(&mut self) -> AnyResult<()> {
        let mut rows = self.make_canvas();
        self.draw(&mut rows);

        for row in &rows {
            debug_assert!(row.width() <= self.chat.width());
        }

        let stdout = self.terminal.stdout();

        let text_style = self.theme.text_base;
        let bg = self.theme.bg_base;
        queue!(
            stdout,
            Hide,
            SetForegroundColor(text_style.fg_color),
            SetBackgroundColor(bg),
        )?;
        for (y, row) in rows.into_iter().enumerate() {
            queue!(stdout, MoveTo(0, y as u16), row)?;
        }
        if let Some((row, col)) = self.chat.cursor() {
            queue!(stdout, ResetColor, MoveTo(col as u16, row as u16), Show)?;
        }
        stdout.flush()?;
        Ok(())
    }

    pub async fn run(&mut self) -> AnyResult<()> {
        self.terminal.enable_raw_mode()?;
        execute!(self.terminal.stdout(), EnterAlternateScreen, DisableLineWrap)?;

        while !self.quit {
            self.render()?;
            let event = self.terminal
                .events()
                .next()
                .await
                .ok_or_else(|| std::io::Error::other("terminal stream closed"))??;
            let event = self.chat.handle_input(event);
            if let Some(event) = event {
                self.process_event(event);
            }
        }

        execute!(self.terminal.stdout(), EnableLineWrap, LeaveAlternateScreen)?;
        self.terminal.disable_raw_mode()?;

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

    /// Returns the ID of the current session, creating a new session if one
    /// does not exist.
    fn create_session(&mut self) -> AnyResult<i32> {
        if let Some(session) = &self.session {
            return Ok(session.id);
        }
        let session = Session::create(&mut self.conn, "Session")?;
        let id = session.id;
        self.session = Some(session);
        Ok(id)
    }

    /// Commits a new user message to the session and spawns an agent to
    /// respawn to it
    fn submit_prompt(&mut self, prompt: &str) -> AnyResult<(Item, Content)> {
        let session_id = self.create_session()?;
        let item = Item::create(
            &mut self.conn,
            session_id,
            None,
            ItemType::User,
            None,
        )?;
        let content = Content::create(
            &mut self.conn,
            item.id,
            ContentType::Text,
            prompt,
        )?;
        Ok((item, content))
    }

    fn process_command(&mut self, command: &str) -> AnyResult<()> {
        if command.trim().is_empty() {
            return Ok(());
        }

        let slash_command = command.starts_with('/');
        let bang_command = command.starts_with('!');
        if slash_command || bang_command {
            let command = &command[1..];
            if slash_command {
                let output = self.process_slash_command(command)?;
                if !output.trim().is_empty() {
                    self.chat.handle_update(Update::HelpMessage(&output));
                }
            } else {
                todo!("call system()")
            };
        } else {
            let (item, content) = self.submit_prompt(command)?;
            self.chat.handle_update(Update::ContentCreated { item: &item, content: &content });
        }

        Ok(())
    }

    fn process_event(&mut self, event: AppEvent) {
        let res = match event {
            AppEvent::Command(cmd) => self.process_command(&cmd),
            // Consumed by the StackedView; should never reach the App.
            AppEvent::HistoryPrev | AppEvent::HistoryNext => Ok(()),
        };
        if let Err(e) = res {
            self.chat.handle_update(Update::ErrorMessage(&e.to_string()));
        }
    }
}

pub async fn run() -> AnyResult<()> {
    App::new(DefaultTerminal::default())?.run().await
}
