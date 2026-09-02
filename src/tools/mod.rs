use std::path::Path;

use fnv::FnvHashMap;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::query::DataQuery;
use crate::session::Item;
use crate::ui::render_item::RenderItem;

pub mod failed;
pub mod sh;
pub mod unknown;

use failed::FailedTool;
use unknown::UnknownTool;

/// Interfaces for interacting with tools and rendering their output.
pub trait Tool: std::fmt::Debug + DataQuery {
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
    pub fn new(cwd: &Path) -> Self {
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
        let sh = sh::ShTool::new(cwd.to_path_buf());
        tools.insert(sh.name().to_owned(), Box::new(sh));
        let failed = failed::FailedTool::new();
        tools.insert(failed.name().to_owned(), Box::new(failed));
        Self { tools }
    }

    /// Lists tools available to the model.
    pub fn list_tools(&self) -> impl Iterator<Item = &dyn Tool> {
        self.tools.values().map(|t| t.as_ref()).filter(|t| t.is_visible())
    }

    /// Invokes a tool from raw name and input text. Handles all possible
    /// errors. Modifies the item but doesn't make any DB queries.
    pub async fn call_tool(&self, item: &mut Item) -> ToolResult {
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
    /// errors.
    pub fn render_to_ui(&self, item: &Item) -> Box<dyn RenderItem> {
        match self.resolve(item) {
            Some((tool, input, output)) => match tool.render_to_ui(&input, &output) {
                Ok(render) => render,
                Err(_) => Self::unknown_ui(item),
            },
            None => Self::unknown_ui(item),
        }
    }

    /// Builds input to the inference API. Handles all possible errors.
    pub fn render_to_interface(&self, item: &Item) -> InterfaceToolOutput {
        match self.resolve(item) {
            Some((tool, input, output)) => match tool.render_to_interface(&input, &output) {
                Ok(out) => out,
                Err(_) => Self::unknown_interface(item),
            },
            None => Self::unknown_interface(item),
        }
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
        let name = item.text.clone().unwrap_or_default();
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
    use crate::session::{ItemType, NewItem, Session, Turn, TurnType};

    fn make_tool_call(
        conn: &mut diesel::SqliteConnection,
        name: &str,
        args: Value,
        output: Option<Value>,
    ) -> Item {
        let session = Session::create(conn, "Session").expect("create session");
        let turn = Turn::create(conn, session.id, TurnType::Assistant, None, None, None)
            .expect("create turn");
        let mut item = Item::create(
            conn,
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(turn.id),
                ty: Some(ItemType::ToolCall),
                text: Some(name),
                ..Default::default()
            },
        )
        .expect("create tool call");
        Item::update_tool_args(conn, item.id, &args.to_string()).expect("set tool args");
        if let Some(output) = output {
            item.set_tool_output(conn, &output).expect("set tool output");
        }
        Item::get_by_id(conn, item.id).unwrap().unwrap()
    }

    #[tokio::test]
    async fn test_call_tool() {
        let dir = cwd();
        let registry = ToolRegistry::new(dir.path());
        let mut conn = crate::db::open_new().expect("open db");

        let mut item = make_tool_call(&mut conn, "sh", json!({ "command": "printf 'hi'" }), None);
        let result = registry.call_tool(&mut item).await;
        assert_eq!(result.name, "sh");
        assert_eq!(result.output["stdout"], json!("hi"));

        let mut item = make_tool_call(&mut conn, "sh", json!({ "command": 123 }), None);
        let result = registry.call_tool(&mut item).await;
        assert_eq!(result.name, "sh");
        assert_eq!(result.output["error"], json!("invalid arguments for 'sh': expected {\"command\": \"...\"}"));
        assert_eq!(item.tool_output, Some(result.output.to_string()));

        let mut item = make_tool_call(&mut conn, "no_such_tool", json!({}), None);
        let result = registry.call_tool(&mut item).await;
        assert_eq!(result.name, "no_such_tool");
        assert_eq!(result.output["error"], json!("unknown tool"));
    }

    #[test]
    fn test_list_tools() {
        let dir = cwd();
        let registry = ToolRegistry::new(dir.path());
        let names: Vec<&str> = registry.list_tools().map(|t| t.name()).collect();
        assert_eq!(names, ["sh"]);
    }

    #[test]
    fn test_render_unknown() {
        let dir = cwd();
        let registry = ToolRegistry::new(dir.path());
        let mut conn = crate::db::open_new().expect("open db");

        let item = make_tool_call(&mut conn, "no_such_tool", json!({}), Some(json!({})));
        let ui = registry.render_to_ui(&item);
        assert_eq!(ui.query("/type").unwrap(), json!("help"));
        let out = registry.render_to_interface(&item);
        assert_eq!(out.name, "no_such_tool");
        assert_eq!(out.content.len(), 1);
    }

    #[test]
    fn test_render_failed() {
        let dir = cwd();
        let registry = ToolRegistry::new(dir.path());
        let mut conn = crate::db::open_new().expect("open db");

        let output = json!({ "tool_name": "sh", "error": "boom" });
        let item = make_tool_call(&mut conn, "failed", json!({}), Some(output));
        let ui = registry.render_to_ui(&item);
        assert_eq!(ui.query("/type").unwrap(), json!("error"));
        let out = registry.render_to_interface(&item);
        assert_eq!(out.name, "sh");
        assert_eq!(out.content.len(), 1);
    }

    #[test]
    fn test_render_sh() {
        let dir = cwd();
        let registry = ToolRegistry::new(dir.path());
        let mut conn = crate::db::open_new().expect("open db");

        let output = json!({ "stdout": "hello", "stderr": "", "return_code": 0 });
        let item = make_tool_call(
            &mut conn,
            "sh",
            json!({ "command": "echo hello" }),
            Some(output),
        );
        let ui = registry.render_to_ui(&item);
        assert_eq!(ui.query("/type").unwrap(), json!("sh"));
        let out = registry.render_to_interface(&item);
        assert_eq!(out.name, "sh");
        assert_eq!(out.content.len(), 2);
    }
}
