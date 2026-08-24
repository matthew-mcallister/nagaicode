// TODO: select mode, disables all horizontal padding

use crate::query::{DataQuery, QueryError, QueryField};
use crate::ui::Component;
use crate::ui::canvas::Canvas;
use crate::ui::style::Color;

/// Adds padding around a UI component. Also styles the background.
#[derive(Debug)]
pub struct Padded<C> {
    pub h_padding: usize,
    pub v_padding: usize,
    pub background_color: Option<Color>,
    pub inner: C,
}

impl<C> Padded<C> {
    pub fn new(
        inner: C,
        h_padding: usize,
        v_padding: usize,
        background_color: Option<Color>,
    ) -> Self {
        Self {
            h_padding,
            v_padding,
            background_color,
            inner,
        }
    }

    pub fn inner(&self) -> &C {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut C {
        &mut self.inner
    }

    pub fn into_inner(self) -> C {
        self.inner
    }
}

impl<C: Component> Component for Padded<C> {
    type Update<'a> = C::Update<'a>;
    type Event = C::Event;

    fn draw(&self, canvas: Canvas) {
        if let Some(bg_color) = self.background_color {
            for row in canvas[..self.height()].iter_mut() {
                row.set_bg_color(bg_color);
            }
        }

        // Vertical pad
        for i in 0..self.v_padding {
            canvas[i].pad(self.width());
        }
        for i in self.height() - self.v_padding..self.height() {
            canvas[i].pad(self.width());
        }

        // Render child
        for i in self.v_padding..self.height() - self.v_padding {
            canvas[i].pad(self.h_padding);
        }
        self.inner.draw(&mut canvas[self.v_padding..self.height() - self.v_padding]);
        for i in self.v_padding..self.height() - self.v_padding {
            canvas[i].pad(self.h_padding);
        }
    }

    fn set_width(&mut self, width: usize) {
        self.inner.set_width(width.saturating_sub(2 * self.h_padding));
    }

    fn set_height(&mut self, height: usize) {
        self.inner.set_height(height.saturating_sub(2 * self.v_padding));
    }

    fn set_focus(&mut self, focused: bool) {
        self.inner.set_focus(focused);
    }

    fn width(&self) -> usize {
        self.inner.width() + 2 * self.h_padding
    }

    fn height(&self) -> usize {
        self.inner.height() + 2 * self.v_padding
    }

    fn cursor(&self) -> Option<(usize, usize)> {
        self.inner
            .cursor()
            .map(|(row, col)| (row + self.v_padding, col + self.h_padding))
    }

    fn handle_input(&mut self, event: crossterm::event::Event) -> Self::Event {
        self.inner.handle_input(event)
    }

    fn handle_update<'a>(&mut self, update: Self::Update<'a>) {
        self.inner.handle_update(update)
    }
}

/// Exposed fields:
/// - h_padding: number
/// - v_padding: number
/// - background_color: color | null
/// - inner: C
impl<C> DataQuery for Padded<C> {
    fn query_field<'a>(&'a self, _field: &str) -> Result<QueryField<'a>, QueryError> {
        todo!()
    }
}
