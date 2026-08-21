use crate::ui::styled_string::StyledString;

/// Target for draw commands. For subcomponents, this won't be the full
/// terminal but instead only the number of rows needed by the component.
/// Components are rendered from left to right with each component pushing to
/// existing rows.
pub type Canvas<'a> = &'a mut [StyledString];

#[cfg(test)]
pub fn render_canvas<'a>(canvas: Canvas<'a>) -> String {
    use crossterm::Command;
    let mut out = String::new();
    for (i, row) in canvas.iter().enumerate() {
        if i > 0 { out.push('\n'); }
        let _ = row.write_ansi(&mut out);
    }
    out
}
