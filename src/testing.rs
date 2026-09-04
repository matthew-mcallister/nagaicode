//! Testing common code

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::task::Poll;

use diesel::SqliteConnection;
use futures::{Stream, pin_mut};
use serde_json::Value;
use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppEvent;
use crate::task::{Task, TaskContext};
use crate::tools::ToolRegistry;
use crate::item::{Item, ItemContent, NewItem, ToolCallContent, ToolOutput};
use crate::session::{Session, Turn, TurnType};
use crate::ui::UiContext;

pub fn tool_registry() -> Arc<ToolRegistry> {
    Arc::new(ToolRegistry::new(&Arc::new(crate::cwd::cwd())))
}

pub fn session_turn(conn: &mut SqliteConnection) -> (Session, Turn) {
    let session = Session::create(conn, "Session").unwrap();
    let turn = Turn::create(conn, session.id, TurnType::Assistant, None, None, None)
        .unwrap();
    (session, turn)
}

pub fn tool_call(
    conn: &mut SqliteConnection,
    turn: &Turn,
    name: &str,
    call_id: &str,
    args: Value,
    output: Option<ToolOutput>,
) -> Item {
    Item::create(
        conn,
        NewItem {
            session_id: turn.session_id,
            turn_id: turn.id,
            response_id: None,
            provider_id: None,
            upstream_id: None,
            seqno: None,
            content: ItemContent::ToolCall(ToolCallContent {
                tool_name: name.to_owned(),
                call_id: call_id.to_owned(),
                args,
                output,
            }),
        },
    ).unwrap()
}

pub fn ui_context() -> UiContext {
    UiContext::new(tool_registry())
}

pub fn task_context(sender: UnboundedSender<AppEvent>) -> TaskContext {
    TaskContext::root(
        Arc::new(AtomicU64::new(0)),
        sender,
        crate::db::db_url().unwrap(),
        tool_registry(),
        Arc::new(crate::cwd::cwd()),
    )
}

/// A task that does nothing until it is signaled to complete. Used for
/// testing task lifecycle behavior.
#[derive(Clone)]
pub struct DummyTask {
    complete: Arc<Notify>,
}

impl Default for DummyTask {
    fn default() -> Self {
        Self::new()
    }
}

impl DummyTask {
    /// Creates a new dummy task.
    pub fn new() -> Self {
        Self {
            complete: Arc::new(Notify::new()),
        }
    }

    /// Signals the task to complete successfully.
    pub fn complete(&self) {
        self.complete.notify_one();
    }
}

impl Task for DummyTask {
    type Output = ();

    async fn run(self, _context: &mut TaskContext) {
        self.complete.notified().await;
    }
}

/// Simulates an async stream by yielding from a queue.
#[derive(Clone, Debug)]
pub struct QueueStream<E>(pub VecDeque<E>);

impl<E> Default for QueueStream<E> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<E> Unpin for QueueStream<E> {}

impl<E, T> From<T> for QueueStream<E>
where
    VecDeque<E>: From<T>,
{
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

impl<E> Stream for QueueStream<E> {
    type Item = E;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self;
        pin_mut!(this);
        Poll::Ready(this.0.pop_front())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.0.len(), Some(self.0.len()))
    }
}
