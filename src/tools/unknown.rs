use std::borrow::Cow;

use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::tools::Tool;
use crate::ui::render_item::{HelpRenderItem, RenderItem};

/// Placeholder for tool calls whose output could not be parsed, e.g. when a
/// tool was removed but calls still exist in old sessions.
#[derive(Debug)]
pub struct UnknownTool {
    tool_name: String,
    input_schema: Value,
}

impl UnknownTool {
    /// Creates a fallback for an unparseable tool call.
    pub fn new(tool_name: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            input_schema: json!({ "type": "object" }),
        }
    }

    /// Builds JSON for persistence to the DB.
    pub fn output(&self) -> Value {
        json!({
            "tool_name": self.tool_name,
            "error": "could not parse output",
        })
    }
}

impl DataQuery for UnknownTool {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "type": "unknown",
                "name": self.name(),
                "tool_name": self.tool_name,
                "error": "could not parse output",
            }))),
            "type" => Ok(QueryField::Value(json!("unknown"))),
            "name" => Ok(QueryField::Value(self.name().into())),
            "tool_name" => Ok(QueryField::Value(self.tool_name.clone().into())),
            "error" => Ok(QueryField::Value(json!("could not parse output"))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

impl Tool for UnknownTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        unreachable!("UnknownTool is never invoked")
    }

    fn is_visible(&self) -> bool {
        false
    }

    fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    fn call<'a>(&'a self, _input: &'a Value) -> BoxFuture<'a, AnyResult<Value>> {
        Box::pin(async { unreachable!("UnknownTool is never invoked") })
    }

    fn render_to_ui(&self, _input: &Value, _output: &Value) -> AnyResult<Box<dyn RenderItem>> {
        Ok(Box::new(HelpRenderItem::new(format!(
            "Called '{}'",
            self.tool_name
        ))))
    }

    fn render_to_interface<'a>(
        &self,
        _input: &'a Value,
        _output: &'a Value,
    ) -> AnyResult<Vec<ToolOutputContent<'a>>> {
        Ok(vec![ToolOutputContent::Text {
            text: Cow::Owned("error: could not parse output".into()),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::QueryError;
    use crate::ui::canvas::render_canvas;
    use crate::ui::render_item::render_help;
    use crate::ui::style::THEME_DARK;

    #[test]
    fn test_unknown_tool() {
        let tool = UnknownTool::new("sh");
        assert_eq!(tool.name(), "sh");
        assert!(!tool.is_visible());

        let expected = json!({ "tool_name": "sh", "error": "could not parse output" });
        assert_eq!(tool.output(), expected);
        assert_eq!(
            tool.query("/").unwrap(),
            json!({
                "type": "unknown",
                "name": "sh",
                "tool_name": "sh",
                "error": "could not parse output",
            })
        );
        assert_eq!(tool.query("/type").unwrap(), json!("unknown"));
        assert_eq!(tool.query("/name").unwrap(), json!("sh"));
        assert_eq!(tool.query("/tool_name").unwrap(), json!("sh"));
        assert_eq!(tool.query("/error").unwrap(), json!("could not parse output"));
        assert!(matches!(tool.query("/missing"), Err(QueryError::InvalidField(_))));

        let input = json!({});
        let content = tool.render_to_interface(&input, &input).unwrap();
        assert_eq!(
            content,
            vec![
                ToolOutputContent::Text { text: Cow::Owned("error: could not parse output".into()) }
            ]
        );

        let item = tool.render_to_ui(&input, &input).unwrap();
        assert_eq!(item.query("/type").unwrap(), json!("help"));
        let (mut lines, _) = item.render(&THEME_DARK, 20, Default::default());
        let mut expected = render_help(&THEME_DARK, 20, "Called 'sh'");
        assert_eq!(render_canvas(&mut lines[..]), render_canvas(&mut expected[..]));
    }
}