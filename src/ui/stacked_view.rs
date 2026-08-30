use crossterm::event::{Event, KeyCode, KeyEvent};
use serde_json::{Value, json};

use crate::app::AppEvent;
use crate::query::{DataQuery, QueryError, QueryField, ToJson};
use crate::session::Item;
use crate::ui::Component;
use crate::ui::canvas::Canvas;
use crate::ui::command_editor::CommandEditor;
use crate::ui::history_view;
use crate::ui::history_view::HistoryView;
use crate::ui::style::Theme;

/// Which child of `StackedView` currently receives keyboard input.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum FocusState {
    History,
    #[default]
    CommandEditor,
}

/// Exposes the focus state as a string: `"history"` or `"command_editor"`.
impl ToJson for FocusState {
    fn to_json(self) -> Value {
        match self {
            FocusState::History => json!("history"),
            FocusState::CommandEditor => json!("command_editor"),
        }
    }
}

/// Stacks components vertically. The command editor is anchored to the bottom
/// and grows upward; the history fills the remaining space. Input events are
/// routed to whichever child is currently focused; Tab toggles focus between
/// the two.
#[derive(Debug)]
pub struct StackedView {
    width: usize,
    height: usize,
    history: HistoryView,
    input: CommandEditor,
    focus_state: FocusState,
}

impl StackedView {
    pub fn new(
        width: usize,
        height: usize,
        input_max_height: usize,
        theme: &'static Theme,
    ) -> Self {
        let mut this = Self {
            width,
            height,
            history: HistoryView::new(width, 0, theme),
            input: CommandEditor::new(width, input_max_height, theme),
            focus_state: FocusState::default(),
        };
        this.resize(); // Compute history height
        this.focus_child();
        this
    }

    pub fn input_mut(&mut self) -> &mut CommandEditor {
        &mut self.input
    }

    pub fn history_mut(&mut self) -> &mut HistoryView {
        &mut self.history
    }

    /// Returns the currently focused child.
    pub fn focus_state(&self) -> FocusState {
        self.focus_state
    }

    /// Recomputes the history region's height after the input box changes size.
    pub fn resize(&mut self) {
        // Reserve one row of padding between the history and the input box.
        let history_height = self
            .height
            .saturating_sub(self.input.height())
            .saturating_sub(1);
        if self.history.max_height() != history_height {
            self.history.set_height(history_height);
        }
    }

    /// Toggles focus between the two children.
    fn toggle_focus(&mut self) {
        self.focus_state = match self.focus_state {
            FocusState::History => FocusState::CommandEditor,
            FocusState::CommandEditor => FocusState::History,
        };
    }

    /// Sets the focus state on both children to match `self.focus_state`.
    fn focus_child(&mut self) {
        match self.focus_state {
            FocusState::History => {
                self.history.set_focus(true);
                self.input.set_focus(false);
            }
            FocusState::CommandEditor => {
                self.input.set_focus(true);
                self.history.set_focus(false);
            }
        }
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

impl<'a> TryFrom<Update<'a>> for history_view::Update<'a> {
    type Error = ();

    fn try_from(update: Update<'a>) -> Result<Self, Self::Error> {
        match update {
            Update::ItemCreated { item } => Ok(history_view::Update::ItemCreated { item }),
            Update::ItemUpdated { item } => Ok(history_view::Update::ItemUpdated { item }),
            Update::HelpMessage(content) => Ok(history_view::Update::HelpMessage(content)),
            Update::ErrorMessage(content) => Ok(history_view::Update::ErrorMessage(content)),
            Update::CommandPrompt(content) => Ok(history_view::Update::CommandPrompt(content)),
            Update::CommandOutput(content) => Ok(history_view::Update::CommandOutput(content)),
        }
    }
}

impl Component for StackedView {
    type Update<'a> = Update<'a>;
    type Event = Option<AppEvent>;

    fn draw(&self, canvas: Canvas) {
        let empty_rows = self.height - self.history.height() - self.input.height() - 1;
        for row in canvas.iter_mut().take(empty_rows) {
            row.pad(self.width);
        }
        self.history
            .draw(&mut canvas[empty_rows..empty_rows + self.history.height()]);
        canvas[empty_rows + self.history.height()].pad(self.width);
        self.input
            .draw(&mut canvas[self.height() - self.input.height()..self.height()]);
    }

    fn set_width(&mut self, width: usize) {
        self.width = width;
        self.history.set_width(width);
        self.input.set_width(width);
        self.resize();
    }

    fn set_height(&mut self, height: usize) {
        self.height = height;
        self.resize();
    }

    fn set_focus(&mut self, focused: bool) {
        if focused {
            self.focus_child();
        } else {
            self.history.set_focus(false);
            self.input.set_focus(false);
        }
    }

    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }

    fn cursor(&self) -> Option<(usize, usize)> {
        // The cursor only appears when the command editor (input box) is
        // focused; it is hidden while navigating the history.
        match self.focus_state {
            FocusState::CommandEditor => {
                let (row, col) = self.input.cursor()?;
                let row = self.height.saturating_sub(self.input.height()) + row;
                Some((row, col))
            }
            FocusState::History => self.history.cursor(),
        }
    }

    fn handle_input(&mut self, event: Event) -> Self::Event {
        let out = match event {
            // Tab switches focus
            Event::Key(KeyEvent {
                code: KeyCode::Tab, ..
            }) => {
                self.toggle_focus();
                self.focus_child();
                None
            }
            // Input goes to focused element
            e => match self.focus_state {
                FocusState::History => {
                    self.history.handle_input(e);
                    None
                }
                FocusState::CommandEditor => self.input.handle_input(e),
            },
        };
        self.resize();
        out
    }

    fn handle_update<'a>(&mut self, update: Self::Update<'a>) {
        if let Ok(child_update) = history_view::Update::try_from(update.clone()) {
            self.history.handle_update(child_update);
        }
        self.input.handle_update(());
        self.resize();
    }
}

impl DataQuery for StackedView {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "width": self.query("/width")?,
                "height": self.query("/height")?,
                "focus_state": self.query("/focus_state")?,
                "history": self.query("/history")?,
                "input": self.query("/input")?,
            }))),
            "width" => Ok(QueryField::Value(json!(self.width))),
            "height" => Ok(QueryField::Value(json!(self.height))),
            "focus_state" => Ok(QueryField::Value(self.focus_state.to_json())),
            "history" => Ok(QueryField::DataQuery(&self.history)),
            "input" => Ok(QueryField::DataQuery(&self.input)),
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
        let stacked = StackedView::new(80, 24, 8, &THEME_DARK);
        let expected = json!({
            "width": 80,
            "height": 24,
            "focus_state": "command_editor",
            "history": stacked.history.query("/").unwrap(),
            "input": stacked.input.query("/").unwrap(),
        });
        assert_eq!(stacked.query("/").unwrap(), expected);
        assert_eq!(stacked.query("/width").unwrap(), json!(80));
        assert_eq!(stacked.query("/height").unwrap(), json!(24));
        assert_eq!(
            stacked.query("/focus_state").unwrap(),
            json!("command_editor")
        );
        assert_eq!(
            stacked.query("/history").unwrap(),
            stacked.history.query("/").unwrap()
        );
        assert_eq!(
            stacked.query("/input").unwrap(),
            stacked.input.query("/").unwrap()
        );
    }
}
