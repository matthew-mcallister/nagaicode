use crossterm::Command;
use crossterm::style::{Color, Print, SetStyle};

use crate::ui::style::{Style, TextStyle, UpdateStyle};
use crate::ui::text::SPACES;

/// Saved state for backtracking
#[derive(Clone, Copy, Debug)]
pub struct SavePoint {
    len: usize,
    width: usize,
    initial_style: Style,
    prev_style: Option<Style>,
    cur_style: Style,
    style_frozen: bool,
}

/// String wrapper which tracks current style and lazily writes control
/// statements when needed. Also tracks column information since we need that
/// anyways
#[derive(Clone, Debug)]
pub struct StyledString {
    inner: String,
    // Style at beginning of string, allows concatenating styled strings
    initial_style: Style,
    // Tracking previous style allows us to emit redundant style updates
    // without emitting extra control sequences
    prev_style: Option<Style>,
    cur_style: Style,
    width: usize,
    style_frozen: bool,
}

impl StyledString {
    pub fn new(style: Style, capacity: usize) -> Self {
        Self {
            inner: String::with_capacity(capacity),
            initial_style: style,
            prev_style: None,
            cur_style: style,
            width: 0,
            style_frozen: false,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn cur_style(&self) -> Style {
        self.cur_style
    }

    pub fn initial_style(&self) -> Style {
        self.initial_style
    }

    pub fn into_inner(self) -> String {
        self.inner
    }

    /// Clones a styled string but allocates (at least) the given amount of
    /// capacity.
    pub fn clone_with_capacity(&self, capacity: usize) -> Self {
        let mut inner = String::with_capacity(capacity.min(self.inner.len()));
        inner.push_str(&self.inner);
        Self {
            inner,
            ..*self
        }
    }

    pub fn set_style(&mut self, style: Style) {
        if self.style_frozen { return; }
        if self.inner.is_empty() {
            self.initial_style = style;
            self.cur_style = style;
            return;
        }
        if self.prev_style.is_none() {
            self.prev_style = Some(self.cur_style);
        }
        self.cur_style = style;
    }

    pub fn set_text(&mut self, text: TextStyle) {
        self.set_style(self.cur_style.with_text(text));
    }

    pub fn set_bg_color(&mut self, bg_color: Color) {
        self.set_style(self.cur_style.with_bg_color(bg_color));
    }

    // Kludge for preventing child nodes from changing the style inside code
    // and quote blocks
    pub fn freeze_style(&mut self, value: bool) {
        self.style_frozen = value;
    }

    pub fn push(&mut self, s: &str, width: usize) {
        if let Some(prev) = self.prev_style {
            let _ = UpdateStyle(prev, self.cur_style).write_ansi(&mut self.inner);
        }
        self.inner.push_str(s);
        self.width += width;
        self.prev_style = None;
    }

    pub fn push_styled(&mut self, other: &StyledString) {
        self.set_style(other.initial_style);
        self.push(&other.inner, other.width);
        self.cur_style = other.cur_style;
        self.prev_style = other.prev_style;
        self.style_frozen = other.style_frozen;
    }

    pub fn save(&mut self) -> SavePoint {
        SavePoint {
            len: self.len(),
            width: self.width(),
            initial_style: self.initial_style,
            prev_style: self.prev_style,
            cur_style: self.cur_style,
            style_frozen: self.style_frozen,
        }
    }

    pub fn restore(&mut self, saved: SavePoint) {
        self.inner.truncate(saved.len);
        self.initial_style = saved.initial_style;
        self.prev_style = saved.prev_style;
        self.cur_style = saved.cur_style;
        self.width = saved.width;
        self.style_frozen = saved.style_frozen;
    }

    pub fn pad_to_width(&mut self, width: usize) {
        if self.width >= width { return; }
        while self.width < width {
            let n = (width - self.width).min(SPACES.len());
            self.push(&SPACES[..n], n);
        }
    }

    /// Shortcut for `pad_to_width(self.width() + count)`
    pub fn pad(&mut self, count: usize) {
        self.pad_to_width(self.width + count)
    }
}

impl Command for StyledString {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        SetStyle(self.initial_style.into()).write_ansi(f)?;
        Print(&self.inner[..]).write_ansi(f)?;
        Ok(())
    }
}

impl std::fmt::Display for StyledString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.write_ansi(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::style::{THEME_DARK, UpdateStyle};

    #[test]
    fn test_render_and_width() {
        let theme = &THEME_DARK;
        let base = theme.base_style();
        let header = Style::new(theme.text_header, theme.bg_base);
        let code = Style::new(theme.text_code, theme.bg_base);
        let to_header = UpdateStyle(base, header);
        let to_code = UpdateStyle(header, code);

        let mut s = StyledString::new(base, 16);
        assert_eq!(s.width(), 0);
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        assert_eq!(s.cur_style(), base);
        assert_eq!(s.initial_style(), base);
        assert_eq!(format!("{s}"), format!("{base}"));

        s.push("hello", 5);
        assert_eq!(s.width(), 5);
        assert_eq!(s.len(), 5);
        assert!(!s.is_empty());
        assert_eq!(format!("{s}"), format!("{base}hello"));

        s.set_style(header);
        assert_eq!(s.cur_style(), header);
        assert_eq!(s.width(), 5);

        s.push(" world", 6);
        assert_eq!(s.width(), 11);
        assert_eq!(format!("{s}"), format!("{base}hello{to_header} world"));

        s.set_style(code);
        s.push("!", 1);
        assert_eq!(s.width(), 12);
        assert_eq!(format!("{s}"), format!("{base}hello{to_header} world{to_code}!"));

        let mut collapsed = StyledString::new(base, 16);
        collapsed.push("a", 1);
        collapsed.set_style(header);
        collapsed.set_style(code);
        assert_eq!(collapsed.cur_style(), code);
        let to_code_from_base = UpdateStyle(base, code);
        collapsed.push("b", 1);
        assert_eq!(format!("{collapsed}"), format!("{base}a{to_code_from_base}b"));

        let mut other = StyledString::new(header, 16);
        other.push("cd", 2);
        other.set_style(code);
        other.push("ef", 2);

        let mut combined = StyledString::new(base, 16);
        combined.push("ab", 2);
        combined.push_styled(&other);
        assert_eq!(combined.width(), 6);
        assert_eq!(combined.cur_style(), code);
        assert_eq!(
            format!("{combined}"),
            format!("{base}ab{to_header}cd{to_code}ef"),
        );

        let cloned = combined.clone_with_capacity(64);
        assert_eq!(cloned.width(), combined.width());
        assert_eq!(cloned.cur_style(), combined.cur_style());
        assert_eq!(cloned.initial_style(), combined.initial_style());
        assert_eq!(format!("{cloned}"), format!("{combined}"));
        assert_eq!(cloned.into_inner(), combined.into_inner());
    }

    #[test]
    fn test_save_restore() {
        let theme = &THEME_DARK;
        let base = theme.base_style();
        let header = Style::new(theme.text_header, theme.bg_base);
        let code = Style::new(theme.text_code, theme.bg_base);
        let to_code = UpdateStyle(base, code);

        let mut s = StyledString::new(base, 16);
        let empty_save = s.save();

        s.set_style(header);
        assert_eq!(s.initial_style(), header);
        assert_eq!(s.cur_style(), header);
        s.push("title", 5);
        assert_eq!(format!("{s}"), format!("{header}title"));

        s.restore(empty_save);
        assert_eq!(s.initial_style(), base);
        assert_eq!(s.cur_style(), base);
        assert_eq!(s.width(), 0);
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        assert_eq!(format!("{s}"), format!("{base}"));

        s.push("foo", 3);
        let mid_save = s.save();

        s.set_style(header);
        s.push("bar", 3);
        s.set_style(code);
        s.push("baz", 3);
        assert_eq!(s.width(), 9);

        s.restore(mid_save);
        assert_eq!(s.width(), 3);
        assert_eq!(s.cur_style(), base);
        assert_eq!(format!("{s}"), format!("{base}foo"));

        s.push("qux", 3);
        assert_eq!(s.width(), 6);
        assert_eq!(format!("{s}"), format!("{base}fooqux"));

        s.set_style(code);
        let pending_save = s.save();
        s.set_style(header);
        s.push("x", 1);
        s.restore(pending_save);
        s.push("y", 1);
        assert_eq!(s.width(), 7);
        assert_eq!(format!("{s}"), format!("{base}fooqux{to_code}y"));
    }

    #[test]
    fn test_style_frozen() {
        let theme = &THEME_DARK;
        let base = theme.base_style();
        let header = Style::new(theme.text_header, theme.bg_base);
        let to_header = UpdateStyle(base, header);

        let mut s = StyledString::new(base, 16);
        s.push("normal", 6);
        s.freeze_style(true);

        s.set_style(header);
        s.set_text(theme.text_code);
        s.set_bg_color(theme.bg_prompt);
        assert_eq!(s.cur_style(), base);

        s.push(" frozen", 7);
        assert_eq!(s.width(), 13);
        assert_eq!(format!("{s}"), format!("{base}normal frozen"));

        s.freeze_style(false);
        s.set_style(header);
        s.push(" unfrozen", 9);
        assert_eq!(s.width(), 22);
        assert_eq!(format!("{s}"), format!("{base}normal frozen{to_header} unfrozen"));

        let mut frozen_src = StyledString::new(base, 16);
        frozen_src.freeze_style(true);
        let mut target = StyledString::new(base, 16);
        target.push_styled(&frozen_src);
        target.set_style(header);
        assert_eq!(target.cur_style(), base);
    }

    #[test]
    fn test_pad_to_width() {
        let theme = &THEME_DARK;
        let base = theme.base_style();
        let header = Style::new(theme.text_header, theme.bg_base);
        let to_header = UpdateStyle(base, header);

        let mut s = StyledString::new(base, 16);
        s.push("hello", 5);
        s.pad_to_width(5);
        assert_eq!(s.width(), 5);
        assert_eq!(format!("{s}"), format!("{base}hello"));

        s.pad_to_width(3);
        assert_eq!(s.width(), 5);
        assert_eq!(format!("{s}"), format!("{base}hello"));

        s.pad_to_width(9);
        assert_eq!(s.width(), 9);
        assert_eq!(format!("{s}"), format!("{base}hello    "));

        s.pad(3);
        assert_eq!(s.width(), 12);
        assert_eq!(format!("{s}"), format!("{base}hello       "));

        s.set_style(header);
        s.pad(4);
        assert_eq!(s.width(), 16);
        assert_eq!(format!("{s}"), format!("{base}hello       {to_header}    "));

        let mut empty = StyledString::new(base, 16);
        empty.pad_to_width(4);
        assert_eq!(empty.width(), 4);
        assert_eq!(format!("{empty}"), format!("{base}    "));

        let large_target = SPACES.len() + 100;
        let mut large = StyledString::new(base, large_target + 4);
        large.push("ab", 2);
        large.pad_to_width(large_target);
        assert_eq!(large.width(), large_target);
        assert_eq!(large.len(), large_target);
        let rendered = format!("{large}");
        assert!(rendered.starts_with(&format!("{base}ab")));
        assert!(rendered[format!("{base}ab").len()..].chars().all(|c| c == ' '));
    }
}
