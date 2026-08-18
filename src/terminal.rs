use std::io::Write;

use crossterm::event::Event;
use futures::Stream;

use crate::error::AnyResult;

pub use detail::DefaultTerminal;

/// A wrapper around the terminal API that can be used to simulate a terminal
/// in tests.
pub trait Terminal {
    /// Returns a writeable output destination.
    fn stdout(&mut self) -> &mut impl Write;

    /// Returns an event stream.
    fn events(&mut self) -> &mut impl Stream<Item = std::io::Result<Event>>;

    /// Returns a pair `(width, height)` of the terminal, in columns and rows
    fn size(&self) -> AnyResult<(u16, u16)>;

    /// Enables raw mode on the underlying terminal.
    fn enable_raw_mode(&self) -> AnyResult<()>;

    /// Disables raw mode on the underlying terminal.
    fn disable_raw_mode(&self) -> AnyResult<()>;
}

#[cfg(not(test))]
mod detail {
    use std::io::Write;

    use crossterm::event::{Event, EventStream};
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size};
    use futures::Stream;

    use super::Terminal;
    use crate::error::AnyResult;

    /// Terminal implementation using crossterm
    #[derive(Debug)]
    pub struct DefaultTerminal {
        stdout: std::io::Stdout,
        events: EventStream,
    }

    impl Default for DefaultTerminal {
        fn default() -> Self {
            Self {
                stdout: std::io::stdout(),
                events: EventStream::new(),
            }
        }
    }

    impl DefaultTerminal {
        pub fn new() -> Self {
            Self::default()
        }
    }

    impl Terminal for DefaultTerminal {
        fn stdout(&mut self) -> &mut impl Write {
            &mut self.stdout
        }

        fn events(&mut self) -> &mut impl Stream<Item = std::io::Result<Event>> {
            &mut self.events
        }

        fn size(&self) -> AnyResult<(u16, u16)> {
            Ok(size()?)
        }

        fn enable_raw_mode(&self) -> AnyResult<()> {
            Ok(enable_raw_mode()?)
        }

        fn disable_raw_mode(&self) -> AnyResult<()> {
            Ok(disable_raw_mode()?)
        }
    }
}

#[cfg(test)]
mod detail {
    use std::io::Write;
    use std::task::Poll;

    use crossterm::event::Event;
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size};
    use futures::{Stream, pin_mut};

    use super::Terminal;
    use crate::error::AnyResult;

    /// Simulates an async stream by yielding from a vector.
    #[derive(Clone, Debug)]
    pub struct VecStream<E>(pub Vec<E>);

    impl<E> Default for VecStream<E> {
        fn default() -> Self {
            Self(Default::default())
        }
    }

    impl<E> Unpin for VecStream<E> {}

    impl<E> Stream for VecStream<E> {
        type Item = E;

        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> Poll<Option<Self::Item>> {
            let this = self;
            pin_mut!(this);
            Poll::Ready(this.0.pop())
        }
    }

    /// Dummy terminal implementation
    #[derive(Debug)]
    pub struct DefaultTerminal {
        pub stdout: Vec<u8>,
        pub events: VecStream<std::io::Result<Event>>,
    }

    impl Default for DefaultTerminal {
        fn default() -> Self {
            Self {
                stdout: Vec::new(),
                events: VecStream::default(),
            }
        }
    }

    impl DefaultTerminal {
        pub fn new() -> Self {
            Self::default()
        }
    }

    impl Terminal for DefaultTerminal {
        fn stdout(&mut self) -> &mut impl Write {
            &mut self.stdout
        }

        fn events(&mut self) -> &mut impl Stream<Item = std::io::Result<Event>> {
            &mut self.events
        }

        fn size(&self) -> AnyResult<(u16, u16)> {
            Ok(size()?)
        }

        fn enable_raw_mode(&self) -> AnyResult<()> {
            Ok(enable_raw_mode()?)
        }

        fn disable_raw_mode(&self) -> AnyResult<()> {
            Ok(disable_raw_mode()?)
        }
    }
}
