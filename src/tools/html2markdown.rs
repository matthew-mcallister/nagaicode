use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use anyhow::anyhow;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::config::Dirs;
use crate::cwd::Cwd;
use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::tools::{InterfaceToolOutput, Tool, ToolCallContext, ToolCreateArgs};
use crate::ui::render_item::{RenderItem, SystemRenderItem};

/// Converts HTML files to Markdown.
#[derive(Debug)]
pub struct Html2MarkdownTool {
    cwd: Arc<Cwd>,
    dirs: Arc<Dirs>,
    input_schema: Value,
}

impl Html2MarkdownTool {
    /// Creates a tool that converts HTML files into session storage.
    pub fn new(args: &ToolCreateArgs<'_>) -> Self {
        Self {
            cwd: Arc::clone(args.cwd),
            dirs: Arc::clone(args.dirs),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "filename": { "type": "string" },
                },
                "required": ["path", "filename"],
                "additionalProperties": false,
            }),
        }
    }
}

impl DataQuery for Html2MarkdownTool {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "name": self.name(),
                "is_visible": self.is_visible(),
            }))),
            "name" => Ok(QueryField::Value(json!(self.name()))),
            "is_visible" => Ok(QueryField::Value(json!(self.is_visible()))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

impl Tool for Html2MarkdownTool {
    fn name(&self) -> &str {
        "html2markdown"
    }

    fn description(&self) -> &str {
        "Convert an HTML file to Markdown and save to session storage."
    }

    fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    fn call<'a>(
        &'a self,
        context: &'a ToolCallContext,
        input: &'a Value,
    ) -> BoxFuture<'a, AnyResult<Value>> {
        Box::pin(async move {
            let invalid = || anyhow!("arguments don't match schema");
            let filepath = input
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let filename = input
                .get("filename")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;

            // TODO: support sandboxed relative filepaths
            if !is_valid_filename(filename) {
                return Err(anyhow!("invalid filename: {filename}"));
            }

            let path = self.cwd.join(filepath);
            let html = std::fs::read_to_string(&path)?;
            let markdown = html_to_markdown_rs::convert(&html, None)
                .map_err(|e| anyhow!("{filepath}: {e}"))?
                .content
                .ok_or_else(|| anyhow!("{filepath}: conversion produced no output"))?;

            let dir = self.dirs.session_storage(context.session_id);
            std::fs::create_dir_all(&dir)?;
            let out_path = dir.join(filename);
            std::fs::write(&out_path, &markdown)?;
            Ok(json!(out_path.display().to_string()))
        })
    }

    fn render_to_ui(&self, input: &Value, _output: &Value) -> AnyResult<Box<dyn RenderItem>> {
        let path = input
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool input"))?;
        Ok(Box::new(SystemRenderItem::new(format!("Converted {path}"))))
    }

    fn render_to_interface(
        &self,
        _input: &Value,
        output: &Value,
    ) -> AnyResult<InterfaceToolOutput> {
        let filepath = output
            .as_str()
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        Ok(InterfaceToolOutput {
            content: vec![ToolOutputContent::Text {
                text: Cow::Owned(filepath.to_owned()),
            }],
        })
    }
}

/// Whether `name` is a single, normal path component.
fn is_valid_filename(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
}
