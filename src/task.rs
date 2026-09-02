//! Task spawning, cancelation, and lifecycle tracking.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use diesel::SqliteConnection;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::app::AppEvent;
use crate::cwd::Cwd;
use crate::error::AnyResult;
use crate::tools::ToolRegistry;

/// Unique identifier for a spawned task.
pub type Tid = u64;

/// Reason a task ended without producing output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskError {
    /// The task was canceled before completion.
    Canceled,
}

/// Handle passed to a running task.
// TODO: Should probably expose some of these fields directly instead of hiding
// behind methods to deal with conflicting borrows
pub struct TaskContext {
    tid_counter: Arc<AtomicU64>,
    cancel: CancellationToken,
    sender: UnboundedSender<AppEvent>,
    db_url: String,
    connection: Option<SqliteConnection>,
    tool_registry: Arc<ToolRegistry>,
    cwd: Arc<Cwd>,
}

impl TaskContext {
    /// Creates a root context for spawning top-level tasks.
    pub(crate) fn root(
        tid_counter: Arc<AtomicU64>,
        sender: UnboundedSender<AppEvent>,
        db_url: String,
        tool_registry: Arc<ToolRegistry>,
        cwd: Arc<Cwd>,
    ) -> Self {
        Self {
            tid_counter,
            cancel: CancellationToken::new(),
            sender,
            db_url,
            connection: None,
            tool_registry,
            cwd,
        }
    }

    /// Sends an event to the app.
    pub fn send(&self, event: AppEvent) {
        let _ = self.sender.send(event);
    }

    /// Returns the channel used to send events to the app.
    pub fn sender(&self) -> &UnboundedSender<AppEvent> {
        &self.sender
    }

    /// Returns the task's database connection, opening it on first use.
    pub fn connection(&mut self) -> AnyResult<&mut SqliteConnection> {
        if self.connection.is_none() {
            self.connection = Some(crate::db::open(&self.db_url)?);
        }
        Ok(self.connection.as_mut().expect("connection opened"))
    }

    /// Returns the tool registry.
    pub fn tool_registry(&self) -> &Arc<ToolRegistry> {
        &self.tool_registry
    }

    /// Returns the current working directory.
    pub fn cwd(&self) -> &Arc<Cwd> {
        &self.cwd
    }

    /// Creates a child context sharing this context's state, using `cancel`.
    pub fn fork(&self, cancel: CancellationToken) -> Self {
        Self {
            tid_counter: Arc::clone(&self.tid_counter),
            cancel,
            sender: self.sender.clone(),
            db_url: self.db_url.clone(),
            connection: None,
            tool_registry: Arc::clone(&self.tool_registry),
            cwd: Arc::clone(&self.cwd),
        }
    }

    /// Creates a Future out of a Task.
    pub fn subtask<T: Task>(&self, task: T) -> impl Future<Output = T::Output> + Send {
        let tid = self.tid_counter.fetch_add(1, Ordering::Relaxed);
        let mut context = self.fork(self.cancel.clone());
        async move {
            let _ = context.sender.send(AppEvent::TaskStarted(tid));
            let output = task.run(&mut context).await;
            let _ = context.sender.send(AppEvent::TaskEnded(tid));
            output
        }
    }

    /// Spawns `task` as a child of this context. Canceling the parent
    /// cancels all descendants transitively.
    pub fn spawn<T: Task>(&self, task: T) -> TaskHandle<T::Output> {
        let tid = self.tid_counter.fetch_add(1, Ordering::Relaxed);
        let cancel = self.cancel.child_token();
        let mut context = self.fork(cancel.clone());
        let sender = self.sender.clone();
        let inner = cancel.clone();
        let join = tokio::spawn(async move {
            let _ = sender.send(AppEvent::TaskStarted(tid));
            let result = tokio::select! {
                biased;
                _ = inner.cancelled() => Err(TaskError::Canceled),
                output = task.run(&mut context) => Ok(output),
            };
            let _ = sender.send(AppEvent::TaskEnded(tid));
            result
        });
        TaskHandle { tid, cancel, join }
    }
}

/// Handle to a spawned task.
pub struct TaskHandle<R> {
    /// Unique id of the task.
    tid: Tid,
    cancel: CancellationToken,
    join: JoinHandle<Result<R, TaskError>>,
}

impl<R> TaskHandle<R> {
    pub fn tid(&self) -> Tid {
        self.tid
    }

    /// Cancels the task as well as all child/descendant tasks.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Waits for the task to finish.
    pub async fn join(self) -> Result<Result<R, TaskError>, JoinError> {
        self.join.await
    }
}

/// Light wrapper for helping with cancelation and statistic tracking. In
/// future will help with status bar messages.
pub trait Task: Send + 'static {
    type Output: Send + 'static;

    /// Runs the task. The task may be canceled at any time, so expect early
    /// return at all await points.
    fn run(self, context: &mut TaskContext) -> impl Future<Output = Self::Output> + Send;
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::unbounded_channel;

    use crate::testing::DummyTask;

    use super::*;

    #[tokio::test]
    async fn test_spawn_cancels() {
        let (sender, mut recv) = unbounded_channel();
        let context = crate::testing::task_context(sender);
        let handle = context.spawn(DummyTask::new());
        handle.cancel();
        let result = handle.join().await.unwrap();
        assert_eq!(result, Err(TaskError::Canceled));
        assert_eq!(recv.try_recv().unwrap(), AppEvent::TaskStarted(0));
        assert_eq!(recv.try_recv().unwrap(), AppEvent::TaskEnded(0));
    }
}
