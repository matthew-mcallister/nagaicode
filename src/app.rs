use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use anyhow::anyhow;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::execute;
use crossterm::queue;
use crossterm::style::ResetColor;
use crossterm::terminal::{
    DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen,
};
use diesel::SqliteConnection;
use fnv::FnvHashSet;
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::agent::Agent;
use crate::command::Command;
use crate::error::AnyResult;
use crate::model::Model;
use crate::model::RevalidateModelsTask;
use crate::provider::Provider;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::request::DefaultClient;
use crate::session::{Item, ItemType, NewItem, Session, Turn, TurnType};
use crate::settings::{ModelRef, Settings};
use crate::tasks::{Task, TaskContext, TaskError, TaskHandle, Tid};
use crate::terminal::{DefaultTerminal, Terminal};
use crate::tools::{DefaultToolServer, ToolResult, ToolServer};
use crate::ui::Component;
use crate::ui::canvas::Canvas;
use crate::ui::chat::{Chat, Update};
use crate::ui::style::{THEME_DARK, Theme};
use crate::ui::styled_string::StyledString;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEvent {
    Command(String),
    ItemCreated {
        item: Item,
    },
    ItemUpdated {
        item: Item,
    },
    /// Navigate to the previous entry in the command history.
    HistoryPrev,
    /// Navigate to the next entry in the command history.
    HistoryNext,
    /// Cancel the active task.
    Interrupt,
    /// The active task was canceled.
    Interrupted,
    /// Report an error from a background task.
    ErrorMessage(String),
    /// A task has started running.
    TaskStarted(Tid),
    /// A task has ended.
    TaskEnded(Tid),
}

#[derive(Debug)]
enum Poll {
    Input(crossterm::event::Event),
    Event(Box<AppEvent>),
}

