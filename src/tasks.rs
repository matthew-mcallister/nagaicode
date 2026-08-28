//! Task spawning, cancelation, and lifecycle tracking.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc::UnboundedSender;
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::app::AppEvent;

/// Unique identifier for a spawned task.
pub type Tid = u64;

/// Reason a task ended without producing output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskError {
    /// The task was canceled before completion.
    Canceled,
}

/// Handle passed to a running task.
pub struct TaskContext {
    tid_counter: Arc<AtomicU64>,
    cancel: CancellationToken,
    sender: UnboundedSender<AppEvent>,
}

impl TaskContext {
    /// Creates a root context for spawning top-level tasks.
    pub(crate) fn root(tid_counter: Arc<AtomicU64>, sender: UnboundedSender<AppEvent>) -> Self {
        Self {
            tid_counter,
            cancel: CancellationToken::new(),
            sender,
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

    /// Spawns `task` as a child of this context. Canceling the parent
    /// cancels all descendants transitively.
    pub fn spawn<T: Task>(&self, task: T) -> TaskHandle<T::Output> {
        let tid = self.tid_counter.fetch_add(1, Ordering::Relaxed);
        let cancel = self.cancel.child_token();
        let context = Self {
            tid_counter: Arc::clone(&self.tid_counter),
            cancel: cancel.clone(),
            sender: self.sender.clone(),
        };
        let sender = self.sender.clone();
        let watch = cancel.clone();
        let join = tokio::spawn(async move {
            let _ = sender.send(AppEvent::TaskStarted(tid));
            let result = tokio::select! {
                biased;
                _ = watch.cancelled() => Err(TaskError::Canceled),
                output = task.run(context) => Ok(output),
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

/// Unit of async work.
pub trait Task: Send + 'static {
    type Output: Send + 'static;

    /// Runs the task. The task may be canceled at any time, so expect early
    /// return at all await points.
    fn run(self, context: TaskContext) -> impl Future<Output = Self::Output> + Send;
}
