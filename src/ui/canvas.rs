use crate::ui::styled_string::StyledString;

/// Target for draw commands. For subcomponents, this won't be the full
/// terminal but instead only the number of rows needed by the component.
#[derive(Debug)]
pub struct Canvas<'a> {
    pub width: usize,
    pub rows: &'a [StyledString],
}
