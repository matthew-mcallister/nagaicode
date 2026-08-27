//! Scroll bar indicator. Renders as a one-column-wide bar on the left side of
//! the component. The bar's position and height are derived from the size of
//! the scrolled content and the position of the viewport within it.
// TODO: Make scroll bar full width and switch to 1/8th row increment using
// box shadow drawing chars and reversed colors.
use crossterm::event::Event;
use serde_json::json;

use crate::query::{DataQuery, QueryError, QueryField};
use crate::ui::canvas::Canvas;
use crate::ui::style::Theme;
use crate::ui::Component;

#[derive(Debug)]
pub struct ScrollBar {
    theme: &'static Theme,
    /// Number of visible rows in the viewport
    height: usize,
    /// Width of the scroll bar in columns. The bar occupies only the first
    /// column; the remaining columns are padded with spaces.
    width: usize,
    /// Total number of rows in the scrolled content
    num_rows: usize,
    /// Row index of the first visible row
    top: usize,
    /// Row index of the last visible row
    bottom: usize,
    /// Whether the scrolled component is focused
    focused: bool,
}

impl ScrollBar {
    pub fn new(theme: &'static Theme) -> Self {
        Self {
            theme,
            height: 0,
            width: 0,
            num_rows: 0,
            top: 0,
            bottom: 0,
            focused: false,
        }
    }

    pub fn set_num_rows(&mut self, num_rows: usize) {
        self.num_rows = num_rows;
    }

    /// Updates the viewport. `top` and `bottom` are the row indices of the
    /// first and last visible rows, respectively.
    pub fn set_viewport(&mut self, top: usize, bottom: usize) {
        self.top = top;
        self.bottom = bottom;
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    /// Computes the scroll bar position as a half-open range of rows
    /// `[start, end)` within the viewport.
    fn scroll_bar_range(&self) -> (usize, usize) {
        if self.num_rows == 0 || self.height == 0 {
            return (0, 0);
        }

        let w = self.bottom - self.top + 1;
        let h = ((w * self.height) / self.num_rows).max(1);
        let r = self.height - h;

        // We are mapping [0, m] to [0, n]
        let (m, n) = (self.num_rows - w, r);
        let k = self.top;
        if r == 0 {
            (0, self.height)
        } else if m <= n {
            let st = (n * k) / m;
            (st, st + h)
        } else {
            // Properties: scroll bar only hits top when viewport is at first
            // row and only hits bottom when viewport is at last row.
            debug_assert!(k <= m);
            let b = (k > 0) as usize;
            let k = k.max(1);
            let p = b + ((k - 1) * (n - 1)) / (m - 1);
            debug_assert!(p <= n);
            (p, p + h)
        }
    }
}

impl Component for ScrollBar {
    type Update<'a> = ();
    type Event = ();

    fn draw(&self, canvas: Canvas) {
        if self.width == 0 {
            return;
        }
        let (start, end) = self.scroll_bar_range();
        for row in 0..self.height {
            let text_style = if (start..end).contains(&row) {
                if self.focused {
                    self.theme.text_scroll_bar_focused
                } else {
                    self.theme.text_scroll_bar_unfocused
                }
            } else {
                self.theme.text_scroll_bar_track
            };
            canvas[row].set_bg_color(self.theme.bg_base);
            canvas[row].set_text(text_style);
            canvas[row].push("▐", 1);
            canvas[row].pad(self.width - 1);
        }
    }

    fn set_width(&mut self, width: usize) {
        self.width = width;
    }

    fn set_height(&mut self, height: usize) {
        self.height = height;
    }

