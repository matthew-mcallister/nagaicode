use crossterm::Command;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use crate::app::AppEvent;
use crate::ui::style::Theme;
use crate::ui::command_editor::{CommandEditor, CommandEditorRow};
use crate::ui::history::{History, HistoryRowRef};
use crate::ui::{write_spaces, Component};

/// Which child of `StackedView` currently receives keyboard input.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum FocusState {
    History,
    #[default]
    CommandEditor,
}

/// Stacks components vertically. The command editor is anchored to the bottom
/// and grows upward; the history fills the remaining space. Input events are
/// routed to whichever child is currently focused; Tab toggles focus between
/// the two. All command history navigation logic lives in `CommandEditor`.
#[derive(Debug)]
pub struct StackedView {
    width: usize,
    height: usize,
    history: History,
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
            history: History::new(width, 0, theme),
            input: CommandEditor::new(width, input_max_height, theme),
            focus_state: FocusState::default(),
        };
        this.resize();  // Compute history height
        this
    }

    pub fn input_mut(&mut self) -> &mut CommandEditor {
        &mut self.input
    }

    pub fn history_mut(&mut self) -> &mut History {
        &mut self.history
    }

    /// Returns the currently focused child.
    pub fn focus_state(&self) -> FocusState {
        self.focus_state
    }

    /// Recomputes the history region's height after the input box changes size.
    pub fn resize(&mut self) {
        let history_height = self.height.saturating_sub(self.input.height());
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
}

#[derive(Debug)]
pub enum StackedRow<'a> {
    Empty { width: usize },
    History(HistoryRowRef<'a>),
    Input(CommandEditorRow<'a>),
}

impl Command for StackedRow<'_> {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        match self {
            StackedRow::Empty { width } => write_spaces(f, *width),
            StackedRow::History(row) => row.write_ansi(f),
            StackedRow::Input(row) => row.write_ansi(f),
        }
    }
}

impl Component for StackedView {
    type Row<'a> = StackedRow<'a> where Self: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where Self: 'a;
    type EventReponse = Option<AppEvent>;

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        let empty_rows = self
            .height
            .saturating_sub(self.history.height())
            .saturating_sub(self.input.height());
        let width = self.width;
        let empty = (0..empty_rows).map(move |_| StackedRow::Empty { width });
        let history = self.history.drawable_rows().map(StackedRow::History);
        let input = self.input.drawable_rows().map(StackedRow::Input);
        Box::new(empty.chain(history).chain(input))
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

    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }

    fn cursor_pos(&self) -> (usize, usize) {
        // The cursor always remains in the command editor, regardless of which
        // child is focused.
        let (row, col) = self.input.cursor_pos();
        (self.height.saturating_sub(self.input.height()) + row, col)
    }

    fn handle_event(&mut self, event: Event) -> Self::EventReponse {
        // Tab (with no modifiers) toggles focus between children and is consumed
        // rather than forwarded.
        if let Event::Key(KeyEvent { code: KeyCode::Tab, modifiers: KeyModifiers::NONE, .. }) = event {
            self.toggle_focus();
            return None;
        }
        match self.focus_state {
            FocusState::History => {
                self.history.handle_event(event);
                None
            }
            FocusState::CommandEditor => self.input.handle_event(event),
        }
    }
}
