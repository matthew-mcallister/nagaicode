use std::error::Error;
use std::io::Write;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::execute;
use crossterm::queue;
use crossterm::style::ResetColor;
use crossterm::terminal::{
    DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen,
};
use diesel::SqliteConnection;
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_util::sync::CancellationToken;

use crate::agent::Agent;
use crate::command::Command;
use crate::error::AnyResult;
use crate::model::Model;
use crate::model::revalidate_models;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::{Content, ContentType, Item, ItemType, Session};
use crate::terminal::{DefaultTerminal, Terminal};
use crate::tools::{DefaultToolServer, ToolServer};
use crate::ui::canvas::Canvas;
use crate::ui::chat::{Chat, Update};
use crate::ui::style::{THEME_DARK, Theme};
use crate::ui::Component;
use crate::ui::styled_string::StyledString;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEvent {
    Command(String),
    ContentCreated { item: Item, content: Content },
    ContentUpdated { item: Item, content: Content },
    /// Navigate to the previous entry in the command history.
    HistoryPrev,
    /// Navigate to the next entry in the command history.
    HistoryNext,
    /// Cancel the active task.
    Interrupt,
    /// Report an error from a background task.
    ErrorMessage(String),
}

#[derive(Debug)]
enum Poll {
    Input(crossterm::event::Event),
    Event(AppEvent),
}

pub struct App {
    terminal: DefaultTerminal,
    chat: Chat,
    selected_model: Option<Model>,
    quit: bool,
    // XXX replace most uses of &'static Theme with Arc<Theme> or Rc<Theme>
    theme: &'static Theme,
    client: DefaultClient,
    conn: SqliteConnection,
    db_url: String,
    session: Option<Session>,
    tools: DefaultToolServer,
    // Channel for async events
    send: UnboundedSender<AppEvent>,
    recv: UnboundedReceiver<AppEvent>,
    // Cancel token for interrupting agents/async tasks
    cancel: Option<CancellationToken>,
}

impl App {
    /// Creates a new `App` instance.
    pub fn new() -> AnyResult<Self> {
        let terminal = DefaultTerminal::default();
        let (w, h) = terminal.size()?;
        let theme = &THEME_DARK;
        let chat = Chat::new(w, h, theme);
        let (send, recv) = unbounded_channel();
        let db_url = crate::db::db_url()?;
        Ok(Self {
            terminal,
            chat,
            selected_model: None,
            quit: false,
            theme,
            client: DefaultClient::default(),
            conn: crate::db::open(&db_url)?,
            db_url,
            session: None,
            tools: DefaultToolServer::default(),
            send,
            recv,
            cancel: None,
        })
    }

    /// Returns the currently selected model, if any.
    pub fn selected_model(&self) -> Option<&Model> {
        self.selected_model.as_ref()
    }

    /// Switches the selected model.
    pub fn switch_model(&mut self, model: Model) {
        self.selected_model = Some(model);
    }

    /// Returns a reference to the terminal.
    pub fn terminal(&self) -> &DefaultTerminal {
        &self.terminal
    }

    /// Returns a mutable reference to the terminal.
    pub fn terminal_mut(&mut self) -> &mut DefaultTerminal {
        &mut self.terminal
    }

    /// Returns a reference to the tool server.
    pub fn tools(&self) -> &DefaultToolServer {
        &self.tools
    }

    /// Returns a mutable reference to the database connection.
    pub(crate) fn conn(&mut self) -> &mut SqliteConnection {
        &mut self.conn
    }

    /// Returns a mutable reference to the tool server.
    pub fn tools_mut(&mut self) -> &mut DefaultToolServer {
        &mut self.tools
    }

    /// Creates a blank canvas matching the chat dimensions and theme.
    pub fn make_canvas(&self) -> Vec<StyledString> {
        let style = self.theme.base_style();
        (0..self.chat.height())
            .map(|_| StyledString::new(style, 2 * self.chat.width()))
            .collect()
    }

    /// Draws the chat component onto the given canvas.
    pub fn draw(&self, canvas: Canvas) {
        self.chat.draw(canvas);
    }

    /// Renders the current state to the terminal.
    pub fn render(&mut self) -> AnyResult<()> {
        let mut rows = self.make_canvas();
        self.draw(&mut rows);

        for row in &rows {
            debug_assert!(row.width() <= self.chat.width());
        }

        let stdout = self.terminal.stdout();

        queue!(stdout, Hide)?;
        for (y, row) in rows.into_iter().enumerate() {
            queue!(stdout, MoveTo(0, y as u16), row)?;
        }
        if let Some((row, col)) = self.chat.cursor() {
            queue!(stdout, ResetColor, MoveTo(col as u16, row as u16), Show)?;
        }
        stdout.flush()?;
        Ok(())
    }

