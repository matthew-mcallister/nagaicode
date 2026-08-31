//! Machinery for rendering tool outputs that allows each tool to define its
//! own rendering logic.

use fnv::FnvHashMap;
use serde_json::{Value, json};

use crate::error::AnyResult;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::tools::ToolResult;
use crate::ui::markdown::ResumePoint;
use crate::ui::render_item::RenderItem;
use crate::ui::style::{Style, Theme};
use crate::ui::styled_string::StyledString;
use crate::ui::text::{Row, SPACES, ellipsize, wrap_line_naive};

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
            DefaultToolRenderItemBuilder.build_render_item(name, args, output)
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

impl DataQuery for ShRenderItem {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "type": self.query("/type")?,
                "cmd_line": self.query("/cmd_line")?,
                "stdout": self.query("/stdout")?,
            }))),
            "type" => Ok(QueryField::Value(json!("sh"))),
            "cmd_line" => Ok(QueryField::Value(self.cmd_line.clone().into())),
            "stdout" => Ok(QueryField::Value(self.stdout.clone().into())),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
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

/// Builds RenderItem for tools without a dedicated renderer.
#[derive(Debug)]
pub struct DefaultToolRenderItemBuilder;

/// Renders a placeholder for tools without a dedicated renderer.
#[derive(Debug)]
pub struct DefaultToolRenderItem {
    name: String,
}

impl ToolRenderItemBuilder for DefaultToolRenderItemBuilder {
    fn build_render_item(
        &self,
        name: &str,
        _args: &Value,
        _output: &ToolResult,
    ) -> AnyResult<Box<dyn RenderItem>> {
        Ok(Box::new(DefaultToolRenderItem {
            name: name.to_owned(),
        }))
    }
}

impl DataQuery for DefaultToolRenderItem {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "type": "default",
                "name": self.name,
            }))),
            "type" => Ok(QueryField::Value(json!("default"))),
            "name" => Ok(QueryField::Value(self.name.clone().into())),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

impl RenderItem for DefaultToolRenderItem {
    fn render(
        &self,
        theme: &Theme,
        width: usize,
        _resume_point: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint) {
        (render_default_tool(theme, width, &self.name), Default::default())
    }
}

fn render_default_tool(theme: &Theme, width: usize, name: &str) -> Vec<StyledString> {
    if width < 8 {
        return Vec::new();
    }

    let style = Style::new(theme.text_subtle, theme.bg_base);
    let content = format!("Called tool '{name}'");
    wrap_line_naive(width - 4, &content)
        .into_iter()
        .map(|row| {
            let mut s = StyledString::new(style, width + 4);
            s.push("  ", 2);
            s.push(&row.to_padded_string(width - 4), width - 4);
            s.push("  ", 2);
            s
        })
        .collect()
}

