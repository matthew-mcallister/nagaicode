use crossterm::event::Event;
use dedent::dedent;
use serde_json::json;

use crate::app::AppEvent;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::session::Item;
use crate::ui::Component;
use crate::ui::canvas::Canvas;
use crate::ui::padded::Padded;
use crate::ui::stacked_view::{self, StackedView};
use crate::ui::style::Theme;

const TEXT_INPUT_MAX_HEIGHT: u16 = 24;

#[derive(Debug)]
pub struct Chat {
    stacked: Padded<StackedView>,
}

impl Chat {
    pub fn new(w: u16, h: u16, theme: &'static Theme) -> Self {
        // Minimum dimensions are 80x24. If the terminal is smaller the UI will
        // just overflow the screen. This helps avoid crashes or bizarre bugs
        // caused by pathologically tiny terminals.
        let w = w.max(80);
        let h = h.max(24);

        let mut stacked = StackedView::new(
            w as usize - 4,
            h as usize - 2,
            TEXT_INPUT_MAX_HEIGHT.min(h.saturating_sub(2)) as usize,
            theme,
        );
        let help = dedent!(
            "
            Welcome to NagaiCode!

            Type /help for a list of commands."
        );
        stacked.handle_update(stacked_view::Update::HelpMessage(&help));

        Self {
            stacked: Padded::new(stacked, 2, 1, Some(theme.bg_base)),
        }
    }

    pub fn resize(&mut self, w: u16, h: u16) {
        self.stacked.set_width(w as usize);
        self.stacked.set_height(h as usize);
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

impl<'a> TryFrom<Update<'a>> for stacked_view::Update<'a> {
    type Error = ();

    fn try_from(update: Update<'a>) -> Result<Self, Self::Error> {
        match update {
            Update::ItemCreated { item } => Ok(stacked_view::Update::ItemCreated { item }),
            Update::ItemUpdated { item } => Ok(stacked_view::Update::ItemUpdated { item }),
            Update::HelpMessage(content) => Ok(stacked_view::Update::HelpMessage(content)),
            Update::ErrorMessage(content) => Ok(stacked_view::Update::ErrorMessage(content)),
            Update::CommandPrompt(content) => Ok(stacked_view::Update::CommandPrompt(content)),
            Update::CommandOutput(content) => Ok(stacked_view::Update::CommandOutput(content)),
        }
    }
}

impl Component for Chat {
    type Update<'a> = Update<'a>;
    type Event = Option<AppEvent>;

    fn draw(&self, canvas: Canvas) {
        self.stacked.draw(canvas);
    }

    fn width(&self) -> usize {
        self.stacked.width()
    }

    fn height(&self) -> usize {
        self.stacked.height()
    }

    fn cursor(&self) -> Option<(usize, usize)> {
        self.stacked.cursor()
    }

    fn set_width(&mut self, width: usize) {
        self.stacked.set_width(width);
    }

    fn set_height(&mut self, height: usize) {
        self.stacked.set_height(height);
    }

    fn set_focus(&mut self, focused: bool) {
        self.stacked.set_focus(focused);
    }

    fn handle_input(&mut self, event: Event) -> Self::Event {
        if let Event::Resize(w, h) = event {
            self.resize(w, h);
            None
        } else {
            self.stacked.handle_input(event)
        }
    }

    fn handle_update<'a>(&mut self, update: Self::Update<'a>) {
        if let Ok(child_update) = stacked_view::Update::try_from(update) {
            self.stacked.handle_update(child_update);
        }
    }
}

/// Exposed fields:
/// - stacked: Padded<StackedView>
impl DataQuery for Chat {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "stacked": self.stacked.query("/")?,
            }))),
            "stacked" => Ok(QueryField::DataQuery(&self.stacked)),
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
        let chat = Chat::new(80, 24, &THEME_DARK);
        let expected = json!({
            "stacked": chat.stacked.query("/").unwrap(),
        });
        assert_eq!(chat.query("/").unwrap(), expected);
        assert_eq!(
            chat.query("/stacked").unwrap(),
            chat.stacked.query("/").unwrap()
        );
    }
}
