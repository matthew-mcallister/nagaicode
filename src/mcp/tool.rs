use std::borrow::Cow;
use std::sync::Arc;

use anyhow::anyhow;
use futures::future::BoxFuture;
use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::AnyResult;
use crate::interface::ToolOutputContent as InterfaceToolOutputContent;
use crate::item::ToolRenderInfo;
use crate::mcp::McpServer;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::tools::{InterfaceToolOutput, Tool, ToolCallInfo, ToolRenderer};
use crate::ui::render_item::{RenderItem, SystemRenderItem};

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpToolDef {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}

impl DataQuery for McpToolDef {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "name": self.query("/name")?,
                "title": self.query("/title")?,
                "description": self.query("/description")?,
                "input_schema": self.query("/input_schema")?,
                "output_schema": self.query("/output_schema")?,
            }))),
            "name" => Ok(QueryField::Value(json!(self.name))),
            "title" => Ok(QueryField::Value(json!(self.title))),
            "description" => Ok(QueryField::Value(json!(self.description))),
            "input_schema" => Ok(QueryField::DataQuery(&self.input_schema)),
            "output_schema" => match &self.output_schema {
                Some(schema) => Ok(QueryField::DataQuery(schema)),
                None => Ok(QueryField::Value(Value::Null)),
            },
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[derive(Debug)]
pub(crate) struct McpRenderer;

impl DataQuery for McpRenderer {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "name": self.name(),
            }))),
            "name" => Ok(QueryField::Value(json!(self.name()))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum UnstructuredContent {
    Text {
        text: String,
    },
    Image,
    Audio,
    ResourceLink,
    Resource,
}

fn render_unstructured(content: &[UnstructuredContent]) -> AnyResult<InterfaceToolOutput> {
    let mut output = Vec::with_capacity(content.len());
    for content in content {
        match content {
            UnstructuredContent::Text { text } => {
                output.push(InterfaceToolOutputContent::Text { text: Cow::Owned(text.clone()) });
            }
            UnstructuredContent::Image => warn!("unsupported MCP content type: image"),
            UnstructuredContent::Audio => warn!("unsupported MCP content type: audio"),
            UnstructuredContent::ResourceLink => warn!("unsupported MCP content type: resource_link"),
            UnstructuredContent::Resource => warn!("unsupported MCP content type: resource"),
        }
    }
    Ok(InterfaceToolOutput { content: output })
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpToolResult {
    result_type: Option<String>,
    content: Option<Vec<UnstructuredContent>>,
    structured_content: Option<Value>,
    is_error: Option<bool>,
}

impl ToolRenderer for McpRenderer {
    fn name(&self) -> &str {
        "mcp"
    }

    fn render_to_ui(&self, info: &ToolRenderInfo) -> AnyResult<Box<dyn RenderItem>> {
        Ok(Box::new(SystemRenderItem::new(format!("Called '{}'", info.tool_name))))
    }

    fn render_to_model(&self, info: &ToolRenderInfo) -> AnyResult<InterfaceToolOutput> {
        let result: McpToolResult = serde_json::from_value(info.output.clone())
            .map_err(|e| anyhow!("invalid MCP tool result: {e}"))?;
        if let Some(result_type) = &result.result_type && result_type != "complete" {
            warn!("unexpected MCP result type: {result_type}");
        }
        if let Some(content) = &result.content {
            let output = render_unstructured(content)?;
            if !output.content.is_empty() {
                return Ok(output);
            }
        }
        if let Some(structured) = &result.structured_content {
            return Ok(InterfaceToolOutput {
                content: vec![InterfaceToolOutputContent::Text {
                    text: Cow::Owned(structured.to_string()),
                }],
            });
        }
        Ok(InterfaceToolOutput {
            content: vec![InterfaceToolOutputContent::Text {
                text: Cow::Borrowed("tool call produced no output."),
            }],
        })
    }
}

/// Tool exposed by an MCP server, implemented via JSON-RPC call.
#[derive(Debug)]
pub(crate) struct McpTool {
    server: Arc<McpServer>,
    def: McpToolDef,
}

impl McpTool {
    pub(crate) fn new(server: Arc<McpServer>, def: McpToolDef) -> Self {
        Self { server, def }
    }
}

impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.def.name
    }

    fn description(&self) -> &str {
        self.def.description.as_deref().unwrap_or("")
    }

    fn input_schema(&self) -> &Value {
        &self.def.input_schema
    }

    fn renderer(&self) -> &str {
        "mcp"
    }

    fn call<'a>(
        &'a self,
        _info: ToolCallInfo<'a>,
    ) -> BoxFuture<'a, AnyResult<Value>> {
        todo!()
    }
}

impl DataQuery for McpTool {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "renderer" => Ok(QueryField::Value(json!(self.renderer()))),
            "" => {
                let mut value = self.def.query("")?;
                value["renderer"] = self.query("/renderer")?;
                Ok(QueryField::Value(value))
            }
            field => self.def.query_field(field),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info<'a>(args: &'a Value, output: &'a Value) -> ToolRenderInfo<'a> {
        ToolRenderInfo { tool_name: "mcp__test", args, output }
    }

    fn text(text: &str) -> InterfaceToolOutputContent<'static> {
        InterfaceToolOutputContent::Text { text: Cow::Owned(text.to_owned()) }
    }

    fn render(output: &Value) -> AnyResult<Vec<InterfaceToolOutputContent<'static>>> {
        let args = json!({});
        let info = info(&args, output);
        McpRenderer.render_to_model(&info).map(|output| output.content)
    }

    #[test]
    fn test_render_model_unstructured() {
        let output = json!({
            "resultType": "complete",
            "content": [
                { "type": "text", "text": "Found 2 users" },
                { "type": "resource_link", "uri": "file://a", "name": "a" },
                { "type": "resource", "resource": { "uri": "file://b", "text": "b" } },
                { "type": "audio", "data": "", "mimeType": "audio/wav" },
                { "type": "image", "data": "", "mimeType": "image/png" },
                { "type": "text", "text": "Done." },
            ],
            "structuredContent": { "ignored": true },
        });
        assert_eq!(render(&output).unwrap(), vec![text("Found 2 users"), text("Done.")]);
    }

    #[test]
    fn test_render_model_structured_fallback() {
        let output = json!({
            "resultType": "complete",
            "content": [
                { "type": "resource", "resource": { "uri": "file://b", "text": "b" } },
            ],
            "structuredContent": [
                { "id": "1", "name": "Alice" },
                { "id": "2", "name": "Bob" },
            ],
        });
        assert_eq!(
            render(&output).unwrap(),
            vec![text(r#"[{"id":"1","name":"Alice"},{"id":"2","name":"Bob"}]"#)]
        );

        let output = json!({ "structuredContent": { "id": "1" } });
        assert_eq!(render(&output).unwrap(), vec![text(r#"{"id":"1"}"#)]);
    }

    #[test]
    fn test_render_model_empty() {
        assert_eq!(render(&json!({})).unwrap(), vec![text("tool call produced no output.")]);
        assert_eq!(render(&json!({ "content": [] })).unwrap(), vec![text("tool call produced no output.")]);
        assert_eq!(
            render(&json!({
                "content": [{ "type": "audio", "data": "", "mimeType": "audio/wav" }],
            }))
            .unwrap(),
            vec![text("tool call produced no output.")]
        );
    }

    #[test]
    fn test_render_model_invalid() {
        assert!(render(&json!({ "content": [{ "type": "bogus" }] })).is_err());
        assert!(render(&json!({ "content": "not a list" })).is_err());
        assert!(render(&json!("not an object")).is_err());
    }
}
