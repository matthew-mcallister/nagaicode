//! Stuff that didn't fit anywhere else

use std::pin::Pin;
use std::task;

use futures::Stream;

struct PollOnce<'a, S>(Pin<&'a mut S>);

impl<'a, S, T> Future for PollOnce<'a, S>
where
    S: Stream<Item = T> + 'a,
{
    type Output = task::Poll<Option<T>>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
    ) -> task::Poll<Self::Output> {
        task::Poll::Ready(match self.0.as_mut().poll_next(cx) {
            task::Poll::Ready(out) => task::Poll::Ready(out),
            task::Poll::Pending => task::Poll::Pending,
        })
    }
}

/// Returns a future which polls a stream for an item and immediately
/// yields the result, no waiting.
pub(crate) fn poll<'a, T>(stream: Pin<&'a mut impl Stream<Item = T>>) ->
    impl Future<Output = task::Poll<Option<T>>> + 'a
{
    PollOnce(stream)
}
