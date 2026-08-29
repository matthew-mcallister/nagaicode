//! Machinery for rendering tool outputs that allows each tool to define its
//! own rendering logic.

use fnv::FnvHashMap;
use serde_json::Value;

use crate::error::AnyResult;
use crate::tools::ToolResult;
use crate::ui::markdown::ResumePoint;
use crate::ui::history_item_content::RenderItem;
use crate::ui::style::{Style, Theme};
use crate::ui::styled_string::StyledString;
use crate::ui::text::{Row, SPACES, wrap_line_naive};

/// Implementor is able to render a class of tool call outputs that share a
/// common input/output format.
pub trait ToolRenderItemBuilder: std::fmt::Debug {
    /// Builds renderable output
    fn build_render_item(
        &self,
        name: &str,
        args: &Value,
        output: &ToolResult,
    ) -> AnyResult<Box<dyn RenderItem>>;
}

/// Renders tool calls by name.
#[derive(Debug)]
pub struct ToolRenderer {
    renderers: FnvHashMap<String, Box<dyn ToolRenderItemBuilder>>,
}

impl ToolRenderer {
    pub fn new() -> Self {
        Self {
            renderers: Default::default(),
        }
    }

    /// Renders a tool call by name, args, and output.
    ///
    /// `name` is the name of the renderer to use, not necessarily the name
    /// of the tool.
    pub fn build_render_item(&self, name: &str, args: &Value, output: &ToolResult) -> AnyResult<Box<dyn RenderItem>> {
        if let Some(renderer) = self.renderers.get(name) {
            renderer.build_render_item(name, args, output)
        } else {
            // FIXME: Default renderer, should display "Called tool '{name}'"
            todo!()
        }
    }

    pub fn register(&mut self, name: impl Into<String>, renderer: Box<dyn ToolRenderItemBuilder>) {
        self.renderers.insert(name.into(), renderer);
    }
}

// In the future may support custom tool renderers.
pub fn load_tool_renderers() -> ToolRenderer {
    let mut renderer = ToolRenderer::new();
    renderer.register("sh", Box::new(ShRenderItemBuilder));
    renderer
}

/// Builds RenderItem for shell calls
#[derive(Debug)]
pub struct ShRenderItemBuilder;

/// Renders shell call output
#[derive(Debug)]
pub struct ShRenderItem {
    cmd_line: String,
    stdout: String,
}

impl ToolRenderItemBuilder for ShRenderItemBuilder {
    fn build_render_item(
        &self,
        _name: &str,
        args: &Value,
        output: &ToolResult,
    ) -> AnyResult<Box<dyn RenderItem>> {
        let cmd_line = args.get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("invalid tool input"))?
            .to_owned();
        let stdout: String = output.as_json()
            .and_then(|v| Some(v.as_object()?.get("stdout")?.as_str()?.to_owned()))
            .ok_or_else(|| anyhow::anyhow!("invalid tool output"))?;
        Ok(Box::new(ShRenderItem {
            cmd_line,
            stdout,
        }))
    }
}

impl RenderItem for ShRenderItem {
    fn render(
        &self,
        theme: &Theme,
        width: usize,
        _resume_point: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint) {
        let rows = render_sh_stdout(
            theme,
            width,
            &self.cmd_line,
            &self.stdout,
        );
        (rows, Default::default())
    }
}

const PADDING: usize = 2;
const MAX_LINES: usize = 11;

fn render_sh_stdout(
    theme: &Theme,
    width: usize,
    cmd_line: &str,
    stdout: &str,
) -> Vec<StyledString> {
    if stdout.is_empty() {
        return Vec::new();
    }

    if width < 8 {
        return vec![];
    }

    let style = Style::new(theme.text_base, theme.bg_prompt);
    let ellipsis_style = Style::new(theme.text_subtle, theme.bg_prompt);

    let mut padding = StyledString::new(style, width);
    padding.push(&SPACES[..width], width);

    let lead = (MAX_LINES - 1) / 2;
    let inner_width = width - 2 * PADDING;

    let mut rows = vec![padding.clone()];

    let push_rows = |rows: &mut Vec<StyledString>, new_rows: &[Row]| {
        for row in new_rows {
            let mut s = StyledString::new(style, 2 * row.graphemes.len());
            s.push(&SPACES[..PADDING], PADDING);
            for g in &row.graphemes {
                s.push(g.formatted(), g.width as usize);
            }
            s.push(&SPACES[..PADDING], PADDING);
            rows.push(s);
        }
    };

    // Command line
    let mut c = "$ ".to_owned();
    c.extend(cmd_line.lines());
    push_rows(&mut rows, &wrap_line_naive(inner_width, cmd_line));

    // Output
    let lines: Vec<&str> = stdout.lines().collect();
    if lines.len() > MAX_LINES {
        // Ellipsized
        for line in &lines[..lead] {
            push_rows(&mut rows, &wrap_line_naive(inner_width, line));
        }
        {
            let mut s = StyledString::new(ellipsis_style, inner_width);
            s.push("...", inner_width);
            s.push(&SPACES[..inner_width - 3], inner_width);
            rows.push(s);
        }
        for line in &lines[lines.len() - lead..] {
            push_rows(&mut rows, &wrap_line_naive(inner_width, line));
        }
    } else {
        // Full output
        for line in lines {
            push_rows(&mut rows, &wrap_line_naive(inner_width, line));
        }
    }

    rows.push(padding);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::canvas::render_canvas;
    use crate::ui::style::THEME_DARK;

    fn render(
        tools: &ToolRenderer,
        width: usize,
        name: &str,
        args: &Value,
        output: &ToolResult,
    ) -> AnyResult<String> {
        let item = tools.build_render_item(name, args, output)?;
        let (mut lines, _) = item.render(&THEME_DARK, width, Default::default());
        Ok(render_canvas(&mut lines[..]))
    }

    #[test]
    fn test_sh_renderer() {
        todo!()
    }
}