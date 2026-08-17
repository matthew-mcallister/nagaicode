use crossterm::event::Event;
use dedent::dedent;
use derive_more::From;

use crate::app::AppEvent;
use crate::ui::history::HistoryItemContent;
use crate::ui::padded::{Padded, PaddedRow};
use crate::ui::stacked_view::{self, StackedRow, StackedView};
use crate::ui::style::Theme;
use crate::ui::Component;

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
        let w = w.max(20);
        let h = h.max(16);

        let mut stacked = StackedView::new(
            w as usize - 4,
            h as usize - 2,
            TEXT_INPUT_MAX_HEIGHT.min(h.saturating_sub(2)) as usize,
            theme,
        );
        stacked.history_mut().add_item(HistoryItemContent::Help(dedent!("
            Welcome to NagaiCode!

            Type /help for a list of commands."
        ).into()));

        Self {
            stacked: Padded::new(stacked, 2, 1, Some(theme.bg_base)),
        }
    }

    pub fn resize(&mut self, w: u16, h: u16) {
        self.stacked.set_width(w as usize);
        self.stacked.set_height(h as usize);
    }

    pub fn add_item(&mut self, content: HistoryItemContent) {
        self.stacked.inner_mut().history_mut().add_item(content);
    }
}

#[derive(Clone, Debug, Eq, PartialEq, From)]
pub enum InEvent {
    Input(Event),
}

impl TryFrom<InEvent> for stacked_view::InEvent {
    type Error = ();

    fn try_from(event: InEvent) -> Result<Self, Self::Error> {
        match event {
            InEvent::Input(event) => Ok(event.into()),
        }
    }
}

impl Component for Chat {
    type Row<'a> = PaddedRow<StackedRow<'a>> where Self: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where Self: 'a;
    type InEvent = InEvent;
    type OutEvent = Option<AppEvent>;

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        self.stacked.drawable_rows()
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

    fn handle_event(&mut self, event: Self::InEvent) -> Self::OutEvent {
        let InEvent::Input(raw_event) = &event;
        if let Event::Resize(w, h) = *raw_event {
            self.resize(w, h);
            return None;
        }

        let response = if let Ok(child_event) = stacked_view::InEvent::try_from(event) {
            self.stacked.handle_event(child_event)
        } else {
            None
        };
        // The input box may have grown or shrunk, so recompute the
        // history region's height.
        self.stacked.inner_mut().resize();
        response
    }
}
