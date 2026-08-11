use std::io::Write;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::Event;
use crossterm::queue;
use crossterm::style::{ResetColor, SetBackgroundColor, SetForegroundColor};
use dedent::dedent;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::ui::history::HistoryItemContent;
use crate::ui::style::Theme;
use crate::ui::padded::Padded;
use crate::ui::stacked_view::StackedView;
use crate::ui::Component;

const TEXT_INPUT_MAX_HEIGHT: u16 = 24;

#[derive(Debug)]
pub struct Chat {
    theme: &'static Theme,
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
            theme,
            stacked: Padded::new(stacked, 2, 1, Some(theme.bg_base)),
        }
    }

    pub fn resize(&mut self, w: u16, h: u16) {
        self.stacked.set_width(w as usize);
        self.stacked.set_height(h as usize);
    }

    // TODO: cap redraw frequency
    pub fn draw(&self, stdout: &mut impl Write) -> AnyResult<()> {
        let text_style = self.theme.text_base;
        let bg = self.theme.bg_base;
        queue!(stdout,
            Hide,
            SetForegroundColor(text_style.fg_color),
            SetBackgroundColor(bg),
        )?;
        for (y, row) in self.stacked.drawable_rows().enumerate() {
            queue!(stdout, MoveTo(0, y as u16), row)?;
        }
        let (row, col) = self.stacked.cursor_pos();
        queue!(stdout, ResetColor, MoveTo(col as u16, row as u16), Show)?;
        stdout.flush()?;
        Ok(())
    }

    pub fn handle_event(&mut self, event: Event) -> Option<AppEvent> {
        match event {
            Event::Resize(w, h) => {
                self.resize(w, h);
                None
            }
            _ => {
                let response = self.stacked.handle_event(event);
                // The input box may have grown or shrunk, so recompute the
                // history region's height.
                self.stacked.inner_mut().resize();
                response
            }
        }
    }

    pub fn add_item(&mut self, content: HistoryItemContent) {
        self.stacked.inner_mut().history_mut().add_item(content);
    }
}
