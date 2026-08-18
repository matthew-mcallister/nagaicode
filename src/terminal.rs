use std::io::Write;

use crossterm::event::Event;
use futures::Stream;

use crate::error::AnyResult;

pub use detail::CrosstermTerminal as DefaultTerminal;

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

mod detail {
    use std::io::Write;

    use crossterm::event::{Event, EventStream};
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size};
    use futures::Stream;

    use super::Terminal;
    use crate::error::AnyResult;

    /// Terminal implementation using crossterm
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
        pub fn new() -> Self {
            Self::default()
        }
    }

    impl Terminal for CrosstermTerminal {
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
