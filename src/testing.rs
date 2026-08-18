//! Testing common code

use std::collections::VecDeque;
use std::task::Poll;

use futures::{Stream, pin_mut};

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
