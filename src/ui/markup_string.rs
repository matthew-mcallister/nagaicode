//! Styled text content. Intended to replace StyledString.
//!
//! Most `push` methods assume adjacent combining characters are not pushed in
//! separate calls. String width and styling may break if combining codepoints
//! are not pushed as a single unit.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::ui::style::{Color, Style, TextStyle};
use crate::ui::styled_string::StyledString;
use crate::ui::text::{SPACES, wrap_line, wrap_line_naive};

// TODO: Style should be replaced with this
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct State {
    pub fg_color: Option<Color>,
    pub bg_color: Option<Color>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub strikethrough: Option<bool>,
}

impl From<TextStyle> for State {
    fn from(value: TextStyle) -> Self {
        Self {
            fg_color: Some(value.fg_color),
            bg_color: None,
            bold: Some(value.bold),
            italic: Some(value.italic),
            underline: Some(value.underline),
            strikethrough: Some(value.strikethrough),
        }
    }
}

impl From<Style> for State {
    fn from(value: Style) -> Self {
        Self {
            bg_color: Some(value.bg_color),
            ..value.text.into()
        }
    }
}

impl State {
    pub fn with_base(self, base: State) -> Self {
        Self {
            fg_color: self.fg_color.or(base.fg_color),
            bg_color: self.bg_color.or(base.bg_color),
            bold: self.bold.or(base.bold),
            italic: self.italic.or(base.italic),
            underline: self.underline.or(base.underline),
            strikethrough: self.strikethrough.or(base.strikethrough),
        }
    }

    pub fn with_bold(self) -> Self {
        Self { bold: Some(true), ..self }
    }

    pub fn with_italic(self) -> Self {
        Self { italic: Some(true), ..self }
    }

    pub fn with_underline(self) -> Self {
        Self { underline: Some(true), ..self }
    }

    pub fn with_strikethrough(self) -> Self {
        Self { strikethrough: Some(true), ..self }
    }

    pub fn to_style(self, base: Style) -> Style {
        Style {
            text: TextStyle {
                fg_color: self.fg_color.unwrap_or(base.text.fg_color),
                bold: self.bold.unwrap_or(base.text.bold),
                italic: self.italic.unwrap_or(base.text.italic),
                underline: self.underline.unwrap_or(base.text.underline),
                strikethrough: self.strikethrough.unwrap_or(base.text.strikethrough),
            },
            bg_color: self.bg_color.unwrap_or(base.bg_color),
        }
    }
}

/// Out-of-line style marker.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct Marker {
    /// Byte offset to insert control sequence in plain_text
    offset: usize,
    state: State,
}

/// A single line of text content with styling applied.
#[derive(Debug, Default)]
pub struct MarkupString {
    plain_text: String,
    markers: Vec<Marker>,
    width: usize,
    state: State,
}

impl MarkupString {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(text: usize, markers: usize) -> Self {
        Self {
            plain_text: String::with_capacity(text),
            markers: Vec::with_capacity(markers),
            width: 0,
            state: State::default(),
        }
    }

    pub fn push_str(&mut self, s: &str) {
        let start = self.plain_text.len();
        self.plain_text.push_str(s);
        self.width += self.plain_text[start..].width();
    }

    pub fn push_char(&mut self, c: char) {
        self.plain_text.push(c);
        self.width += c.width().unwrap_or(0);
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn plain_text(&self) -> &str {
        &self.plain_text
    }

    /// Append another styled string to this string.
    ///
    /// To inherit the current string's style, call [`apply_base_style`] first.
    pub fn push(&mut self, rhs: &Self) {
        for marker in rhs.markers.iter() {
            self.markers.push(Marker {
                offset: self.plain_text.len() + marker.offset,
                state: marker.state,
            });
        }
        self.plain_text.push_str(&rhs.plain_text);
        self.width += rhs.width;
        self.state = rhs.state.with_base(self.state);
    }

    /// Fills in unset style parameters from a base style.
    pub fn apply_base_style(&mut self, base: State) {
        for marker in self.markers.iter_mut() {
            marker.state = marker.state.with_base(base);
        }
    }

    pub fn set_state(&mut self, state: State) {
        self.state = state;
        self.markers.push(Marker {
            offset: self.plain_text.len(),
            state,
        });
    }

    pub fn set_text(&mut self, style: TextStyle) {
        self.set_state(State::from(style).with_base(self.state));
    }

    pub fn set_style(&mut self, style: Style) {
        self.set_state(style.into());
    }

    pub fn pad_to_width(&mut self, width: usize) {
        if self.width >= width { return; }
        while self.width < width {
            let n = (width - self.width).min(SPACES.len());
            self.push_str(&SPACES[..n]);
        }
    }

    fn wrap_inner(&self, max_width: usize, naive: bool) -> Vec<Self> {
        let markers = &self.markers;
        let mut state = State::default();
        let mut marker_idx = 0usize;
        let mut offset = 0usize;
        let rows = if naive {
            wrap_line_naive(max_width, &self.plain_text)
        } else {
            wrap_line(max_width, &self.plain_text)
        };
        rows.into_iter().map(move |row| {
            while marker_idx < markers.len() && markers[marker_idx].offset <= offset {
                state = markers[marker_idx].state;
                marker_idx += 1;
            }

            let mut s = Self::new();
            // Re-apply current style at start of each row
            s.set_state(state.into());

            for g in &row.graphemes {
                // Newline added by wrap_line, not part of the source text
                if g.data == "\n" { continue; }
                s.push_str(&g.data);
                offset += g.data.len();

                let prev_state = state;
                while marker_idx < markers.len() && markers[marker_idx].offset <= offset {
                    state = markers[marker_idx].state;
                    marker_idx += 1;
                }
                if prev_state != state { s.set_state(state.into()) }
            }

            s
        }).collect()
    }

    /// Wraps the string at a given width following the behavior of
    /// [`wrap_line`].
    pub fn wrap(&self, max_width: usize) -> Vec<Self> {
        self.wrap_inner(max_width, false)
    }

    /// Wraps the string at a given width following the behavior of
    /// [`wrap_line_naive`].
    pub fn wrap_naive(&self, max_width: usize) -> Vec<Self> {
        self.wrap_inner(max_width, true)
    }

    pub fn to_ansi(&self, base: Style) -> StyledString {
        let mut s = StyledString::new(base, 2 * self.plain_text.capacity());
        let mut offset = 0;
        for marker in self.markers.iter() {
            if marker.offset != offset {
                s.push_str(&self.plain_text[offset..marker.offset]);
                offset = marker.offset;
            }
            s.set_style(marker.state.to_style(base));
        }
        s.push_str(&self.plain_text[offset..]);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::style::{THEME_DARK, UpdateStyle};

    #[test]
    fn test_wrap() {
        let base = THEME_DARK.base_style();
        let code = Style::new(THEME_DARK.text_code, THEME_DARK.bg_base);
        let to_code = UpdateStyle(base, code);
        let to_base = UpdateStyle(code, base);

        let mut markup = MarkupString::new();
        markup.push_str("12345");
        markup.set_style(code);
        markup.push_str("6789012345");
        markup.set_style(base);
        markup.push_str("67890");

        let mut out = StyledString::new(base, 64);
        for (i, row) in markup.wrap_naive(10).iter().enumerate() {
            if i > 0 { out.push("\n", 0); }
            out.push_markup(row);
        }

        assert_eq!(
            format!("{out}"),
            format!("{base}12345{to_code}67890\n12345{to_base}67890"),
        );
    }
}
