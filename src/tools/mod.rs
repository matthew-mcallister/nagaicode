use std::sync::Arc;

use fnv::FnvHashMap;
use log::warn;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::cwd::Cwd;
use crate::error::AnyResult;
use crate::interface::{ToolInfo, ToolOutputContent};
use crate::query::DataQuery;
use crate::session::Item;
use crate::try_nested;
use crate::ui::render_item::RenderItem;

pub mod failed;
pub mod read;
pub mod sh;
pub mod unknown;

use failed::FailedTool;
use unknown::UnknownTool;

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

/// Output of a tool call. Returns fields to serialize to DB.
#[derive(Debug)]
pub struct ToolResult {
    pub name: String,
    pub output: Value,
}

/// Tool call representation submitted to inference API.
#[derive(Debug)]
pub struct InterfaceToolOutput {
    pub name: String,
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
        let failed = failed::FailedTool::new();
        tools.insert(failed.name().to_owned(), Box::new(failed));
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

    /// Invokes a tool from raw name and input text. Handles all possible
    /// errors. Modifies the item but doesn't make any DB queries.
    pub async fn call(&self, item: &mut Item) -> ToolResult {
        let args = match item.tool_args() {
            Ok(Some(args)) => args,
            Ok(None) => {
                let name = item.text.clone().unwrap_or_default();
                return Self::fail(item, &name, "tool call item has no args");
            }
            Err(e) => {
                let name = item.text.clone().unwrap_or_default();
                return Self::fail(item, &name, &e.to_string());
            }
        };
        match self.tools.get(&args.name) {
            Some(tool) => match tool.call(&args.args).await {
                Ok(output) => ToolResult { name: args.name, output },
                Err(e) => Self::fail(item, &args.name, &e.to_string()),
            },
            None => Self::fail(item, &args.name, "unknown tool"),
        }
    }

    /// Builds a render item to display a tool call. Handles all possible
    /// errors. Returns `None` if the item is not a tool call with output.
    pub fn render_to_ui(&self, item: &Item) -> Option<Box<dyn RenderItem>> {
        fn inner(reg: &ToolRegistry, item: &Item) -> AnyResult<Option<Box<dyn RenderItem>>> {
            // Output is parsed first so that calls which are still pending
            // are ignored instead of falling back to the placeholder.
            let output = try_nested!(item.tool_output());
            let args = try_nested!(item.tool_args());
            let tool = reg.tools.get(&args.name)
                .ok_or_else(|| anyhow::anyhow!("unknown tool: '{}'", args.name))?;
            Ok(Some(tool.render_to_ui(&args.args, &output)?))
        }

        match inner(self, item).transpose()? {
            Ok(item) => Some(item),
            Err(e) => {
                warn!("item {}: tool call parse error: {}", item.id, e);
                Some(Self::unknown_ui(item))
            }
        }
    }

    /// Builds input to the inference API. Handles all possible errors.
    pub fn render_to_interface(&self, item: &Item) -> InterfaceToolOutput {
        let res: Option<_> = self.resolve(item).and_then(|(tool, input, output)| {
            match tool.render_to_interface(&input, &output) {
                Ok(out) => Some(out),
                Err(_) => None,
            }
        });
        res.unwrap_or_else(|| Self::unknown_interface(item))
    }

