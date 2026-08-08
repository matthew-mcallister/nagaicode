mod chat;
mod input_box;
mod history;
mod padded;

pub use chat::run;
use crossterm::Command;

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

    fn drawable_rows(&self) -> Self::RowIter<'_>;
}
