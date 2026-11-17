use std::sync::Arc;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::AnyResult;
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

impl ToolRenderer for McpRenderer {
    fn name(&self) -> &str {
        "mcp"
    }

    fn render_to_ui(&self, info: &ToolRenderInfo) -> AnyResult<Box<dyn RenderItem>> {
        Ok(Box::new(SystemRenderItem::new(format!("Called '{}'", info.tool_name))))
    }

    fn render_to_model(&self, _info: &ToolRenderInfo) -> AnyResult<InterfaceToolOutput> {
        todo!()
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
