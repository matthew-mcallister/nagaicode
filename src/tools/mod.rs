use std::borrow::Cow;
use std::sync::Arc;

use fnv::FnvHashMap;
use log::warn;
use futures::future::BoxFuture;
use serde_json::Value;

use crate::config::Dirs;
use crate::cwd::Cwd;
use crate::error::AnyResult;
use crate::interface::{ToolInfo, ToolOutputContent};
use crate::item::{ToolCallContent, ToolOutput};
use crate::query::DataQuery;
use crate::request::DefaultClient;
use crate::ui::render_item::{ErrorRenderItem, SystemRenderItem, RenderItem};

pub mod edit;
pub mod glob;
pub mod read;
pub mod sh;
pub mod web_fetch;
pub mod write;

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

    // XXX: Should remove AnyResult from signature and force implementors to
    // be liberal in what they accept.
    /// Builds a renderable item from the tool's output.
    fn render_to_ui(&self, input: &Value, output: &Value) -> AnyResult<Box<dyn RenderItem>>;

    // XXX: Same as above.
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

/// Args passed at tool creation time.
#[derive(Debug)]
pub struct ToolCreateArgs<'a> {
    pub(crate) cwd: &'a Arc<Cwd>,
    pub(crate) dirs: &'a Dirs,
    pub(crate) client: &'a DefaultClient,
}

/// Maps tool names to tools.
#[derive(Debug)]
pub struct ToolRegistry {
    tools: FnvHashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new(args: &ToolCreateArgs<'_>) -> Self {
        // Builtin tools
        // - sh
        // - read
        // - write
        // - edit
        // - glob
        // - web_fetch
        let mut tools: FnvHashMap<String, Box<dyn Tool>> = FnvHashMap::default();
        let sh = sh::ShTool::new(args);
        tools.insert(sh.name().to_owned(), Box::new(sh));
        let read = read::ReadTool::new(args);
        tools.insert(read.name().to_owned(), Box::new(read));
        let edit = edit::EditTool::new(args);
        tools.insert(edit.name().to_owned(), Box::new(edit));
        let write = write::WriteTool::new(args);
        tools.insert(write.name().to_owned(), Box::new(write));
        let glob = glob::GlobTool::new(args);
        tools.insert(glob.name().to_owned(), Box::new(glob));
        let web_fetch = web_fetch::WebFetchTool::new(args);
        tools.insert(web_fetch.name().to_owned(), Box::new(web_fetch));
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
            ToolOutput::Failed { error } => Self::failure_ui(tool_name, error),
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
                .unwrap_or_else(Self::unknown_to_interface),
            Some(ToolOutput::Failed { error }) => Self::failure_to_interface(error),
            None => InterfaceToolOutput {
                content: vec![ToolOutputContent::Text { text: "tool call interrupted".into() }],
            },
        }
    }

    fn unknown_ui(tool_name: &str) -> Box<dyn RenderItem> {
        let tool_name = if tool_name.is_empty() { "<missing name>" } else { tool_name };
        Box::new(SystemRenderItem::new(format!("Called '{}'", tool_name)))
    }

    fn unknown_to_interface() -> InterfaceToolOutput {
        InterfaceToolOutput {
            content: vec![ToolOutputContent::Text {
                text: Cow::Owned("error: could not parse output".into()),
            }],
        }
    }

    fn failure_ui(tool_name: &str, error: &str) -> Box<dyn RenderItem> {
        Box::new(ErrorRenderItem::new(format!("Called '{tool_name}': {error}")))
    }

    fn failure_to_interface(error: &str) -> InterfaceToolOutput {
        InterfaceToolOutput {
            content: vec![ToolOutputContent::Text {
                text: Cow::Owned(format!("error: {error}")),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::app::App;
    use crate::item::Item;
    use crate::testing::{normalize, session_turn, tool_call};

    fn registry() -> Arc<ToolRegistry> {
        App::new().unwrap().tool_registry().clone()
    }

    #[tokio::test]
    async fn test_call_tool() {
        let registry = registry();

        let output = registry.call("sh", &json!({ "command": "echo hi" })).await;
        let ToolOutput::Completed { value } = &output else {
            panic!("expected a completed call, got {output:?}");
        };
        assert_eq!(value["return_code"], json!(0));
        assert_eq!(value["stderr"], json!(""));
        assert_eq!(normalize(value["stdout"].as_str().unwrap()), "hi\n");

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
        let registry = registry();
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
        let registry = registry();
        let mut names: Vec<&str> = registry.list_tools().map(|t| t.name()).collect();
        names.sort();
        assert_eq!(names, ["edit", "glob", "read", "sh", "web_fetch", "write"]);

        let mut names: Vec<_> = registry.list_tool_infos()
            .into_iter()
            .map(|info| info.name)
            .collect();
        names.sort();
        assert_eq!(names, &["edit", "glob", "read", "sh", "web_fetch", "write"]);
    }

    #[test]
    fn test_render_unknown() {
        let registry = registry();

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
        let registry = registry();

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
        let registry = registry();
        let pending = ToolCallContent {
            tool_name: "sh".into(),
            call_id: "call123".into(),
            args: json!({ "command": "echo 123" }),
            output: None,
        };
        assert!(registry.render_to_ui(&pending).is_none());
        let ToolOutputContent::Text { text } = &registry.render_to_interface(&pending).content[0];
        assert_eq!(text, "tool call interrupted");
    }
}
