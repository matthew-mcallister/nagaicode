use std::fmt;

use crossterm::Command;
use crossterm::event::Event;

use crate::ui::history::{History, HistoryItemContent, HistoryRowRef};
use crate::ui::scroll_bar::{ScrollBar, ScrollBarRow};
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

impl Component for HistoryView {
    type Row<'a> = HistoryViewRow<'a> where Self: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where Self: 'a;
    type InEvent = Event;
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
        self.history.handle_event(event);
        self.sync_scroll_bar();
    }
}
