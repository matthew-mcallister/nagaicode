use crossterm::Command;
use crossterm::style::{Color, Print, SetStyle};

use crate::ui::style::{Style, TextStyle, UpdateStyle};
use crate::ui::text::SPACES;

/// Saved state for backtracking
#[derive(Clone, Copy, Debug)]
pub struct SavePoint {
    len: usize,
    width: usize,
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

    pub fn cur_style(&self) -> Style {
        self.cur_style
    }

    pub fn as_str(&self) -> &str {
        &self.inner
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
        self.set_style(other.cur_style);
        self.inner.push_str(&other.inner);
        self.prev_style = other.prev_style;
        self.width += other.width;
        self.style_frozen = other.style_frozen;
    }

    pub fn save(&mut self) -> SavePoint {
        SavePoint {
            len: self.len(),
            width: self.width(),
            prev_style: self.prev_style,
            cur_style: self.cur_style,
            style_frozen: self.style_frozen,
        }
    }

    pub fn restore(&mut self, saved: SavePoint) {
        self.inner.truncate(saved.len);
        self.prev_style = saved.prev_style;
        self.cur_style = saved.cur_style;
        self.width = saved.width;
        self.style_frozen = saved.style_frozen;
    }

    /// Writes a full style update to the string.
    // XXX delete this after switching to draw() rendering
    pub fn flush_style(&mut self) {
        let _ = SetStyle(self.cur_style().into()).write_ansi(&mut self.inner);
    }

    pub fn pad_to_width(&mut self, width: usize) {
        if self.width >= width { return; }
        while self.width < width {
            let n = (width - self.width).min(SPACES.len());
            self.push(&SPACES[..n], n);
        }
    }
}

