use crate::ui::styled_string::StyledString;

/// Target for draw commands. For subcomponents, this won't be the full
/// terminal but instead only the number of rows needed by the component.
/// Components are rendered from left to right with each component pushing to
/// existing rows.
#[derive(Debug)]
pub struct Canvas<'a> {
    /// Right margin that child should not exceed. This is the *total* row
    /// width, including any content already pushed to the row, not the child
    /// component width.
    pub width: usize,
    pub rows: &'a mut [StyledString],
}
