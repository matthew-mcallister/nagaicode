use crossterm::Command;
use crossterm::event::Event;

/// Trait which all drawable UI components implement. This UI is *row-based*:
/// every drawable must be able to decompose itself into rows, which the parent
/// may style or transform as needed. Each row must be printable to the
/// terminal.
///
/// Each component is a rectangle with a width and height. All rows have the
/// same width; content may be padded with space to fit. Generally, a component
/// is responsible for determining its childrens' sizes, but some components
/// report an intrinsic size which the parent may read back to compute its
/// layout.
// XXX: Maybe get rid of set_width/set_height/set_focus and pass those as
// parameters to draw()? Then draw() will lazily recompute style and layout
// when it notices the params change. Basically replacing state with caching.
pub trait Component {
    type Row<'a>: Command where Self: 'a;
    type RowIter<'a>: Iterator<Item = Self::Row<'a>> where Self: 'a;
    /// The type(s) of update which the component responds to
    type Update;
    /// Event(s) fired in response to input.
    type Event;

    /// Returns an iterator over the component's printable rows.
    fn drawable_rows(&self) -> Self::RowIter<'_>;

    /// Returns the component's width.
    fn width(&self) -> usize;

    /// Returns the component's height.
    fn height(&self) -> usize;

    /// Returns the (row, column) the cursor should appear at, relative to the
    /// component's top-left corner, when the component is focused/active.
    /// Returns `None` if the cursor should be hidden.
    fn cursor(&self) -> Option<(usize, usize)>;

    /// Updates the component's width. The component should recompute its
    /// layout.
    fn set_width(&mut self, width: usize);

    /// Updates the component's height. The parent will never draw more rows
    /// than the component's height even if the drawable row iterator returns
    /// more values, i.e. overflow is always truncated. However, components
    /// should recompute their layout or scroll windows in response to height
    /// updates.
    fn set_height(&mut self, height: usize);

    /// Tells a component when it is focused, mainly for styling.
    fn set_focus(&mut self, focused: bool);

    /// Handles a raw terminal input event directed at this component (or
    /// descendant) and returns any event raised in response. For components
    /// with multiple children, inputs are routed to the focused child.
    fn handle_input(&mut self, event: Event) -> Self::Event;

    /// Updates the component in response to a change in an ancestor. Updates
    /// should be forwarded to all children. This method is meant to facilitate
    /// long-range side-effects; not every mutation has to be handled this way.
    fn handle_update(&mut self, update: Self::Update);
}
