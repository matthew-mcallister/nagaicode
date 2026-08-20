use crossterm::Command;
use crossterm::event::{Event, KeyCode, KeyEvent};

use crate::app::AppEvent;
use crate::session::{Content, Item};
use crate::ui::canvas::Canvas;
use crate::ui::style::Theme;
use crate::ui::command_editor::{CommandEditor, CommandEditorRow};
use crate::ui::history_view::{self, HistoryView, HistoryViewRow};
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
        this.resize();  // Compute history height
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

#[derive(Debug)]
pub enum StackedRow<'a> {
    Empty { width: usize },
    History(HistoryViewRow<'a>),
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Update<'a> {
    ContentCreated { item: &'a Item, content: &'a Content },
    ContentUpdated { item: &'a Item, content: &'a Content },
    HelpMessage(&'a str),
    ErrorMessage(&'a str),
}

impl<'a> TryFrom<Update<'a>> for history_view::Update<'a> {
    type Error = ();

    fn try_from(update: Update<'a>) -> Result<Self, Self::Error> {
        match update {
            Update::ContentCreated { item, content } => Ok(history_view::Update::ContentCreated { item, content }),
            Update::ContentUpdated { item, content } => Ok(history_view::Update::ContentUpdated { item, content }),
            Update::HelpMessage(content) => Ok(history_view::Update::HelpMessage(content)),
            Update::ErrorMessage(content) => Ok(history_view::Update::ErrorMessage(content)),
        }
    }
}

impl Component for StackedView {
    type Row<'a> = StackedRow<'a> where Self: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where Self: 'a;
    type Update<'a> = Update<'a>;
    type Event = Option<AppEvent>;

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        let empty_rows = self
            .height
            .saturating_sub(self.history.height())
            .saturating_sub(self.input.height())
            .saturating_sub(1);
        let width = self.width;
        let empty = (0..empty_rows).map(move |_| StackedRow::Empty { width });
        let spacer = std::iter::once(StackedRow::Empty { width });
        let history = self.history.drawable_rows().map(StackedRow::History);
        let input = self.input.drawable_rows().map(StackedRow::Input);
        Box::new(empty.chain(history).chain(spacer).chain(input))
    }

    fn draw(&self, canvas: &mut Canvas) {
        let empty_rows = self.height - self.history.height() - self.input.height() - 1;
        for i in 0..empty_rows {
            canvas.rows[i].pad(self.width);
        }
        self.history.draw(&mut Canvas {
            rows: &mut canvas.rows[empty_rows..empty_rows + self.history.height()],
        });
        canvas.rows[empty_rows + self.history.height()].pad(self.width);
        self.input.draw(&mut Canvas {
            rows: &mut canvas.rows[self.height() - self.input.height()..self.height()],
        });
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
            },
            FocusState::History => self.history.cursor(),
        }
    }

    fn handle_input(&mut self, event: Event) -> Self::Event {
        let out = match event {
            // Tab switches focus
            Event::Key(KeyEvent { code: KeyCode::Tab, .. }) => {
                self.toggle_focus();
                self.focus_child();
                None
            }
            // Input goes to focused element
            e => match self.focus_state {
                FocusState::History => {
                    self.history.handle_input(e);
                    None
                },
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
