use std::borrow::Cow;

use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::tools::Tool;
use crate::ui::render_item::{ErrorRenderItem, RenderItem};

/// Placeholder for tool calls that could not be executed, e.g. due to an
/// invalid tool name or arguments.
#[derive(Debug)]
pub struct FailedTool {
    tool_name: String,
    message: String,
    input_schema: Value,
}

impl FailedTool {
    /// Creates a fallback for a failed tool call.
    pub fn new(tool_name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            message: message.into(),
            input_schema: json!({ "type": "object" }),
        }
    }

    /// Builds JSON for persistence to the DB
    pub fn output(&self) -> Value {
        json!({
            "tool_name": self.tool_name,
            "error": self.message,
        })
    }
}

impl DataQuery for FailedTool {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "type": "failed",
                "name": self.name(),
                "tool_name": self.tool_name,
                "error": self.message,
            }))),
            "type" => Ok(QueryField::Value(json!("failed"))),
            "name" => Ok(QueryField::Value(self.name().into())),
            "tool_name" => Ok(QueryField::Value(self.tool_name.clone().into())),
            "error" => Ok(QueryField::Value(self.message.clone().into())),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

impl Tool for FailedTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        unreachable!("FailedTool is never invoked")
    }

    fn is_visible(&self) -> bool {
        false
    }

    fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    fn call<'a>(&'a self, _input: &'a Value) -> BoxFuture<'a, AnyResult<Value>> {
        Box::pin(async { unreachable!("FailedTool is never invoked") })
    }

    fn render_to_ui(&self, _input: &Value, _output: &Value) -> AnyResult<Box<dyn RenderItem>> {
        Ok(Box::new(ErrorRenderItem::new(format!(
            "Called '{}': {}",
            self.tool_name, self.message
        ))))
    }

    fn render_to_interface<'a>(
        &self,
        _input: &'a Value,
        _output: &'a Value,
    ) -> AnyResult<Vec<ToolOutputContent<'a>>> {
        Ok(vec![ToolOutputContent::Text {
            text: Cow::Owned(format!("error: {}", self.message)),
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::QueryError;
    use crate::ui::canvas::render_canvas;
    use crate::ui::render_item::render_error;
    use crate::ui::style::THEME_DARK;

    #[test]
    fn test_failed_tool() {
        let tool = FailedTool::new("sh", "invalid arguments");
        assert_eq!(tool.name(), "sh");
        assert!(!tool.is_visible());

        let expected = json!({ "tool_name": "sh", "error": "invalid arguments" });
        assert_eq!(tool.output(), expected);
        assert_eq!(
            tool.query("/").unwrap(),
            json!({
                "type": "failed",
                "name": "sh",
                "tool_name": "sh",
                "error": "invalid arguments",
            })
        );
        assert_eq!(tool.query("/type").unwrap(), json!("failed"));
        assert_eq!(tool.query("/name").unwrap(), json!("sh"));
        assert_eq!(tool.query("/tool_name").unwrap(), json!("sh"));
        assert_eq!(tool.query("/error").unwrap(), json!("invalid arguments"));
        assert!(matches!(tool.query("/missing"), Err(QueryError::InvalidField(_))));

        let input = json!({});
        let content = tool.render_to_interface(&input, &input).unwrap();
        assert_eq!(
            content,
            vec![
                ToolOutputContent::Text { text: Cow::Owned("error: invalid arguments".into()) }
            ]
        );

        let item = tool.render_to_ui(&input, &input).unwrap();
        assert_eq!(item.query("/type").unwrap(), json!("error"));
        let (mut lines, _) = item.render(&THEME_DARK, 20, Default::default());
        let mut expected = render_error(&THEME_DARK, 20, "Called 'sh': invalid arguments");
        assert_eq!(render_canvas(&mut lines[..]), render_canvas(&mut expected[..]));
    }
}