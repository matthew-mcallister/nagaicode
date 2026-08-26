// FIXME: scroll bar should be hidden when history fits entirely onto screen

use crossterm::event::Event;
use serde_json::json;

use crate::query::{DataQuery, QueryError, QueryField};
use crate::session::Item;
use crate::ui::Component;
use crate::ui::canvas::Canvas;
use crate::ui::history::{self, History};
use crate::ui::scroll_bar::ScrollBar;
use crate::ui::style::Theme;

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

    pub fn max_height(&self) -> usize {
        self.history.max_height()
    }

    /// Syncs the scroll bar with the current state of the history.
    fn sync_scroll_bar(&mut self) {
        let history = &self.history;
        self.scroll_bar.set_num_rows(history.num_rows());
        self.scroll_bar
            .set_viewport(history.viewport_top_pos(), history.viewport_bottom_pos());
        self.scroll_bar.set_height(history.height());
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Update<'a> {
    ItemCreated { item: &'a Item },
    ItemUpdated { item: &'a Item },
    HelpMessage(&'a str),
    ErrorMessage(&'a str),
    CommandPrompt(&'a str),
    CommandOutput(&'a str),
}

impl<'a> TryFrom<Update<'a>> for history::Update<'a> {
    type Error = ();

    fn try_from(update: Update<'a>) -> Result<Self, Self::Error> {
        match update {
            Update::ItemCreated { item } => Ok(history::Update::ItemCreated { item }),
            Update::ItemUpdated { item } => Ok(history::Update::ItemUpdated { item }),
            Update::HelpMessage(content) => Ok(history::Update::HelpMessage(content)),
            Update::ErrorMessage(content) => Ok(history::Update::ErrorMessage(content)),
            Update::CommandPrompt(content) => Ok(history::Update::CommandPrompt(content)),
            Update::CommandOutput(content) => Ok(history::Update::CommandOutput(content)),
        }
    }
}

impl Component for HistoryView {
    type Update<'a> = Update<'a>;
    type Event = ();

    fn draw(&self, canvas: Canvas) {
        self.history.draw(&mut *canvas);
        self.scroll_bar.draw(canvas);
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

    fn handle_input(&mut self, event: Event) -> Self::Event {
        self.history.handle_input(event);
        self.sync_scroll_bar();
    }

    fn handle_update<'a>(&mut self, update: Self::Update<'a>) {
        if let Ok(child_update) = history::Update::try_from(update) {
            self.history.handle_update(child_update);
        }
        self.scroll_bar.handle_update(());
        self.sync_scroll_bar();
    }
}

/// Exposed fields:
/// - history: History
/// - scroll_bar: ScrollBar
impl DataQuery for HistoryView {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "history": self.history.query("/")?,
                "scroll_bar": self.scroll_bar.query("/")?,
            }))),
            "history" => Ok(QueryField::DataQuery(&self.history)),
            "scroll_bar" => Ok(QueryField::DataQuery(&self.scroll_bar)),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::style::THEME_DARK;
    use serde_json::json;

    #[test]
    fn test_query() {
        let view = HistoryView::new(20, 5, &THEME_DARK);
        let expected = json!({
            "history": view.history.query("/").unwrap(),
            "scroll_bar": view.scroll_bar.query("/").unwrap(),
        });
        assert_eq!(view.query("/").unwrap(), expected);
        assert_eq!(
            view.query("/history").unwrap(),
            view.history.query("/").unwrap()
        );
        assert_eq!(
            view.query("/scroll_bar").unwrap(),
            view.scroll_bar.query("/").unwrap()
        );
    }
}
