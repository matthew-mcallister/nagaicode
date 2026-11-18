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
use crate::json_rpc::{ErrorStd, ResponseBody};
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
    // TODO: optional automatic tool namespacing
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
        info: ToolCallInfo<'a>,
    ) -> BoxFuture<'a, AnyResult<Value>> {
        Box::pin(async move {
            let params = json!({
                "name": self.def.name,
                "arguments": info.input,
            });
            match self.server.request("tools/call", params).await? {
                ResponseBody::Result(result) => Ok(result),
                ResponseBody::Error(e) => Err(ErrorStd(e).into()),
            }
        })
    }
}

impl DataQuery for McpTool {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "renderer" => Ok(QueryField::Value(json!(self.renderer()))),
            "def" => Ok(QueryField::DataQuery(&self.def)),
            "" => {
                let mut value = self.def.query("")?;
                value["renderer"] = self.query("/renderer")?;
                value["def"] = self.query("/def")?;
                Ok(QueryField::Value(value))
            }
            field => self.def.query_field(field),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;
    use std::time::Duration;

    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

    use super::*;
    use crate::config::{McpServerConfig, McpServerConfigType};
    use crate::json_rpc::{Error as JsonRpcError, Request as JsonRpcRequest, ResponseBody};
    use crate::mcp::mock::MockTransport;
    use crate::mcp::transport::TransportError;

    fn mock_server() -> (
        McpServer,
        UnboundedSender<Result<ResponseBody, TransportError>>,
        UnboundedReceiver<JsonRpcRequest>,
    ) {
        let (request_sender, request_receiver) = unbounded_channel();
        let (response_sender, response_receiver) = unbounded_channel();
        let config = McpServerConfig {
            name: "mock".to_owned(),
            type_: McpServerConfigType::Stdio,
            enabled: true,
            command: Vec::new(),
            include: None,
            exclude: Vec::new(),
        };
        let server = McpServer {
            config,
            transport: Box::new(MockTransport::new(response_receiver, request_sender)),
            request_id: AtomicU64::new(0),
            timeout: Duration::from_millis(100),
        };
        (server, response_sender, request_receiver)
    }

    fn mock_tool(server: McpServer) -> McpTool {
        McpTool::new(
            Arc::new(server),
            McpToolDef {
                name: "echo".to_owned(),
                title: None,
                description: None,
                input_schema: json!({ "type": "object" }),
                output_schema: None,
            },
        )
    }

    #[tokio::test]
    async fn test_tool_call() {
        let (server, responses, mut requests) = mock_server();
        let tool = mock_tool(server);
        let input = json!({ "text": "hello" });
        let result = json!({
            "content": [{ "type": "text", "text": "hello" }],
            "isError": false,
        });
        responses.send(Ok(ResponseBody::Result(result.clone()))).unwrap();
        let output = tool
            .call(ToolCallInfo { session_id: 1, tool_name: "echo", input: &input })
            .await
            .unwrap();
        assert_eq!(output, result);
        let request = requests.recv().await.unwrap();
        assert_eq!(request.method, "tools/call");
        assert_eq!(
            request.params,
            json!({ "name": "echo", "arguments": { "text": "hello" } })
        );
        assert!(request.id.is_some());
    }

    #[tokio::test]
    async fn test_tool_call_error() {
        let (server, responses, mut requests) = mock_server();
        let tool = mock_tool(server);
        let input = json!({});
        responses
            .send(Ok(ResponseBody::Error(JsonRpcError {
                code: -32602,
                message: "bad params".to_owned(),
                data: None,
            })))
            .unwrap();
        let error = tool
            .call(ToolCallInfo { session_id: 1, tool_name: "echo", input: &input })
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "code -32602: bad params");
        let request = requests.recv().await.unwrap();
        assert_eq!(request.method, "tools/call");
        assert_eq!(request.params, json!({ "name": "echo", "arguments": {} }));
    }

    #[tokio::test]
    async fn test_server_request() {
        let (server, responses, mut requests) = mock_server();

        responses.send(Ok(ResponseBody::Result(json!({ "tools": [] })))).unwrap();
        assert_eq!(
            server.request("tools/list", json!({})).await.unwrap(),
            ResponseBody::Result(json!({ "tools": [] }))
        );
        let request = requests.recv().await.unwrap();
        assert_eq!(request.method, "tools/list");
        assert_eq!(request.params, json!({}));

        let response = ResponseBody::Error(JsonRpcError {
            code: -32602,
            message: "bad params".to_owned(),
            data: None,
        });
        responses.send(Ok(response.clone())).unwrap();
        assert_eq!(
            server.request("tools/call", json!({ "name": "echo" })).await.unwrap(),
            response,
        );
        let request = requests.recv().await.unwrap();
        assert_eq!(request.method, "tools/call");
        assert_eq!(request.params, json!({ "name": "echo" }));
    }

    #[tokio::test]
    async fn test_server_timeout() {
        // No responses on channel -> timeout
        let (server, responses, mut requests) = mock_server();
        assert_eq!(
            server.request("tools/list", json!({})).await.unwrap_err(),
            TransportError::Timeout,
        );
        let request = requests.recv().await.unwrap();
        assert_eq!(request.method, "tools/list");

        // Channel closed -> unrecoverable
        std::mem::drop(responses);
        assert_eq!(
            server.request("tools/list", json!({})).await.unwrap_err(),
            TransportError::Unrecoverable,
        );
        let request = requests.recv().await.unwrap();
        assert_eq!(request.method, "tools/list");
    }

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