const PADDING: usize = 2;
const MAX_ROWS: usize = 11;

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

    let inner_width = width - 2 * PADDING;

    let mut rows = vec![padding.clone()];

    let push_rows = |rows: &mut Vec<StyledString>, new_rows: &[Row]| {
        for row in new_rows {
            let mut s = StyledString::new(style, 2 * row.graphemes.len());
            s.push(&SPACES[..PADDING], PADDING);
            for g in &row.graphemes {
                s.push(g.formatted(), g.width as usize);
            }
            s.pad_to_width(width - PADDING);
            s.push(&SPACES[..PADDING], PADDING);
            rows.push(s);
        }
    };

    // Command line
    let prompt = format!("$ {cmd_line}");
    push_rows(&mut rows, &wrap_line_naive(inner_width, &prompt));

    // Output
    let (head, tail) = ellipsize(inner_width, MAX_ROWS, stdout);
    push_rows(&mut rows, &head);
    if let Some(tail) = tail {
        let mut s = StyledString::new(ellipsis_style, width);
        s.push(&SPACES[..PADDING], PADDING);
        s.push("...", 3);
        s.push(&SPACES[..inner_width - 3], inner_width - 3);
        s.push(&SPACES[..PADDING], PADDING);
        rows.push(s);
        push_rows(&mut rows, &tail);
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
    fn test_sh_render_item_query() {
        let item = ShRenderItem {
            cmd_line: "echo hi".into(),
            stdout: "hello\n".into(),
        };
        let expected = json!({
            "type": "sh",
            "cmd_line": "echo hi",
            "stdout": "hello\n",
        });
        assert_eq!(item.query("/").unwrap(), expected);
        assert_eq!(item.query("/type").unwrap(), json!("sh"));
        assert_eq!(item.query("/cmd_line").unwrap(), json!("echo hi"));
        assert_eq!(item.query("/stdout").unwrap(), json!("hello\n"));
        assert!(matches!(item.query("/missing"), Err(crate::query::QueryError::InvalidField(_))));
    }

    #[test]
    fn test_sh_renderer() {
        let tools = load_tool_renderers();
        let theme = &THEME_DARK;
        let style = Style::new(theme.text_base, theme.bg_prompt);
        let ok_output = || ToolResult::Json(json!({"stdout": "", "stderr": "", "return_code": 0}));

        let output = ToolResult::Json(json!({"stdout": "hello\n", "stderr": "", "return_code": 0}));
        assert_eq!(
            render(&tools, 14, "sh", &json!({"command": "echo hi"}), &output).unwrap(),
            format!(
                "{style}              \n{style}  $ echo hi   \n{style}  hello       \n{style}              "
            )
        );
        let output = ToolResult::Json(json!({"stdout": "hi\n", "stderr": "", "return_code": 0}));
        assert_eq!(
            render(&tools, 14, "sh", &json!({"command": "echo hello world"}), &output).unwrap(),
            format!(
                "{style}              \n{style}  $ echo hel  \n{style}  lo world    \n{style}  hi          \n{style}              "
            )
        );

        // Long stdout is ellipsized; the final row is filled to the full width
        let ellipsis_style = Style::new(theme.text_subtle, theme.bg_prompt);
        let output = ToolResult::Json(json!({"stdout": "x".repeat(120), "stderr": "", "return_code": 0}));
        let line = || format!("{style}  xxxxxxxxxx  ");
        assert_eq!(
            render(&tools, 14, "sh", &json!({"command": "echo hi"}), &output).unwrap(),
            [
                format!("{style}              "),
                format!("{style}  $ echo hi   "),
                line(), line(), line(), line(), line(),
                format!("{ellipsis_style}  ...         "),
                line(), line(), line(), line(), line(),
                format!("{style}              "),
            ].join("\n")
        );

        assert_eq!(
            render(&tools, 14, "sh", &json!({"command": "echo hi"}), &ok_output()).unwrap(),
            ""
        );
        assert_eq!(
            render(&tools, 7, "sh", &json!({"command": "echo hi"}), &output).unwrap(),
            ""
        );

        assert!(render(&tools, 14, "sh", &json!({}), &ok_output()).is_err());
        assert!(render(&tools, 14, "sh", &json!({"command": "echo hi"}), &ToolResult::Json(json!({}))).is_err());
    }

    #[test]
    fn test_default_render_item_query() {
        let item = DefaultToolRenderItem { name: "foo".into() };
        let expected = json!({
            "type": "default",
            "name": "foo",
        });
        assert_eq!(item.query("/").unwrap(), expected);
        assert_eq!(item.query("/type").unwrap(), json!("default"));
        assert_eq!(item.query("/name").unwrap(), json!("foo"));
        assert!(matches!(item.query("/missing"), Err(crate::query::QueryError::InvalidField(_))));
        assert!(item.trailing_padding());
    }

    #[test]
    fn test_default_renderer() {
        let tools = load_tool_renderers();
        let theme = &THEME_DARK;
        let style = Style::new(theme.text_subtle, theme.bg_base);

        let output = ToolResult::Json(json!({"stdout": "ignored", "stderr": "", "return_code": 0}));

        // Single line
        assert_eq!(
            render(&tools, 24, "grep", &json!({"pattern": "foo"}), &output).unwrap(),
            format!("{style}  Called tool 'grep'    ")
        );

        // Wrapped to multiple lines
        assert_eq!(
            render(&tools, 24, "web_search", &json!({}), &output).unwrap(),
            format!("{style}  Called tool 'web_sea  \n{style}  rch'                  ")
        );

        // Too narrow to render
        assert_eq!(
            render(&tools, 7, "grep", &json!({}), &output).unwrap(),
            ""
        );
    }
}