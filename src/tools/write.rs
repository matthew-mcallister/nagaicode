use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use anyhow::anyhow;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::cwd::Cwd;
use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::item::ToolRenderInfo;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::tools::{InterfaceToolOutput, Tool, ToolCallContext, ToolCreateArgs, ToolRenderer};
use crate::ui::render_item::{SystemRenderItem, RenderItem};

/// Creates or overwrites a text file with the given content.
#[derive(Debug)]
pub struct WriteTool {
    cwd: Arc<Cwd>,
    renderer: WriteRenderer,
    input_schema: Value,
}

impl WriteTool {
    /// Creates a tool that writes text files, rendering paths relative to
    /// `cwd`.
    pub fn new(args: &ToolCreateArgs<'_>) -> Self {
        Self {
            cwd: Arc::clone(args.cwd),
            renderer: WriteRenderer::new(args),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "filepath": { "type": "string" },
                    "content": { "type": "string" },
                },
                "required": ["filepath", "content"],
                "additionalProperties": false,
            }),
        }
    }
}

impl DataQuery for WriteTool {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "name": self.name(),
                "is_visible": self.is_visible(),
                "renderer": self.query("/renderer")?,
            }))),
            "name" => Ok(QueryField::Value(json!(self.name()))),
            "is_visible" => Ok(QueryField::Value(json!(self.is_visible()))),
            "renderer" => Ok(QueryField::DataQuery(self.renderer())),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Creates or overwrites a text file with the given content."
    }

    fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    fn renderer(&self) -> &dyn ToolRenderer {
        &self.renderer
    }

    fn call<'a>(
        &'a self,
        _context: &'a ToolCallContext,
        input: &'a Value,
    ) -> BoxFuture<'a, AnyResult<Value>> {
        Box::pin(async move {
            let invalid = || anyhow!("arguments don't match schema");
            let filepath = input
                .get("filepath")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let content = input
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;

            let path = self.cwd.join(filepath);
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            let created = !path.exists();
            std::fs::write(&path, content)?;

            Ok(json!({ "created": created, "num_bytes": content.len() }))
        })
    }
}

#[derive(Debug)]
pub struct WriteRenderer {
    cwd: Arc<Cwd>,
}

impl WriteRenderer {
    pub fn new(args: &ToolCreateArgs<'_>) -> Self {
        Self { cwd: Arc::clone(args.cwd) }
    }
}

impl DataQuery for WriteRenderer {
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

impl ToolRenderer for WriteRenderer {
    fn name(&self) -> &str {
        "write"
    }

    fn render_to_ui(&self, info: &ToolRenderInfo) -> AnyResult<Box<dyn RenderItem>> {
        let input = info.args;
        let output = info.output;
        let filepath = input
            .get("filepath")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool input"))?;
        let created = output
            .get("created")
            .and_then(Value::as_bool)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let path = display_path(&self.cwd, Path::new(filepath));
        let message = if created {
            format!("Created {path}")
        } else {
            format!("Overwrote {path}")
        };
        Ok(Box::new(SystemRenderItem::new(message)))
    }

    fn render_to_model(&self, info: &ToolRenderInfo) -> AnyResult<InterfaceToolOutput> {
        let input = info.args;
        let output = info.output;
        let filepath = input
            .get("filepath")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool input"))?;
        let created = output
            .get("created")
            .and_then(Value::as_bool)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let num_bytes = output
            .get("num_bytes")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let verb = if created { "created" } else { "overwrote" };
        Ok(InterfaceToolOutput {
            content: vec![ToolOutputContent::Text {
                text: Cow::Owned(format!("{verb} {filepath}\nwrote {num_bytes} bytes")),
            }],
        })
    }
}

fn display_path(cwd: &Path, path: &Path) -> String {
    path.strip_prefix(cwd).unwrap_or(path).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;

    #[tokio::test]
    async fn test_write() {
        let app = App::new().unwrap();
        let tool = WriteTool::new(&app.tool_create_args());
        let context = ToolCallContext { session_id: 1 };
        let path = app.cwd().path().join("a.txt");

        let out = tool
            .call(&context, &json!({ "filepath": path.to_string_lossy(), "content": "one\ntwo\n" }))
            .await
            .unwrap();
        assert_eq!(out["created"], json!(true));
        assert_eq!(out["num_bytes"], json!(8));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
    }

    #[tokio::test]
    async fn test_overwrite() {
        let app = App::new().unwrap();
        let tool = WriteTool::new(&app.tool_create_args());
        let context = ToolCallContext { session_id: 1 };
        let path = app.cwd().path().join("a.txt");
        std::fs::write(&path, "original").unwrap();

        let out = tool
            .call(&context, &json!({ "filepath": path.to_string_lossy(), "content": "replaced\n" }))
            .await
            .unwrap();
        assert_eq!(out["created"], json!(false));
        assert_eq!(out["num_bytes"], json!(9));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "replaced\n");
    }

    #[tokio::test]
    async fn test_write_creates_parent_dirs() {
        let app = App::new().unwrap();
        let tool = WriteTool::new(&app.tool_create_args());
        let context = ToolCallContext { session_id: 1 };
        let path = app.cwd().path().join("nested").join("dir").join("a.txt");

        let out = tool
            .call(&context, &json!({ "filepath": path.to_string_lossy(), "content": "hello" }))
            .await
            .unwrap();
        assert_eq!(out["created"], json!(true));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    }
}
