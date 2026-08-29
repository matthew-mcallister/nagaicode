use fnv::FnvHashMap;
use serde_json::Value;

use crate::ui::style::{Style, Theme};
use crate::ui::styled_string::StyledString;
use crate::ui::text::{SPACES, wrap_line_naive};

/// Implementor is able to render a class of tool call outputs.
pub trait ToolOutputRenderer {
    /// Renders output
    fn render(
        &self,
        theme: &'static Theme,
        width: usize,
        args: &Value,
        output: &str,
    ) -> Vec<StyledString>;
}

/// Renders tool calls by name.
pub struct ToolRenderer {
    map: FnvHashMap<String, Box<dyn ToolOutputRenderer>>,
}

impl ToolRenderer {
    pub fn new() -> Self {
        Self {
            map: Default::default(),
        }
    }

    /// Renders a tool call by name, args, and output.
    ///
    /// `name` is the name of the renderer to use, not necessarily the name
    /// of the tool.
    pub fn render(
        &self,
        theme: &'static Theme,
        width: usize,
        name: &str,
        args: &Value,
        output: &str,
    ) -> Option<Vec<StyledString>> {
        Some(self.map.get(name)?.render(theme, width, args, output))
    }

    pub fn register(&mut self, name: impl Into<String>, renderer: Box<dyn ToolOutputRenderer>) {
        self.map.insert(name.into(), renderer);
    }
}

// In the future may support custom tool renderers.
pub fn load_tool_renderers() -> ToolRenderer {
    let mut renderer = ToolRenderer::new();
    renderer.register("sh", Box::new(ShRenderer));
    renderer
}

/// Renders the stdout of an `sh` tool call with the same background and
/// padding as a user prompt. stderr is not rendered.
pub struct ShRenderer;

impl ToolOutputRenderer for ShRenderer {
    fn render(
        &self,
        theme: &'static Theme,
        width: usize,
        _args: &Value,
        output: &str,
    ) -> Vec<StyledString> {
        let stdout = serde_json::from_str::<Value>(output)
            .map(|json| {
                json.get("stdout")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            })
            .unwrap_or_default();
        render_sh_stdout(theme, width, &stdout)
    }
}

fn render_sh_stdout(theme: &'static Theme, width: usize, stdout: &str) -> Vec<StyledString> {
    if stdout.is_empty() {
        return Vec::new();
    }
    let style = Style::new(theme.text_base, theme.bg_prompt);
    let ellipsis_style = Style::new(theme.text_subtle, theme.bg_prompt);

    let lines: Vec<&str> = stdout.lines().collect();
    let truncated = lines.len() > 11;
    let selected = if truncated {
        let mut selected: Vec<&str> = lines[..5].to_vec();
        selected.push("...");
        selected.extend_from_slice(&lines[lines.len() - 5..]);
        selected
    } else {
        lines
    };

    let make_padding = || {
        let mut s = StyledString::new(style, width);
        s.push(&SPACES[..width], width);
        s
    };

    let mut rows = vec![make_padding()];
    for (i, line) in selected.into_iter().enumerate() {
        let row_style = if truncated && i == 5 {
            ellipsis_style
        } else {
            style
        };
        for row in wrap_line_naive(width - 4, line) {
            let mut s = StyledString::new(row_style, width);
            s.push("  ", 2);
            s.push(&row.to_padded_string(width - 4), width - 4);
            s.push("  ", 2);
            rows.push(s);
        }
    }
    rows.push(make_padding());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::canvas::render_canvas;
    use crate::ui::style::THEME_DARK;
    use serde_json::json;

    fn render(width: usize, output: &Value) -> String {
        let renderer = ShRenderer;
        let mut lines = renderer.render(
            &THEME_DARK,
            width,
            &json!({ "command": "echo hi" }),
            &output.to_string(),
        );
        render_canvas(&mut lines[..])
    }

    #[test]
    fn test_sh_renderer() {
        let theme = &THEME_DARK;
        let style = Style::new(theme.text_base, theme.bg_prompt);
        let ellipsis_style = Style::new(theme.text_subtle, theme.bg_prompt);

        // Renders stdout with the same background and padding as a prompt.
        assert_eq!(
            render(14, &json!({ "stdout": "hello\n", "stderr": "", "return_code": 0 })),
            format!(
                "{style}              \n{style}  hello       \n{style}              "
            )
        );

        // stderr is not rendered.
        let with_stderr =
            json!({ "stdout": "hello\n", "stderr": "boom\n", "return_code": 1 });
        let without_stderr =
            json!({ "stdout": "hello\n", "stderr": "", "return_code": 0 });
        assert_eq!(render(14, &with_stderr), render(14, &without_stderr));

        // Empty stdout renders nothing.
        assert_eq!(render(14, &json!({ "stdout": "" })), "");

        // Long lines are wrapped naively.
        assert_eq!(
            render(14, &json!({ "stdout": "abcdefghijklmnopq\n" })),
            format!(
                "{style}              \n{style}  abcdefghij  \n{style}  klmnopq     \n{style}              "
            )
        );

        // Outputs with more than 11 lines are truncated to the first five,
        // an ellipsis, and the last five.
        let stdout = (1..=13)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let expected = vec![
            format!("{style}              "),
            format!("{style}  line1       "),
            format!("{style}  line2       "),
            format!("{style}  line3       "),
            format!("{style}  line4       "),
            format!("{style}  line5       "),
            format!("{ellipsis_style}  ...         "),
            format!("{style}  line9       "),
            format!("{style}  line10      "),
            format!("{style}  line11      "),
            format!("{style}  line12      "),
            format!("{style}  line13      "),
            format!("{style}              "),
        ];
        assert_eq!(render(14, &json!({ "stdout": stdout })), expected.join("\n"));

        // Exactly 11 lines are not truncated.
        let stdout = (1..=11)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            render(14, &json!({ "stdout": stdout })).lines().count(),
            13
        );
    }
}