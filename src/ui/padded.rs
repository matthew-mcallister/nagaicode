// TODO: select mode, disables all horizontal padding

use serde_json::json;

use crate::query::{DataQuery, QueryError, QueryField, ToJson};
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
        for row in canvas.iter_mut().take(self.v_padding) {
            row.pad(self.width());
        }
        for row in canvas.iter_mut().skip(self.height() - self.v_padding) {
            row.pad(self.width());
        }

        // Render child
        for row in canvas
            .iter_mut()
            .skip(self.v_padding)
            .take(self.height() - 2 * self.v_padding)
        {
            row.pad(self.h_padding);
        }
        self.inner.draw(&mut canvas[self.v_padding..self.height() - self.v_padding]);
        for row in canvas
            .iter_mut()
            .skip(self.v_padding)
            .take(self.height() - 2 * self.v_padding)
        {
            row.pad(self.h_padding);
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
impl<C: DataQuery> DataQuery for Padded<C> {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "h_padding": self.query("/h_padding")?,
                "v_padding": self.query("/v_padding")?,
                "background_color": self.query("/background_color")?,
                "inner": self.query("/inner")?,
            }))),
            "h_padding" => Ok(QueryField::Value(json!(self.h_padding))),
            "v_padding" => Ok(QueryField::Value(json!(self.v_padding))),
            "background_color" => Ok(QueryField::Value(match self.background_color {
                Some(c) => c.to_json(),
                None => json!(null),
            })),
            "inner" => Ok(QueryField::DataQuery(&self.inner)),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::scroll_bar::ScrollBar;
    use crate::ui::style::THEME_DARK;
    use serde_json::json;

    #[test]
    fn test_query() {
        let padded = Padded::new(ScrollBar::new(&THEME_DARK), 2, 1, Some(Color::White));
        let expected = json!({
            "h_padding": 2,
            "v_padding": 1,
            "background_color": "white",
            "inner": padded.inner.query("/").unwrap(),
        });
        assert_eq!(padded.query("/").unwrap(), expected);
        assert_eq!(padded.query("/h_padding").unwrap(), json!(2));
        assert_eq!(padded.query("/v_padding").unwrap(), json!(1));
        assert_eq!(padded.query("/background_color").unwrap(), json!("white"));
        assert_eq!(padded.query("/inner").unwrap(), padded.inner.query("/").unwrap());
    }
}
