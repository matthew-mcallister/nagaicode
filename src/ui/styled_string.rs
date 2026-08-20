use crossterm::Command;
use crossterm::style::SetStyle;

use crate::ui::style::{TextStyle, UpdateStyle};

/// Saved state for backtracking
#[derive(Clone, Copy, Debug)]
pub struct SavePoint {
    len: usize,
    width: usize,
    prev_style: Option<TextStyle>,
    cur_style: TextStyle,
    style_frozen: bool,
}

/// String wrapper which tracks current style and lazily writes control
/// statements when needed. Also tracks column information since we need that
/// anyways
#[derive(Debug)]
pub struct StyledString {
    inner: String,
    // Tracking previous style allows us to emit redundant style updates
    // without emitting extra control sequences
    prev_style: Option<TextStyle>,
    cur_style: TextStyle,
    width: usize,
    style_frozen: bool,
}

impl StyledString {
    pub fn new(style: TextStyle, capacity: usize) -> Self {
        Self {
            inner: String::with_capacity(capacity),
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

    pub fn cur_style(&self) -> TextStyle {
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

    pub fn set_style(&mut self, style: TextStyle) {
        if self.style_frozen { return; }
        if self.prev_style.is_none() {
            self.prev_style = Some(self.cur_style);
        }
        self.cur_style = style;
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

    // XXX shouldn't need this
    pub fn flush_style(&mut self) {
        let _ = SetStyle(self.cur_style().into()).write_ansi(&mut self.inner);
    }
}
