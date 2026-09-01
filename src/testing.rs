//! Testing common code

use std::collections::VecDeque;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::task::Poll;

use futures::{Stream, pin_mut};
use tokio::sync::Notify;

use crate::task::{Task, TaskContext};

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

/// Scoped temporary directory; deletes the directory when dropped.
#[derive(Debug)]
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Creates a new temporary directory under the system temp dir.
    pub fn new() -> Self {
        let path = unique_temp_dir();
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    /// Returns the temporary directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Default for TempDir {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn unique_temp_dir() -> PathBuf {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("nagai-test-{}-{n}", std::process::id()))
}

/// Returns a temporary working directory for use in tests.
pub fn cwd() -> TempDir {
    TempDir::new()
}
