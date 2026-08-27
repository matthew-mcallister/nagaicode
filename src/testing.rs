//! Testing common code

use std::collections::VecDeque;
use std::sync::Arc;
use std::task::Poll;

use futures::{Stream, pin_mut};
use tokio::sync::Notify;

use crate::tasks::{Task, TaskContext};

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

    async fn run(self, _context: TaskContext) {
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
