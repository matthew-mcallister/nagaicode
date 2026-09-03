use std::borrow::Cow;

use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::session::DbItem;
use crate::tools::{InterfaceToolOutput, Tool};
use crate::ui::render_item::{ErrorRenderItem, RenderItem};

/// Placeholder for tool calls that could not be executed, e.g. due to an
/// invalid tool name or arguments.
#[derive(Debug)]
pub struct FailedTool {
    input_schema: Value,
}

impl FailedTool {
    /// Creates a fallback for a failed tool call.
    pub fn new() -> Self {
        Self {
            input_schema: json!({ "type": "object" }),
        }
    }

    /// Returns the error message parsed from `output`, falling back to a generic
    /// message.
    pub fn get_message(output: &Value) -> &str {
        output.get("error").and_then(Value::as_str).unwrap_or("unknown error")
    }

    /// Writes failure output for a failed tool call, falling back to a generic
    /// message when `message` is empty. The item is renamed to the failed tool
    /// so its output renders as an error instead of as `tool_name`'s output.
    pub fn write_failure(item: &mut DbItem, tool_name: &str, message: &str) {
        let message = if message.is_empty() { "unknown error" } else { message };
        item.text = Some("failed".to_string());
        let output = json!({
            "tool_name": tool_name,
            "error": message,
        });
        item.tool_output = Some(output.to_string());
    }
}

impl DataQuery for FailedTool {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "name": self.name(),
                "is_visible": self.is_visible(),
            }))),
            "name" => Ok(QueryField::Value(self.name().into())),
            "is_visible" => Ok(QueryField::Value(json!(self.is_visible()))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

impl Tool for FailedTool {
    fn name(&self) -> &str {
        "failed"
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

    fn render_to_ui(&self, _input: &Value, output: &Value) -> AnyResult<Box<dyn RenderItem>> {
        let tool_name = output.get("tool_name").and_then(Value::as_str).unwrap_or("failed");
        let message = Self::get_message(output);
        Ok(Box::new(ErrorRenderItem::new(format!(
            "Called '{}': {}",
            tool_name, message
        ))))
    }

    fn render_to_interface(
        &self,
        _input: &Value,
        output: &Value,
    ) -> AnyResult<InterfaceToolOutput> {
        let tool_name = output.get("tool_name").and_then(Value::as_str).unwrap_or("failed");
        let message = Self::get_message(output);
        Ok(InterfaceToolOutput {
            name: tool_name.to_owned(),
            content: vec![ToolOutputContent::Text {
                text: Cow::Owned(format!("error: {message}")),
            }],
        })
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
        let tool = FailedTool::new();
        assert_eq!(tool.name(), "failed");
        assert!(!tool.is_visible());

        let output = json!({ "tool_name": "sh", "error": "invalid arguments" });
        let mut conn = crate::db::open_new().expect("open db");
        let session = crate::session::Session::create(&mut conn, "Session").expect("create session");
        let turn = crate::session::Turn::create(
            &mut conn,
            session.id,
            crate::session::TurnType::Assistant,
            None,
            None,
            None,
        )
        .expect("create turn");
        let mut item = crate::session::DbItem::create(
            &mut conn,
            crate::session::NewItem {
                session_id: Some(session.id),
                turn_id: Some(turn.id),
                ty: Some(crate::session::ItemType::ToolCall),
                text: Some("sh"),
                ..Default::default()
            },
        )
        .expect("create item");
        FailedTool::write_failure(&mut item, "sh", "invalid arguments");
        assert_eq!(item.text.as_deref(), Some("failed"));
        assert_eq!(item.tool_output, Some(output.to_string()));

        let input = json!({});
        let result = tool.render_to_interface(&input, &output).unwrap();
        assert_eq!(result.name, "sh");
        assert_eq!(
            result.content,
            vec![
                ToolOutputContent::Text { text: Cow::Owned("error: invalid arguments".into()) }
            ]
        );

        let item = tool.render_to_ui(&input, &output).unwrap();
        assert_eq!(item.query("/type").unwrap(), json!("error"));
        let (mut lines, _) = item.render(&THEME_DARK, 20, Default::default());
        let mut expected = render_error(&THEME_DARK, 20, "Called 'sh': invalid arguments");
        assert_eq!(render_canvas(&mut lines[..]), render_canvas(&mut expected[..]));
    }

    #[test]
    fn test_query() {
        let tool = FailedTool::new();
        assert_eq!(tool.query("/").unwrap(), json!({ "name": "failed", "is_visible": false }));
        assert_eq!(tool.query("/name").unwrap(), json!("failed"));
        assert_eq!(tool.query("/is_visible").unwrap(), json!(false));
        assert!(matches!(tool.query("/missing"), Err(QueryError::InvalidField(_))));
    }
}