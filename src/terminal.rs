use std::io::Write;

use crossterm::event::Event;
use futures::Stream;

use crate::error::AnyResult;

#[cfg(not(test))]
pub use self::real::CrosstermTerminal as DefaultTerminal;
#[cfg(test)]
pub use self::mock::TestTerminal as DefaultTerminal;

/// A wrapper around the terminal API that can be used to simulate a terminal
/// in tests.
pub trait Terminal {
    /// Returns a writeable output destination.
    fn stdout(&mut self) -> &mut impl Write;

    /// Returns an event stream.
    fn events(&mut self) -> &mut (impl Stream<Item = std::io::Result<Event>> + Unpin);

    /// Returns a pair `(width, height)` of the terminal, in columns and rows.
    fn size(&self) -> AnyResult<(u16, u16)>;

    /// Enables raw mode on the underlying terminal.
    fn enable_raw_mode(&self) -> AnyResult<()>;

    /// Disables raw mode on the underlying terminal.
    fn disable_raw_mode(&self) -> AnyResult<()>;
}

#[cfg(not(test))]
mod real {
    use crossterm::event::{Event, EventStream};
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size};

    use super::*;

    /// Terminal implementation using crossterm.
    #[derive(Debug)]
    pub struct CrosstermTerminal {
        stdout: std::io::Stdout,
        events: EventStream,
    }

    impl Default for CrosstermTerminal {
        fn default() -> Self {
            Self {
                stdout: std::io::stdout(),
                events: EventStream::new(),
            }
        }
    }

    impl CrosstermTerminal {
        /// Creates a new `CrosstermTerminal`.
        pub fn new() -> Self {
            Self::default()
        }
    }

    impl Terminal for CrosstermTerminal {
        fn stdout(&mut self) -> &mut impl Write {
            &mut self.stdout
        }

        fn events(&mut self) -> &mut (impl Stream<Item = std::io::Result<Event>> + Unpin) {
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
pub mod mock {
    use std::io::Write;

    use crossterm::event::Event;
    use futures::Stream;

    use crate::error::AnyResult;
    use crate::testing::QueueStream;
    use super::Terminal;

    /// Terminal implementation for testing.
    #[derive(Debug)]
    pub struct TestTerminal {
        pub stdout: Vec<u8>,
        pub events: QueueStream<std::io::Result<Event>>,
        pub size: (u16, u16),
    }

    impl Default for TestTerminal {
        fn default() -> Self {
            Self {
                stdout: Vec::new(),
                events: QueueStream::default(),
                size: (80, 24),
            }
        }
    }

    impl TestTerminal {
        /// Creates a new `TestTerminal` with default 80x24 size.
        pub fn new() -> Self {
            Self::default()
        }

        /// Creates a new `TestTerminal` with the specified size.
        pub fn with_size(width: u16, height: u16) -> Self {
            Self {
                stdout: Vec::new(),
                events: QueueStream::default(),
                size: (width, height),
            }
        }
    }

    impl Terminal for TestTerminal {
        fn stdout(&mut self) -> &mut impl Write {
            &mut self.stdout
        }

        fn events(&mut self) -> &mut (impl Stream<Item = std::io::Result<Event>> + Unpin) {
            &mut self.events
        }

        fn size(&self) -> AnyResult<(u16, u16)> {
            Ok(self.size)
        }

        fn enable_raw_mode(&self) -> AnyResult<()> {
            Ok(())
        }

        fn disable_raw_mode(&self) -> AnyResult<()> {
            Ok(())
        }
    }
}
