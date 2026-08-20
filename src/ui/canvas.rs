use crate::ui::styled_string::StyledString;

/// Target for draw commands. For subcomponents, this won't be the full
/// terminal but instead only the number of rows needed by the component.
/// Components are rendered from left to right with each component pushing to
/// existing rows.
#[derive(Debug)]
pub struct Canvas<'a> {
    pub rows: &'a mut [StyledString],
}

//pub type Canvas<'a> = &'a mut [StyledString];