    /// Handles a terminal input event.
    pub fn handle_input(&mut self, input: crossterm::event::Event) {
        let event = self.chat.handle_input(input);
        if let Some(event) = event {
            self.process_event(event);
        }
    }

    /// Returns whether the app has received a quit signal.
    pub fn quit(&self) -> bool {
        self.quit
    }

    /// Returns whether the active task has been canceled.
    /// TODO: delete this
    #[cfg(test)]
    pub fn task_canceled(&self) -> bool {
        self.cancel.as_ref().is_none()
    }

    /// Spawns a background task to revalidate stale model lists.
    fn spawn_revalidate_models(&self) {
        let db_url = self.db_url.clone();
        tokio::spawn(async move {
            let mut conn = match crate::db::open(&db_url) {
                Ok(conn) => conn,
                Err(e) => {
                    eprintln!("failed to open db for revalidation: {e}");
                    return;
                }
            };
            if let Err(e) = revalidate_models(&mut conn).await {
                eprintln!("failed to revalidate models: {e}");
            }
        });
    }

    /// Runs the main event loop.
    pub async fn run(&mut self) -> AnyResult<()> {
        self.spawn_revalidate_models();
        self.terminal.enable_raw_mode()?;
        execute!(self.terminal.stdout(), EnterAlternateScreen, DisableLineWrap)?;

        while !self.quit {
            self.render()?;
            let poll = tokio::select! {
                input = self.terminal.events().next() => {
                    Poll::Input(input.ok_or_else(|| std::io::Error::other("terminal stream closed"))??)
                }
                event = self.recv.recv() => Poll::Event(event.expect("channel closed")),
            };
            match poll {
                Poll::Input(event) => self.handle_input(event),
                Poll::Event(event) => self.process_event(event),
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
            Command::Provider(cmd) => crate::command::run_provider_command(&mut self.conn, cmd),
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

    /// Cancels the active task.
    fn cancel(&mut self) {
        if let Some(cancel) = self.cancel.as_mut() {
            cancel.cancel();
            self.cancel = None;
        }
    }

    /// Spawns an agent to handle the submitted prompt.
    fn submit_prompt(&mut self, prompt: &str) -> AnyResult<()> {
        self.cancel();

        let model = self.selected_model.clone().ok_or("no model selected")?;
        let provider = Provider::get_by_id(&mut self.conn, model.provider_id)?
            .ok_or_else(|| format!("no provider found for model '{}'", model.id))?;

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

        let cancel = CancellationToken::new();
        let agent = Agent::new(
            item,
            content,
            provider,
            model,
            self.send.clone(),
            self.client.clone(),
            crate::db::open(&self.db_url)?,
            cancel.clone(),
        );

        agent.spawn();
        self.cancel = Some(cancel);

        Ok(())
    }

    // TODO eventually: execute these as an asynchronous and interruptable
    // agent and stream stdout to history
    fn process_bang_command(&mut self, command: &str) -> AnyResult<String> {
        let result = self.tools.call("sh", serde_json::json!(command))?;
        let output = if let Some(obj) = result.content.as_object() {
            let stdout = obj.get("stdout").and_then(Value::as_str).unwrap_or("");
            let stderr = obj.get("stderr").and_then(Value::as_str).unwrap_or("");
            format!("{stdout}{stderr}")
        } else if let Some(s) = result.content.as_str() {
            s.to_string()
        } else {
            result.content.to_string()
        };
        Ok(output)
    }

    pub(crate) fn process_command(&mut self, command: &str) -> AnyResult<()> {
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
                let output = self.process_bang_command(command)?;
                let prompt = format!("$ {command}");
                self.chat.handle_update(Update::CommandPrompt(&prompt));
                self.chat.handle_update(Update::CommandOutput(&output));
            }
        } else {
            self.submit_prompt(command)?;
        }

        Ok(())
    }

    pub(crate) fn process_event(&mut self, event: AppEvent) {
        let res = match event {
            AppEvent::Command(cmd) => self.process_command(&cmd),
            AppEvent::ContentCreated { item, content } => {
                self.chat.handle_update(Update::ContentCreated { item: &item, content: &content });
                Ok(())
            }
            AppEvent::ContentUpdated { item, content } => {
                self.chat.handle_update(Update::ContentUpdated { item: &item, content: &content });
                Ok(())
            }
            AppEvent::HistoryPrev | AppEvent::HistoryNext => Ok(()),
            AppEvent::Interrupt => {
                if let Some(cancel) = &self.cancel {
                    cancel.cancel();
                }
                self.cancel = None;
                Ok(())
            }
            AppEvent::ErrorMessage(msg) => {
                self.chat.handle_update(Update::ErrorMessage(&msg));
                Ok(())
            }
        };
        if let Err(e) = res {
            self.chat.handle_update(Update::ErrorMessage(&e.to_string()));
        }
    }
}

/// Runs the app.
pub async fn run() -> AnyResult<()> {
    App::new()?.run().await
}
