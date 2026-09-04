use std::sync::Arc;

use fnv::FnvHashMap;
use log::warn;
use futures::future::BoxFuture;
use serde_json::Value;

use crate::cwd::Cwd;
use crate::error::AnyResult;
use crate::interface::{ToolInfo, ToolOutputContent};
use crate::item::{Item, ToolCallContent, ToolOutput};
use crate::query::DataQuery;
use crate::session::DbItem;
use crate::tools::failed::{render_failure_to_interface, render_failure_to_ui};
use crate::tools::unknown::{render_unknown_to_interface, render_unknown_to_ui};
use crate::ui::render_item::RenderItem;

pub mod edit;
pub mod failed;
pub mod read;
pub mod sh;
pub mod unknown;

/// Interfaces for interacting with tools and rendering their output.
pub trait Tool: std::fmt::Debug + DataQuery + Send + Sync {
    /// Name provided to the agent and persisted to the database. Must be
    /// unique.
    fn name(&self) -> &str;

    /// Description provided to the agent.
    fn description(&self) -> &str;

    /// Whether the tool is advertised to the model.
    fn is_visible(&self) -> bool {
        true
    }

    /// JSON schema used by the model to generate tool calls. Must obey
    /// "strict" rules: all inputs required, no additional fields allowed.
    fn input_schema(&self) -> &Value;

    /// Invokes the tool on input.
    fn call<'a>(&'a self, input: &'a Value) -> BoxFuture<'a, AnyResult<Value>>;

    /// Builds a renderable item from the tool's output.
    fn render_to_ui(&self, input: &Value, output: &Value) -> AnyResult<Box<dyn RenderItem>>;

    /// Builds inference call inputs from the tool's output.
    fn render_to_interface(&self, input: &Value, output: &Value) -> AnyResult<InterfaceToolOutput>;

    /// Describes the tool for the inference API.
    fn tool_info(&self) -> ToolInfo {
        ToolInfo {
            name: self.name().to_owned(),
            description: self.description().to_owned(),
            input_schema: self.input_schema().clone(),
        }
    }
}

/// Tool call representation submitted to inference API.
#[derive(Debug)]
pub struct InterfaceToolOutput {
    pub content: Vec<ToolOutputContent<'static>>,
}

/// Maps tool names to tools.
#[derive(Debug)]
pub struct ToolRegistry {
    tools: FnvHashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new(cwd: &Arc<Cwd>) -> Self {
        // Builtin tools
        // - sh
        // - read
        // - write
        // - edit
        // - grep
        // - glob
        // - failed
        // - unknown
        let mut tools: FnvHashMap<String, Box<dyn Tool>> = FnvHashMap::default();
        let sh = sh::ShTool::new(Arc::clone(cwd));
        tools.insert(sh.name().to_owned(), Box::new(sh));
        let read = read::ReadTool::new(Arc::clone(cwd));
        tools.insert(read.name().to_owned(), Box::new(read));
        let edit = edit::EditTool::new(Arc::clone(cwd));
        tools.insert(edit.name().to_owned(), Box::new(edit));
        Self { tools }
    }

    /// Lists tools available to the model.
    pub fn list_tools(&self) -> impl Iterator<Item = &dyn Tool> {
        self.tools.values().map(|t| t.as_ref()).filter(|t| t.is_visible())
    }

    /// Describes every tool available to the model for the inference API.
    pub fn list_tool_infos(&self) -> Vec<ToolInfo> {
        self.list_tools().map(|tool| tool.tool_info()).collect()
    }

    /// Invokes the tool named by a tool call item. Handles all possible
    /// errors. Makes no DB queries.
    pub async fn call(&self, tool_name: &str, input: &Value) -> ToolOutput {
        let Some(tool) = self.tools.get(tool_name) else {
            return ToolOutput::Failed { error: format!("no such tool '{}'", tool_name) };
        };
        match tool.call(input).await {
            Ok(value) => ToolOutput::Completed { value },
            Err(e) => ToolOutput::Failed { error: e.to_string() },
        }
    }

