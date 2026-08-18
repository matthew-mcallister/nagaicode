use std::fmt;

use crossterm::Command;
use crossterm::event::Event;

use crate::session::Content;
use crate::ui::history::{self, History, HistoryItemContent, HistoryRowRef};
use crate::ui::scroll_bar::{self, ScrollBar, ScrollBarRow};
use crate::ui::style::Theme;
use crate::ui::Component;

/// A single drawable row of the history view. The scroll bar is rendered to
/// the right of the history.
#[derive(Debug)]
pub struct HistoryViewRow<'a> {
    history: HistoryRowRef<'a>,
    bar: ScrollBarRow<'a>,
}

impl Command for HistoryViewRow<'_> {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        self.history.write_ansi(f)?;
        self.bar.write_ansi(f)
    }
}

/// Chat history view, wrapper around History
#[derive(Debug)]
pub struct HistoryView {
    history: History,
    scroll_bar: ScrollBar,
}

impl HistoryView {
    pub fn new(width: usize, max_height: usize, theme: &'static Theme) -> Self {
        let mut this = Self {
            // Reserve one column for the scroll bar
            history: History::new(width.saturating_sub(1), max_height, theme),
            scroll_bar: ScrollBar::new(theme),
        };
        this.scroll_bar.set_width(1);
        this.sync_scroll_bar();
        this
    }

    pub fn history_mut(&mut self) -> &mut History {
        &mut self.history
    }

    pub fn add_item(&mut self, content: HistoryItemContent) {
        self.history.add_item(content);
        self.sync_scroll_bar();
    }

    pub fn max_height(&self) -> usize {
        self.history.max_height()
    }

    /// Syncs the scroll bar with the current state of the history.
    fn sync_scroll_bar(&mut self) {
        let history = &self.history;
        self.scroll_bar.set_num_rows(history.num_rows());
        self.scroll_bar.set_viewport(history.viewport_top_pos(), history.viewport_bottom_pos());
        self.scroll_bar.set_height(history.height());
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InEvent {
    Input(Event),
    ContentCreated(Content),
    ContentUpdated(Content),
}

impl From<Event> for InEvent {
    fn from(event: Event) -> Self {
        InEvent::Input(event)
    }
}

impl TryFrom<InEvent> for history::InEvent {
    type Error = ();

    fn try_from(event: InEvent) -> Result<Self, Self::Error> {
        match event {
            InEvent::Input(event) => Ok(event.into()),
            InEvent::ContentCreated(content) => Ok(history::InEvent::ContentCreated(content)),
            InEvent::ContentUpdated(content) => Ok(history::InEvent::ContentUpdated(content)),
        }
    }
}

impl TryFrom<InEvent> for scroll_bar::InEvent {
    type Error = ();

    fn try_from(_event: InEvent) -> Result<Self, Self::Error> {
        Err(())
    }
}

impl Component for HistoryView {
    type Row<'a> = HistoryViewRow<'a> where Self: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where Self: 'a;
    type InEvent = InEvent;
    type OutEvent = ();

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        Box::new(
            self.history
                .drawable_rows()
                .zip(self.scroll_bar.drawable_rows())
                .map(|(history, bar)| HistoryViewRow { history, bar }),
        )
    }

    fn set_width(&mut self, width: usize) {
        self.history.set_width(width.saturating_sub(1));
        self.scroll_bar.set_width(1);
        self.sync_scroll_bar();
    }

    fn set_height(&mut self, height: usize) {
        self.history.set_height(height);
        self.sync_scroll_bar();
    }

    fn set_focus(&mut self, focused: bool) {
        self.history.set_focus(focused);
        self.scroll_bar.set_focus(focused);
    }

    fn width(&self) -> usize {
        self.history.width() + 1
    }

    fn height(&self) -> usize {
        self.history.height()
    }

    fn cursor(&self) -> Option<(usize, usize)> {
        self.history.cursor()
    }

    fn handle_event(&mut self, event: Self::InEvent) -> Self::OutEvent {
        if let Ok(child_event) = history::InEvent::try_from(event.clone()) {
            self.history.handle_event(child_event);
        }
        if let Ok(child_event) = scroll_bar::InEvent::try_from(event) {
            self.scroll_bar.handle_event(child_event);
        }
        self.sync_scroll_bar();
    }
}
