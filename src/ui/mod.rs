// TODO maybe: write a diffing system that compares each row with the row from
// the previous draw and only redraws the portions which changed.

pub mod chat;
pub mod input_box;
pub mod history;
pub mod markdown;
pub mod padded;
pub mod stacked_view;
pub mod style;
pub mod text;

use std::fmt;

use crossterm::Command;

use text::SPACES;

/// Trait which all drawable UI components implement. This UI is *row-based*:
/// every drawable must be able to decompose itself into rows, which the parent
/// may style or transform as needed. Each row must be printable to the
/// terminal.
///
/// Each component is a rectangle with a width and height. All rows have the
/// same width. Generally, a component is responsible for determining its
/// childrens' sizes, but some components report an intrinsic size which the
/// parent may read back to compute its layout.
trait Component {
    type Row<'a>: Command where Self: 'a;
    type RowIter<'a>: Iterator<Item = Self::Row<'a>> where Self: 'a;

    /// Returns an iterator over the component's printable rows.
    fn drawable_rows(&self) -> Self::RowIter<'_>;

    /// Returns the component's width.
    fn width(&self) -> usize;

    /// Returns the component's height.
    fn height(&self) -> usize;

    /// Returns the (row, column) the cursor should appear at, relative to the
    /// component's top-left corner, when the component is focused/active.
    fn cursor_pos(&self) -> (usize, usize);

    /// Updates the component's width. The component should recompute its
    /// layout.
    fn set_width(&mut self, width: usize);

    /// Updates the component's height. The parent will never draw more rows
    /// than the component's height even if the drawable row iterator returns
    /// more values, i.e. overflow is always truncated. However, components
    /// should recompute their layout or scroll windows in response to height
    /// updates.
    fn set_height(&mut self, height: usize);
}

pub(crate) fn write_spaces(f: &mut impl fmt::Write, count: usize) -> fmt::Result {
    let mut remaining = count;
    while remaining != 0 {
        let n = remaining.min(SPACES.len());
        f.write_str(&SPACES[..n])?;
        remaining -= n;
    }
    Ok(())
}