    /// Builds a render item to display a tool call. Handles all possible
    /// errors. Returns `None` if the tool call has no output.
    pub fn render_to_ui(&self, content: &ToolCallContent) -> Option<Box<dyn RenderItem>> {
        let tool_name = &content.tool_name;
        Some(match content.output.as_ref()? {
            ToolOutput::Completed { value } => self.tools.get(tool_name)
                .and_then(|tool| tool.render_to_ui(&content.args, value)
                    .inspect_err(|e| warn!("tool output render error: {e}"))
                    .ok()
                )
                .unwrap_or_else(|| Self::unknown_ui(tool_name)),
            ToolOutput::Failed { error } => render_failure_to_ui(tool_name, error),
        })
    }

    /// Builds input to the inference API from a tool call's output. Handles
    /// all possible errors.
    pub fn render_to_interface(&self, content: &ToolCallContent) -> InterfaceToolOutput {
        let tool_name = &content.tool_name;
        match &content.output {
            Some(ToolOutput::Completed { value }) => self.tools.get(tool_name)
                .and_then(|tool| tool.render_to_interface(&content.args, value)
                    .inspect_err(|e| warn!("tool call parse error: {e}"))
                    .ok()
                )
                .unwrap_or_else(render_unknown_to_interface),
            Some(ToolOutput::Failed { error }) => {
                render_failure_to_interface(error)
            }
            None => InterfaceToolOutput {
                content: vec![ToolOutputContent::Text { text: "tool call interrupted".into() }],
            },
        }
    }

    // Temp shim
    pub fn render_db_item_to_ui(&self, row: &DbItem) -> Option<Box<dyn RenderItem>> {
        let item = Item::from_row(row).ok()?;
        let content = item.content.as_tool_call()?;
        self.render_to_ui(content)
    }