    fn set_focus(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn width(&self) -> usize {
        self.width
    }

    fn height(&self) -> usize {
        self.height
    }

    fn cursor(&self) -> Option<(usize, usize)> {
        None
    }

    fn handle_input(&mut self, _event: Event) -> Self::Event {
    }

    fn handle_update<'a>(&mut self, _update: Self::Update<'a>) {
    }
}

/// Exposed fields:
/// - height: number
/// - width: number
/// - num_rows: number
/// - top: number
/// - bottom: number
/// - focused: bool
impl DataQuery for ScrollBar {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "height": self.query("/height")?,
                "width": self.query("/width")?,
                "num_rows": self.query("/num_rows")?,
                "top": self.query("/top")?,
                "bottom": self.query("/bottom")?,
                "focused": self.query("/focused")?,
            }))),
            "height" => Ok(QueryField::Value(json!(self.height))),
            "width" => Ok(QueryField::Value(json!(self.width))),
            "num_rows" => Ok(QueryField::Value(json!(self.num_rows))),
            "top" => Ok(QueryField::Value(json!(self.top))),
            "bottom" => Ok(QueryField::Value(json!(self.bottom))),
            "focused" => Ok(QueryField::Value(json!(self.focused))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::Command;
    use crossterm::style::{ContentStyle, SetStyle};

    use super::*;
    use crate::ui::style::{THEME_DARK, TextStyle};
    use crate::ui::styled_string::StyledString;
    use serde_json::json;

    fn bar(height: usize, width: usize, num_rows: usize, top: usize, bottom: usize) -> ScrollBar {
        let mut bar = ScrollBar::new(&THEME_DARK);
        bar.set_height(height);
        bar.set_width(width);
        bar.set_num_rows(num_rows);
        bar.set_viewport(top, bottom);
        bar
    }

    fn render(bar: &ScrollBar) -> String {
        let mut rows: Vec<StyledString> = (0..bar.height())
            .map(|_| StyledString::new(bar.theme.base_style(), bar.width()))
            .collect();
        bar.draw(&mut rows);
        rows.iter()
            .map(|row| {
                let mut out = String::new();
                row.write_ansi(&mut out).unwrap();
                out
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn style_prefix(style: TextStyle) -> String {
        let mut out = String::new();
        let mut content: ContentStyle = style.into();
        content.background_color = Some(THEME_DARK.bg_base.into());
        SetStyle(content).write_ansi(&mut out).unwrap();
        out
    }

    #[test]
    fn test_scroll_bar_range() {
        // Viewport covers the entire content
        assert_eq!(bar(10, 1, 100, 0, 99).scroll_bar_range(), (0, 10));
        // Viewport in the middle of the content
        assert_eq!(bar(10, 1, 100, 20, 39).scroll_bar_range(), (2, 4));
        // Viewport at the bottom of the content
        assert_eq!(bar(10, 1, 100, 90, 99).scroll_bar_range(), (9, 10));
        // Single visible row; the bar is at least one row tall
        assert_eq!(bar(10, 1, 20, 0, 0).scroll_bar_range(), (0, 1));
        // Content smaller than the viewport
        assert_eq!(bar(10, 1, 5, 0, 4).scroll_bar_range(), (0, 10));
        // Empty content
        assert_eq!(bar(10, 1, 0, 0, 0).scroll_bar_range(), (0, 0));
        // Empty viewport
        assert_eq!(bar(0, 1, 100, 0, 99).scroll_bar_range(), (0, 0));
    }

    #[test]
    fn test_render_full_viewport() {
        let bar = bar(5, 1, 5, 0, 4);
        let unfocused = style_prefix(THEME_DARK.text_scroll_bar_unfocused);
        assert_eq!(
            render(&bar),
            format!(
                "{unfocused}▐\n{unfocused}▐\n{unfocused}▐\n{unfocused}▐\n{unfocused}▐",
            ),
        );
    }

    #[test]
    fn test_render_scrolled() {
        let bar = bar(5, 1, 20, 4, 8);
        let track = style_prefix(THEME_DARK.text_scroll_bar_track);
        let unfocused = style_prefix(THEME_DARK.text_scroll_bar_unfocused);
        assert_eq!(
            render(&bar),
            format!("{track}▐\n{unfocused}▐\n{track}▐\n{track}▐\n{track}▐"),
        );
    }

    #[test]
    fn test_render_focused() {
        let mut bar = bar(5, 1, 20, 4, 8);
        let track = style_prefix(THEME_DARK.text_scroll_bar_track);
        let focused = style_prefix(THEME_DARK.text_scroll_bar_focused);
        let unfocused = style_prefix(THEME_DARK.text_scroll_bar_unfocused);
        bar.set_focused(true);
        assert_eq!(
            render(&bar),
            format!("{track}▐\n{focused}▐\n{track}▐\n{track}▐\n{track}▐"),
        );
        assert!(bar.focused());

        bar.set_focused(false);
        assert_eq!(
            render(&bar),
            format!("{track}▐\n{unfocused}▐\n{track}▐\n{track}▐\n{track}▐"),
        );
        assert!(!bar.focused());
    }

    #[test]
    fn test_render_width() {
        let bar = bar(3, 2, 3, 0, 2);
        let unfocused = style_prefix(THEME_DARK.text_scroll_bar_unfocused);
        assert_eq!(
            render(&bar),
            format!("{unfocused}▐ \n{unfocused}▐ \n{unfocused}▐ "),
        );
    }

    #[test]
    fn test_render_empty() {
        let track = style_prefix(THEME_DARK.text_scroll_bar_track);
        let empty_content = bar(5, 1, 0, 0, 0);
        assert_eq!(
            render(&empty_content),
            format!("{track}▐\n{track}▐\n{track}▐\n{track}▐\n{track}▐"),
        );

        let empty_viewport = bar(0, 1, 100, 0, 99);
        assert_eq!(render(&empty_viewport), "");
    }

    #[test]
    fn test_size() {
        let mut bar = ScrollBar::new(&THEME_DARK);
        assert_eq!(bar.width(), 0);
        assert_eq!(bar.height(), 0);
        assert_eq!(bar.cursor(), None);

        bar.set_width(2);
        bar.set_height(4);
        assert_eq!(bar.width(), 2);
        assert_eq!(bar.height(), 4);

        bar.set_num_rows(8);
        bar.set_viewport(2, 5);
        assert_eq!(bar.scroll_bar_range(), (1, 3));
    }

    #[test]
    fn test_query() {
        let bar = ScrollBar::new(&THEME_DARK);
        let expected = json!({
            "height": 0,
            "width": 0,
            "num_rows": 0,
            "top": 0,
            "bottom": 0,
            "focused": false,
        });
        assert_eq!(bar.query("/").unwrap(), expected);
        assert_eq!(bar.query("/height").unwrap(), json!(0));
        assert_eq!(bar.query("/width").unwrap(), json!(0));
        assert_eq!(bar.query("/num_rows").unwrap(), json!(0));
        assert_eq!(bar.query("/top").unwrap(), json!(0));
        assert_eq!(bar.query("/bottom").unwrap(), json!(0));
        assert_eq!(bar.query("/focused").unwrap(), json!(false));
    }
}