pub struct App {
    terminal: DefaultTerminal,
    chat: Chat,
    selected_model: Option<(Provider, Model)>,
    quit: bool,
    // XXX replace most uses of &'static Theme with Arc<Theme> or Rc<Theme>
    theme: &'static Theme,
    client: DefaultClient,
    conn: SqliteConnection,
    db_url: String,
    settings: Settings,
    session: Option<Session>,
    tools: DefaultToolServer,
    // Channel for async events
    send: UnboundedSender<AppEvent>,
    recv: UnboundedReceiver<AppEvent>,
    tid_counter: Arc<AtomicU64>,
    // Tracks all tasks including background and child tasks. Sole purpose is
    // stats for the status bar.
    tasks: FnvHashSet<Tid>,
    // Bookkeeping to ensure one foreground task at a time and to allow
    // manual interrupt.
    current_task: Option<TaskHandle<()>>,
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
        let mut conn = crate::db::open(&db_url)?;
        let settings = Settings::open(&db_url)?;
        let selected_model = settings
            .current_model()
            .and_then(|r| r.resolve(&mut conn).ok())
            .flatten();
        Ok(Self {
            terminal,
            chat,
            selected_model,
            quit: false,
            theme,
            client: DefaultClient::default(),
            conn,
            db_url,
            settings,
            session: None,
            tools: DefaultToolServer::default(),
            send,
            recv,
            tid_counter: Arc::new(AtomicU64::new(0)),
            tasks: FnvHashSet::default(),
            current_task: None,
        })
    }

    /// Returns the currently selected provider and model, if any.
    pub fn selected_model(&self) -> Option<&(Provider, Model)> {
        self.selected_model.as_ref()
    }

    /// Returns the database URL.
    pub fn db_url(&self) -> &str {
        &self.db_url
    }

    /// Switches the selected model and persists the choice across runs.
    pub fn switch_model(&mut self, provider: Provider, model: Model) -> AnyResult<()> {
        self.settings.set_current_model(Some(ModelRef {
            provider: provider.name.clone(),
            model: model.id.clone(),
        }))?;
        self.selected_model = Some((provider, model));
        Ok(())
    }

    /// Clears the selected model if it belonged to a deleted provider.
    pub fn on_provider_deleted(&mut self, provider_name: &str) -> AnyResult<()> {
        let deleted_selected = match &self.selected_model {
            Some((provider, _)) => provider.name == provider_name,
            None => false,
        };
        if !deleted_selected {
            return Ok(());
        }
        self.settings.set_current_model(None)?;
        self.selected_model = None;
        Ok(())
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

    /// Returns a mutable reference to the HTTP client.
    pub fn client_mut(&mut self) -> &mut DefaultClient {
        &mut self.client
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
    pub async fn handle_input(&mut self, input: crossterm::event::Event) {
        let event = self.chat.handle_input(input);
        if let Some(event) = event {
            self.process_event(event).await;
        }
    }

    /// Returns whether the app has received a quit signal.
    pub fn quit(&self) -> bool {
        self.quit
    }

    /// Spawns a background task to revalidate stale model lists.
    fn spawn_revalidate_models(&mut self) {
        self.spawn_background(RevalidateModelsTask);
    }

    /// Runs the main event loop.
    pub async fn run(&mut self) -> AnyResult<()> {
        self.spawn_revalidate_models();
        self.terminal.enable_raw_mode()?;
        execute!(
            self.terminal.stdout(),
            EnterAlternateScreen,
            DisableLineWrap
        )?;

        while !self.quit {
            self.render()?;
            let poll = tokio::select! {
                input = self.terminal.events().next() => {
                    Poll::Input(input.ok_or_else(|| std::io::Error::other("terminal stream closed"))??)
                }
                event = self.recv.recv() => Poll::Event(Box::new(event.expect("channel closed"))),
            };
            match poll {
                Poll::Input(event) => self.handle_input(event).await,
                Poll::Event(event) => self.process_event(*event).await,
            }
        }

        execute!(self.terminal.stdout(), EnableLineWrap, LeaveAlternateScreen)?;
        self.terminal.disable_raw_mode()?;

        Ok(())
    }

    fn process_slash_command(
        &mut self,
        command: &str,
    ) -> AnyResult<String> {
        let command = match crate::command::parse_command(command) {
            Ok(x) => x,
            Err(e) => return Ok(e.to_string()),
        };
        match command {
            Command::Provider(cmd) => crate::command::run_provider_command(self, cmd),
            Command::Model(cmd) => crate::command::run_model_command(self, cmd),
            Command::Quit => {
                self.quit = true;
                Ok(String::new())
            }
        }
    }

    /// Returns the current session, creating a new session if one does not
    /// exist.
    fn create_session(&mut self) -> AnyResult<Session> {
        if let Some(session) = &self.session {
            return Ok(session.clone());
        }
        let session = Session::create(&mut self.conn, "Session")?;
        self.session = Some(session.clone());
        Ok(session)
    }

    /// Processes any events sent by the active task that are still pending.
    pub async fn process_pending_events(&mut self) {
        while let Ok(event) = self.recv.try_recv() {
            self.process_event(event).await;
        }
    }

    /// Returns a root task context for spawning tasks.
    fn context(&self) -> TaskContext {
        TaskContext::root(
            Arc::clone(&self.tid_counter),
            self.send.clone(),
            self.db_url.clone(),
        )
    }

    /// Spawns a task as the current foreground task. Interrupts any currently
    /// running task.
    async fn spawn_foreground<T: Task<Output = ()>>(&mut self, task: T) {
        self.cancel_task().await;
        let handle = self.context().spawn(task);
        self.current_task = Some(handle);
    }

    /// Spawns a detached background task.
    fn spawn_background<T: Task<Output = ()>>(&mut self, task: T) -> TaskHandle<()> {
        self.context().spawn(task)
    }

    /// Cancels the active task and waits for it to complete.
    async fn cancel_task(&mut self) {
        let Some(task) = self.current_task.take() else { return };
        task.cancel();
        self.finish_task(task).await;
    }

    /// Waits for a task to finish and posts a message to the history if it was
    /// canceled.
    async fn finish_task(&mut self, task: TaskHandle<()>) {
        let tid = task.tid();
        match task.join().await {
            Ok(Ok(())) => {}
            Ok(Err(TaskError::Canceled)) => {
                let _ = self.send.send(AppEvent::Interrupted);
            }
            Err(e) => {
                // Panicked tasks never send TaskEnded, so clean up here.
                self.tasks.remove(&tid);
                log::error!("task {tid} did not end cleanly: {e}");
            }
        }
    }

    /// Awaits completion of the active task, if any.
    pub async fn await_task(&mut self) -> AnyResult<()> {
        if let Some(task) = self.current_task.take() {
            self.finish_task(task).await;
        }
        Ok(())
    }

    /// Spawns a dummy task tracked as the current task, for testing.
    #[cfg(test)]
    pub(crate) async fn spawn_dummy_task(&mut self) -> crate::testing::DummyTask {
        let task = crate::testing::DummyTask::new();
        let control = task.clone();
        self.spawn_foreground(task).await;
        control
    }

    /// Spawns an agent to handle the submitted prompt.
    async fn submit_prompt(&mut self, prompt: &str) -> AnyResult<()> {
        let (provider, model) = self
            .selected_model
            .clone()
            .ok_or_else(|| anyhow!("no model selected"))?;

        let session = self.create_session()?;
        let turn = Turn::create(&mut self.conn, session.id, TurnType::User, None, None, None)?;
        let item = Item::create(
            &mut self.conn,
            NewItem {
                session_id: session.id,
                turn_id: turn.id,
                response_id: None,
                provider_id: None,
                ty: ItemType::UserText,
                upstream_id: None,
                upstream_type: None,
                upstream_call_id: None,
                text: Some(prompt),
            },
        )?;

        let _ = self.send.send(AppEvent::ItemCreated { item: item.clone() });

        let agent = Agent::new(
            session,
            provider,
            model,
            self.client.clone(),
            self.tools.clone(),
        );
        self.spawn_foreground(agent).await;

        Ok(())
    }

    // TODO eventually: execute these as an asynchronous and interruptable
    // agent and stream stdout to history
    async fn process_bang_command(&mut self, command: &str) -> AnyResult<String> {
        let result = self.tools.call("sh", json!({ "command": command })).await;
        match result {
            ToolResult::Text(msg) => Err(anyhow!(msg)),
            ToolResult::Json(value) => {
                let obj = value.as_object().ok_or_else(|| {
                    anyhow!("invalid result for 'sh': expected an object")
                })?;
                let stdout = obj.get("stdout").and_then(Value::as_str).unwrap_or("");
                let stderr = obj.get("stderr").and_then(Value::as_str).unwrap_or("");
                let return_code = obj
                    .get("return_code")
                    .and_then(Value::as_i64)
                    .unwrap_or(-1);
                if return_code == 0 {
                    Ok(format!("{stdout}{stderr}"))
                } else {
                    Err(anyhow!(
                        "command exited with code {return_code}: {stdout}{stderr}"
                    ))
                }
            }
        }
    }

    pub(crate) async fn process_command(&mut self, command: &str) -> AnyResult<()> {
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
                let output = self.process_bang_command(command).await?;
                let prompt = format!("$ {command}");
                self.chat.handle_update(Update::CommandPrompt(&prompt));
                self.chat.handle_update(Update::CommandOutput(&output));
            }
        } else {
            self.submit_prompt(command).await?;
        }

        Ok(())
    }

    pub(crate) async fn process_event(&mut self, event: AppEvent) {
        let res = match event {
            AppEvent::Command(cmd) => self.process_command(&cmd).await,
            AppEvent::ItemCreated { item } => {
                self.chat.handle_update(Update::ItemCreated { item: &item });
                Ok(())
            }
            AppEvent::ItemUpdated { item } => {
                self.chat.handle_update(Update::ItemUpdated { item: &item });
                Ok(())
            }
            AppEvent::HistoryPrev | AppEvent::HistoryNext => Ok(()),
            AppEvent::Interrupt => {
                self.cancel_task().await;
                Ok(())
            }
            AppEvent::Interrupted => {
                self.chat.handle_update(Update::HelpMessage("Interrupted."));
                Ok(())
            }
            AppEvent::ErrorMessage(msg) => {
                self.chat.handle_update(Update::ErrorMessage(&msg));
                Ok(())
            }
            AppEvent::TaskStarted(tid) => {
                self.tasks.insert(tid);
                Ok(())
            }
            AppEvent::TaskEnded(tid) => {
                self.tasks.remove(&tid);
                if self.current_task.as_ref().is_some_and(|t| t.tid() == tid) {
                    let task = self.current_task.take().expect("current task matches");
                    self.finish_task(task).await;
                }
                Ok(())
            }
        };
        if let Err(e) = res {
            self.chat
                .handle_update(Update::ErrorMessage(&e.to_string()));
        }
    }
}

