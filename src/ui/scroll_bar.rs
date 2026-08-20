//! Scroll bar indicator. Renders as a one-column-wide bar on the left side of
//! the component. The bar's position and height are derived from the size of
//! the scrolled content and the position of the viewport within it.

use std::fmt;

use crossterm::Command;
use crossterm::event::Event;
use crossterm::style::{Color, ContentStyle, SetStyle};

use crate::ui::canvas::Canvas;
use crate::ui::style::{TextStyle, Theme};
use crate::ui::{write_spaces, Component};

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
        let bar_height = ((self.bottom - self.top + 1) * self.height).div_ceil(self.num_rows);
        let start = (self.top * self.height) / self.num_rows;
        (start, start + bar_height)
    }
}

/// A single drawable row of the scroll bar. The first column is filled with
/// the bar character, styled as either the bar or the track.
#[derive(Debug)]
pub struct ScrollBarRow<'a> {
    style: &'a TextStyle,
    background: Color,
    width: usize,
}

impl Command for ScrollBarRow<'_> {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        if self.width == 0 {
            return Ok(());
        }
        let mut style: ContentStyle = (*self.style).into();
        style.background_color = Some(self.background);
        SetStyle(style).write_ansi(f)?;
        f.write_char('▐')?;
        write_spaces(f, self.width - 1)
    }
}

impl Component for ScrollBar {
    type Row<'a> = ScrollBarRow<'a> where Self: 'a;
    type RowIter<'a> = Box<dyn Iterator<Item = Self::Row<'a>> + 'a> where Self: 'a;
    type Update<'a> = ();
    type Event = ();

    fn drawable_rows(&self) -> Self::RowIter<'_> {
        let (start, end) = self.scroll_bar_range();
        Box::new((0..self.height).map(move |row| ScrollBarRow {
            style: if (start..end).contains(&row) {
                if self.focused {
                    &self.theme.text_scroll_bar_focused
                } else {
                    &self.theme.text_scroll_bar_unfocused
                }
            } else {
                &self.theme.text_scroll_bar_track
            },
            background: self.theme.bg_base,
            width: self.width,
        }))
    }

    fn draw(&self, canvas: &mut Canvas) {
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
            canvas.rows[row].set_text(text_style);
            canvas.rows[row].push("▐", 1);
            canvas.rows[row].pad_to_width(canvas.width);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::style::THEME_DARK;

    fn bar(height: usize, width: usize, num_rows: usize, top: usize, bottom: usize) -> ScrollBar {
        let mut bar = ScrollBar::new(&THEME_DARK);
        bar.set_height(height);
        bar.set_width(width);
        bar.set_num_rows(num_rows);
        bar.set_viewport(top, bottom);
        bar
    }

    fn render(bar: &ScrollBar) -> String {
        bar.drawable_rows()
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
        content.background_color = Some(THEME_DARK.bg_base);
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
            format!("{track}▐\n{unfocused}▐\n{unfocused}▐\n{track}▐\n{track}▐"),
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
            format!("{track}▐\n{focused}▐\n{focused}▐\n{track}▐\n{track}▐"),
        );
        assert!(bar.focused());

        bar.set_focused(false);
        assert_eq!(
            render(&bar),
            format!("{track}▐\n{unfocused}▐\n{unfocused}▐\n{track}▐\n{track}▐"),
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
}