    fn unknown_ui(tool_name: &str) -> Box<dyn RenderItem> {
        let tool_name = if tool_name.is_empty() { "<missing name>" } else { tool_name };
        render_unknown_to_ui(tool_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::cwd::cwd;
    use crate::testing::{session_turn, tool_call};

    #[tokio::test]
    async fn test_call_tool() {
        let dir = Arc::new(cwd());
        let registry = ToolRegistry::new(&dir);

        let output = registry.call("sh", &json!({ "command": "printf 'hi'" })).await;
        let ToolOutput::Completed { value } = &output else {
            panic!("expected a completed call, got {output:?}");
        };
        assert_eq!(value["stdout"], json!("hi"));

        // Failed calls record their error.
        assert_eq!(
            registry.call("sh", &json!({ "dnammoc": 123 })).await,
            ToolOutput::Failed {
                error: "invalid arguments for 'sh': expected {\"command\": \"...\"}".to_owned(),
            }
        );

        assert_eq!(
            registry.call("missing", &json!({})).await,
            ToolOutput::Failed { error: "no such tool 'missing'".to_owned() },
        );
    }

    #[tokio::test]
    async fn test_call_failure_round_trip() {
        let dir = Arc::new(cwd());
        let registry = ToolRegistry::new(&dir);
        let mut conn = crate::db::open_new().unwrap();
        let (_, turn) = session_turn(&mut conn);

        // Tool call expects a string
        let mut item = tool_call(&mut conn, &turn, "sh", "call123", json!({ "command": 123 }), None);
        let tc = item.content.as_tool_call().unwrap();
        let output = registry.call(&tc.tool_name, &tc.args).await;
        let error = "invalid arguments for 'sh': expected {\"command\": \"...\"}";
        assert_eq!(output, ToolOutput::Failed { error: error.to_owned() });
        item.set_output(&mut conn, output).unwrap();

        // Round trip gives expected results
        let item = Item::get(&mut conn, item.id).unwrap().unwrap();
        assert_eq!(
            item.content.as_tool_call().unwrap(),
            &ToolCallContent {
                tool_name: "sh".into(),
                call_id: "call123".into(),
                args: json!({ "command": 123 }),
                output: Some(ToolOutput::Failed { error: error.into() }),
            },
        );
    }

    #[test]
    fn test_list_tools() {
        let dir = Arc::new(cwd());
        let registry = ToolRegistry::new(&dir);
        let mut names: Vec<&str> = registry.list_tools().map(|t| t.name()).collect();
        names.sort();
        assert_eq!(names, ["edit", "read", "sh"]);

        // Only visible tools are advertised to the model.
        let mut infos = registry.list_tool_infos();
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            infos,
            [
                ToolInfo {
                    name: "edit".to_owned(),
                    description: "Finds and replaces text in a file. The old \
                        string must be unique unless `replace_all` is true."
                        .to_owned(),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "filepath": { "type": "string" },
                            "old_string": { "type": "string" },
                            "new_string": { "type": "string" },
                            "replace_all": { "type": "boolean" },
                        },
                        "required": ["filepath", "old_string", "new_string", "replace_all"],
                        "additionalProperties": false,
                    }),
                },
                ToolInfo {
                    name: "read".to_owned(),
                    description: "Reads lines from a text file. Start line is \
                        1-indexed. Output includes next line offset for \
                        pagination. Lines truncated at 2000 bytes max. UTF-8 \
                        only currently."
                        .to_owned(),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "filepath": { "type": "string" },
                            "start_line": { "type": "integer", "minimum": 1 },
                            "max_lines": { "type": "integer", "minimum": 1 },
                        },
                        "required": ["filepath", "start_line", "max_lines"],
                        "additionalProperties": false,
                    }),
                },
                ToolInfo {
                    name: "sh".to_owned(),
                    description: "Run a shell command on the host system. \
                        Equivalent to `sh -C 'command'`."
                        .to_owned(),
                    input_schema: json!({
                        "type": "object",
                        "properties": { "command": { "type": "string" } },
                        "required": ["command"],
                        "additionalProperties": false,
                    }),
                },
            ]
        );
    }

    #[test]
    fn test_render_unknown() {
        let dir = Arc::new(cwd());
        let registry = ToolRegistry::new(&dir);

        let call = ToolCallContent {
            tool_name: "missing".into(),
            call_id: "call123".into(),
            args: json!({}),
            output: Some(ToolOutput::Completed { value: json!({}) }),
        };
        let ui = registry.render_to_ui(&call).unwrap();
        assert_eq!(ui.query("/type").unwrap(), json!("help"));
        let out = registry.render_to_interface(&call);
        assert_eq!(out.content.len(), 1);
    }

    #[test]
    fn test_render_failed() {
        let dir = Arc::new(cwd());
        let registry = ToolRegistry::new(&dir);

        let call = ToolCallContent {
            tool_name: "boom".into(),
            call_id: "call123".into(),
            args: json!({}),
            output: Some(ToolOutput::Failed { error: "boom".to_owned() }),
        };
        let ui = registry.render_to_ui(&call).unwrap();
        assert_eq!(ui.query("/type").unwrap(), json!("error"));
        let out = registry.render_to_interface(&call);
        assert_eq!(out.content.len(), 1);
    }

    #[test]
    fn test_render_incomplete() {
        let dir = Arc::new(cwd());
        let registry = ToolRegistry::new(&dir);
        let pending = ToolCallContent {
            tool_name: "sh".into(),
            call_id: "call123".into(),
            args: json!({ "command": "echo 123" }),
            output: None,
        };
        assert!(registry.render_to_ui(&pending).is_none());
        let ToolOutputContent::Text { text } = &registry.render_to_interface(&pending).content[0] else { panic!() };
        assert_eq!(text, "tool call interrupted");
    }
}