/// Exposed fields:
/// - chat: Chat
/// - selected_model: Model | null
/// - db_url: string
/// - session: Session | null
/// - current_task: int | null
/// - task_count: int
impl DataQuery for App {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "chat": self.query("/chat")?,
                "selected_model": self.query("/selected_model")?,
                "db_url": self.query("/db_url")?,
                "session": self.query("/session")?,
                "current_task": self.query("/current_task")?,
                "task_count": self.query("/task_count")?,
            }))),
            "chat" => Ok(QueryField::DataQuery(&self.chat)),
            "selected_model" => match self.selected_model.as_ref() {
                Some((_, model)) => Ok(QueryField::DataQuery(model)),
                None => Ok(QueryField::Value(json!(null))),
            },
            "db_url" => Ok(QueryField::Value(json!(self.db_url))),
            "session" => match self.session.as_ref() {
                Some(session) => Ok(QueryField::DataQuery(session)),
                None => Ok(QueryField::Value(json!(null))),
            },
            "current_task" => Ok(QueryField::Value(json!(
                self.current_task.as_ref().map(|t| t.tid())
            ))),
            "task_count" => Ok(QueryField::Value(json!(self.tasks.len()))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

/// Runs the app.
pub async fn run() -> AnyResult<()> {
    App::new()?.run().await
}