impl Command for StyledString {
    fn write_ansi(&self, f: &mut impl std::fmt::Write) -> std::fmt::Result {
        SetStyle(self.initial_style.into()).write_ansi(f)?;
        Print(&self.inner[..]).write_ansi(f)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::style::{Style, THEME_DARK};

    fn transition(old: Style, new: Style) -> String {
        let mut out = String::new();
        UpdateStyle(old, new).write_ansi(&mut out).unwrap();
        out
    }

    fn full_style(style: Style) -> String {
        let mut out = String::new();
        SetStyle(style.into()).write_ansi(&mut out).unwrap();
        out
    }

    #[test]
    fn test_push_and_width() {
        let base = THEME_DARK.base_style();
        let mut s = StyledString::new(base, 16);

        assert_eq!(s.len(), 0);
        assert_eq!(s.width(), 0);
        assert_eq!(s.as_str(), "");
        assert_eq!(s.cur_style(), base);

        // Pushing without a style change emits no control codes
        s.push("hello", 5);
        assert_eq!(s.as_str(), "hello");
        assert_eq!(s.len(), 5);
        assert_eq!(s.width(), 5);
        assert!(!s.as_str().contains('\x1b'));

        s.push(" world", 6);
        assert_eq!(s.as_str(), "hello world");
        assert_eq!(s.width(), 11);
        assert_eq!(s.len(), "hello world".len());
    }

    #[test]
    fn test_style_transitions() {
        let theme = &THEME_DARK;
        let base = theme.base_style();
        let header = Style::new(theme.text_header, theme.bg_base);
        let code = Style::new(theme.text_code, theme.bg_base);
        let math = Style::new(theme.text_math, theme.bg_base);

        // set_style before any content updates the initial style silently
        let mut s = StyledString::new(base, 16);
        s.set_style(header);
        assert_eq!(s.cur_style(), header);
        s.push("Title", 5);
        assert_eq!(s.as_str(), "Title");
        assert!(!s.as_str().contains('\x1b'));

        // Changing style after content marks a transition emitted on next push
        s.set_style(code);
        assert_eq!(s.cur_style(), code);
        s.push("code", 4);
        assert_eq!(s.as_str(), format!("Title{}code", transition(header, code)));

        // Multiple set_style calls collapse into a single transition on push
        s.set_style(header);
        s.set_style(math);
        assert_eq!(s.cur_style(), math);
        let prev = format!("Title{}code", transition(header, code));
        s.push("math", 4);
        assert_eq!(s.as_str(), format!("{}{}math", prev, transition(code, math)));
    }

    #[test]
    fn test_push_styled() {
        let theme = &THEME_DARK;
        let base = theme.base_style();
        let header = Style::new(theme.text_header, theme.bg_base);

        // Matching style: content/width appended, no transitions, state clean
        let mut a = StyledString::new(base, 16);
        a.push("foo", 3);
        let mut b = StyledString::new(base, 16);
        b.push("bar", 3);
        a.push_styled(&b);
        assert_eq!(a.as_str(), "foobar");
        assert_eq!(a.width(), 6);
        assert_eq!(a.len(), 6);
        assert_eq!(a.cur_style(), base);
        assert_eq!(a.prev_style, None);
        assert!(!a.style_frozen);

        // Different style on a flush_style'd other: full style baked into content
        let mut c = StyledString::new(base, 16);
        c.push("foo", 3);
        let mut d = StyledString::new(header, 16);
        d.flush_style();
        d.push("bar", 3);
        c.push_styled(&d);
        assert_eq!(c.as_str(), format!("foo{}bar", full_style(header)));
        assert_eq!(c.width(), 6);
        assert_eq!(c.cur_style(), header);
        assert_eq!(c.prev_style, None);

        // Inherits other's pending transition; subsequent push emits it
        let mut e = StyledString::new(base, 16);
        e.push("foo", 3);
        let mut f = StyledString::new(base, 16);
        f.push("x", 1);
        f.set_style(header);
        assert_eq!(f.prev_style, Some(base));
        e.push_styled(&f);
        assert_eq!(e.as_str(), "foox");
        assert_eq!(e.cur_style(), header);
        assert_eq!(e.prev_style, Some(base));
        e.push("!", 1);
        assert_eq!(e.as_str(), format!("foox{}!", transition(base, header)));

        // Inherits other's style_frozen
        let mut g = StyledString::new(base, 16);
        g.freeze_style(true);
        g.push("x", 1);
        let mut h = StyledString::new(base, 16);
        h.push("foo", 3);
        h.push_styled(&g);
        assert!(h.style_frozen);
        assert_eq!(h.cur_style(), base);

        // freeze_style on self blocks set_style inside push_styled
        let mut i = StyledString::new(base, 16);
        i.push("foo", 3);
        i.freeze_style(true);
        let mut j = StyledString::new(header, 16);
        j.push("bar", 3);
        i.push_styled(&j);
        assert_eq!(i.cur_style(), base);
        assert_eq!(i.prev_style, None);
        assert_eq!(i.as_str(), "foobar");
    }

    #[test]
    fn test_save_restore_and_freeze() {
        let theme = &THEME_DARK;
        let base = theme.base_style();
        let header = Style::new(theme.text_header, theme.bg_base);

        let mut s = StyledString::new(base, 16);
        s.push("abc", 3);
        let save = s.save();
        s.push("de", 2);
        s.set_style(header);
        s.push("fg", 2);
        assert_eq!(s.width(), 7);

        // Restore reverts content, width, and style to the save point
        s.restore(save);
        assert_eq!(s.as_str(), "abc");
        assert_eq!(s.width(), 3);
        assert_eq!(s.len(), 3);
        assert_eq!(s.cur_style(), base);
        assert_eq!(s.prev_style, None);

        // freeze_style blocks subsequent set_style calls
        s.freeze_style(true);
        s.set_style(header);
        assert_eq!(s.cur_style(), base);
        assert_eq!(s.prev_style, None);

        // Unfreezing allows style changes again
        s.freeze_style(false);
        s.set_style(header);
        assert_eq!(s.cur_style(), header);
        assert_eq!(s.prev_style, Some(base));

        // flush_style always emits the full current style
        let mut t = StyledString::new(base, 16);
        t.set_style(header);
        t.flush_style();
        assert_eq!(t.as_str(), full_style(header));
    }

    #[test]
    fn test_pad_to_width() {
        let theme = &THEME_DARK;
        let base = theme.base_style();
        let header = Style::new(theme.text_header, theme.bg_base);

        // No-op when already at the target width
        let mut s = StyledString::new(base, 16);
        s.push("hello", 5);
        s.pad_to_width(5);
        assert_eq!(s.as_str(), "hello");
        assert_eq!(s.width(), 5);
        assert_eq!(s.len(), 5);

        // No-op when already wider than the target
        s.pad_to_width(3);
        assert_eq!(s.as_str(), "hello");
        assert_eq!(s.width(), 5);

        // Pads with spaces up to the target width, no control codes
        s.pad_to_width(11);
        assert_eq!(s.as_str(), "hello      ");
        assert_eq!(s.width(), 11);
        assert_eq!(s.len(), "hello      ".len());
        assert!(!s.as_str().contains('\x1b'));

        // Pads from an empty string
        let mut t = StyledString::new(base, 16);
        t.pad_to_width(4);
        assert_eq!(t.as_str(), "    ");
        assert_eq!(t.width(), 4);

        // Padding past SPACES.len() still reaches the target
        let mut u = StyledString::new(base, SPACES.len() + 102);
        u.push("ab", 2);
        u.pad_to_width(SPACES.len() + 100);
        assert_eq!(u.width(), SPACES.len() + 100);
        assert_eq!(u.len(), SPACES.len() + 100);
        assert!(u.as_str().starts_with("ab"));
        assert!(u.as_str().chars().skip(2).all(|c| c == ' '));

        // Padding after a style change emits the transition before the spaces
        let mut v = StyledString::new(base, 16);
        v.push("foo", 3);
        v.set_style(header);
        v.pad_to_width(7);
        assert_eq!(v.as_str(), format!("foo{}    ", transition(base, header)));
        assert_eq!(v.width(), 7);
    }
}
