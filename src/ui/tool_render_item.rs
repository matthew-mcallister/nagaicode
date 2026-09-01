//! Machinery for rendering tool outputs that allows each tool to define its
//! own rendering logic.

use fnv::FnvHashMap;
use serde_json::{Value, json};

use crate::error::AnyResult;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::tools::sh::ShRenderItemBuilder;
use crate::ui::markdown::ResumePoint;
use crate::ui::render_item::RenderItem;
use crate::ui::style::{Style, Theme};
use crate::ui::styled_string::StyledString;
use crate::ui::text::wrap_line_naive;

/// Implementor is able to render a class of tool call outputs that share a
/// common input/output format.
pub trait ToolRenderItemBuilder: std::fmt::Debug {
    /// Builds renderable output
    fn build_render_item(
        &self,
        name: &str,
        args: &Value,
        output: &Value,
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
    pub fn build_render_item(&self, name: &str, args: &Value, output: &Value) -> AnyResult<Box<dyn RenderItem>> {
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
        _output: &Value,
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
        output: &Value,
    ) -> AnyResult<String> {
        let item = tools.build_render_item(name, args, output)?;
        let (mut lines, _) = item.render(&THEME_DARK, width, Default::default());
        Ok(render_canvas(&mut lines[..]))
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

        let output = json!({"stdout": "ignored", "stderr": "", "return_code": 0});

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