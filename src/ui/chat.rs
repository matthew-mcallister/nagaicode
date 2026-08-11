use std::io::Write;

use compact_str::CompactString;
use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
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

    pub fn handle_key(&mut self, key: KeyEvent) -> Option<AppEvent> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let input = self.stacked.inner_mut().input_mut();
        let mut response = None;
        match (key.code, ctrl, shift, alt) {
            // Ctrl + char
            (KeyCode::Char('a'), true, _, _) => input.go_to_line_start(),
            (KeyCode::Char('e'), true, _, _) => input.go_to_line_end(),
            (KeyCode::Char('u'), true, _, _) => input.delete_to_line_start(),
            (KeyCode::Char('k'), true, _, _) => input.delete_to_line_end(),
            (KeyCode::Char('w'), true, _, _) => input.delete_prev_word(),
            (KeyCode::Char('y'), true, _, _) => input.paste_buffer(),
            // Alt + char
            (KeyCode::Char('f'), _, _, true) => input.go_to_word_end(),
            (KeyCode::Char('b'), _, _, true) => input.go_to_prev_word_start(),
            // Other combinations
            | (KeyCode::Char('j'), true, _, _)
            | (KeyCode::Char('j'), _, _, true)
            | (KeyCode::Enter, true, _, _)
            | (KeyCode::Enter, _, true, _)
            | (KeyCode::Enter, _, _, true) => input.paste("\n"),
            // Ignoring modifiers
            (KeyCode::Char(c), _, _, _) => {
                let mut s = CompactString::with_capacity(1);
                s.push(c);
                input.paste(&s);
            }
            (KeyCode::Enter, _, _, _) => {
                let mut text = input.get_text();
                input.set_text("");
                if text.ends_with('\n') { text.pop(); }
                response = Some(AppEvent::Command(text));
            }
            // XXX: Maybe should expand to spaces when input via keyboard
            (KeyCode::Tab, _, _, _) => input.paste("\t"),
            (KeyCode::Backspace, _, _, _) => input.backspace(),
            (KeyCode::Delete, _, _, _) => input.delete(),
            (KeyCode::Left, _, _, _) => input.move_left(),
            (KeyCode::Right, _, _, _) => input.move_right(),
            (KeyCode::Up, _, _, _) => input.move_up(1),
            (KeyCode::Down, _, _, _) => input.move_down(1),
            (KeyCode::PageUp, _, _, _) => input.move_up(input.max_height()),
            (KeyCode::PageDown, _, _, _) => input.move_down(input.max_height()),
            _ => {},
        };
        self.stacked.inner_mut().resize();
        response
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
            Event::Key(key) => {
                self.handle_key(key)
            }
            Event::Resize(w, h) => {
                self.resize(w, h);
                None
            }
            _ => None,
        }
    }

    pub fn add_item(&mut self, content: HistoryItemContent) {
        self.stacked.inner_mut().history_mut().add_item(content);
    }
}