    /// Resolves a tool call item to its tool and parsed input/output.
    fn resolve<'a>(&'a self, item: &'a Item) -> Option<(&'a dyn Tool, Value, Value)> {
        let name = item.text.as_deref()?;
        let tool = self.tools.get(name)?;
        let input = item.tool_args_json().ok().flatten().unwrap_or(Value::Null);
        let output = item.tool_output().ok().flatten().unwrap_or(Value::Null);
        Some((tool.as_ref(), input, output))
    }

    /// Writes a failure for a tool call and builds its `ToolResult`.
    fn fail(item: &mut Item, tool_name: &str, message: &str) -> ToolResult {
        FailedTool::write_failure(item, tool_name, message);
        let output = item
            .tool_output()
            .ok()
            .flatten()
            .unwrap_or_else(|| json!({ "error": "unknown error" }));
        let name = output
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("failed")
            .to_owned();
        ToolResult { name, output }
    }

    /// Renders an unparseable tool call for the UI.
    fn unknown_ui(item: &Item) -> Box<dyn RenderItem> {
        let name = item.text.as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or("<missing name>");
        let tool = UnknownTool::new(name);
        tool.render_to_ui(&Value::Null, &Value::Null).expect("infallible")
    }

    /// Renders an unparseable tool call for the inference API.
    fn unknown_interface(item: &Item) -> InterfaceToolOutput {
        let name = item.text.clone().unwrap_or_default();
        let tool = UnknownTool::new(name);
        tool.render_to_interface(&Value::Null, &Value::Null).expect("infallible")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cwd::cwd;
    use crate::session::{ItemType, NewItem};
    use crate::testing::{session_turn, tool_call};

    #[tokio::test]
    async fn test_call_tool() {
        let dir = Arc::new(cwd());
        let registry = ToolRegistry::new(&dir);
        let mut conn = crate::db::open_new().expect("open db");
        let (_, turn) = session_turn(&mut conn);

        let mut item = tool_call(&mut conn, &turn, "sh", json!({ "command": "printf 'hi'" }), None);
        let result = registry.call(&mut item).await;
        assert_eq!(result.name, "sh");
        assert_eq!(result.output["stdout"], json!("hi"));

        let mut item = tool_call(&mut conn, &turn, "sh", json!({ "command": 123 }), None);
        let result = registry.call(&mut item).await;
        assert_eq!(result.name, "sh");
        assert_eq!(result.output["error"], json!("invalid arguments for 'sh': expected {\"command\": \"...\"}"));
        assert_eq!(item.tool_output, Some(result.output.to_string()));

        let mut item = tool_call(&mut conn, &turn, "no_such_tool", json!({}), None);
        let result = registry.call(&mut item).await;
        assert_eq!(result.name, "no_such_tool");
        assert_eq!(result.output["error"], json!("unknown tool"));
    }

    #[test]
    fn test_list_tools() {
        let dir = Arc::new(cwd());
        let registry = ToolRegistry::new(&dir);
        let mut names: Vec<&str> = registry.list_tools().map(|t| t.name()).collect();
        names.sort();
        assert_eq!(names, ["read", "sh"]);

        // Only visible tools are advertised to the model.
        let mut infos = registry.list_tool_infos();
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            infos,
            [
                ToolInfo {
                    name: "read".to_owned(),
                    description: "Reads lines from a text file. Start line is \
                        1-indexed. Output includes next line offset for \
                        pagination. Lines truncated at 2000 bytes. UTF-8 only \
                        currently."
                        .to_owned(),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "filepath": { "type": "string" },
                            "start_line": { "type": "integer" },
                            "max_lines": { "type": "integer" },
                        },
                        "required": ["filepath", "start_line", "max_lines"],
                        "additionalProperties": false,
                    }),
                },
                ToolInfo {
                    name: "sh".to_owned(),
                    description: "Run a shell command on the host system".to_owned(),
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
        let mut conn = crate::db::open_new().expect("open db");
        let (_, turn) = session_turn(&mut conn);

        let item = tool_call(&mut conn, &turn, "no_such_tool", json!({}), Some(json!({})));
        let ui = registry.render_to_ui(&item).unwrap();
        assert_eq!(ui.query("/type").unwrap(), json!("help"));
        let out = registry.render_to_interface(&item);
        assert_eq!(out.name, "no_such_tool");
        assert_eq!(out.content.len(), 1);
    }

    #[test]
    fn test_render_failed() {
        let dir = Arc::new(cwd());
        let registry = ToolRegistry::new(&dir);
        let mut conn = crate::db::open_new().expect("open db");
        let (_, turn) = session_turn(&mut conn);

        let output = json!({ "tool_name": "sh", "error": "boom" });
        let item = tool_call(&mut conn, &turn, "failed", json!({}), Some(output));
        let ui = registry.render_to_ui(&item).unwrap();
        assert_eq!(ui.query("/type").unwrap(), json!("error"));
        let out = registry.render_to_interface(&item);
        assert_eq!(out.name, "sh");
        assert_eq!(out.content.len(), 1);
    }

    #[test]
    fn test_render_sh() {
        let dir = Arc::new(cwd());
        let registry = ToolRegistry::new(&dir);
        let mut conn = crate::db::open_new().expect("open db");
        let (_, turn) = session_turn(&mut conn);

        let output = json!({ "stdout": "hello", "stderr": "", "return_code": 0 });
        let item = tool_call(
            &mut conn,
            &turn,
            "sh",
            json!({ "command": "echo hello" }),
            Some(output),
        );
        let ui = registry.render_to_ui(&item).unwrap();
        assert_eq!(ui.query("/type").unwrap(), json!("sh"));
        let out = registry.render_to_interface(&item);
        assert_eq!(out.name, "sh");
        assert_eq!(out.content.len(), 2);
    }

    #[test]
    fn test_render_incomplete_item() {
        let dir = Arc::new(cwd());
        let registry = ToolRegistry::new(&dir);
        let mut conn = crate::db::open_new().expect("open db");
        let (_, turn) = session_turn(&mut conn);

        // A tool call which hasn't produced output yet isn't rendered.
        let pending = tool_call(
            &mut conn,
            &turn,
            "sh",
            json!({ "command": "echo hello" }),
            None,
        );
        assert!(registry.render_to_ui(&pending).is_none());

        // Items which aren't tool calls aren't rendered.
        let text_item = Item::create(
            &mut conn,
            NewItem {
                session_id: Some(turn.session_id),
                turn_id: Some(turn.id),
                ty: Some(ItemType::UserText),
                text: Some("hello"),
                ..Default::default()
            },
        )
        .expect("create text item");
        assert!(registry.render_to_ui(&text_item).is_none());
    }
}
