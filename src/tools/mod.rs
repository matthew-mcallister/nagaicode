use std::path::Path;

use fnv::FnvHashMap;
use futures::future::BoxFuture;
use serde_json::Value;

use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::query::DataQuery;
use crate::ui::render_item::RenderItem;

pub mod sh;

/// Interfaces for interacting with tools and rendering their output.
pub trait Tool: std::fmt::Debug + DataQuery {
    /// Name provided to the agent and persisted to the database. Must be
    /// unique.
    fn name(&self) -> &str;

    /// Description provided to the agent.
    fn description(&self) -> &str;

    /// JSON schema used by the model to generate tool calls. Must obey
    /// "strict" rules: all inputs required, no additional fields allowed.
    fn input_schema(&self) -> &Value;

    /// Invokes the tool on input.
    fn call<'a>(&'a self, input: &'a Value) -> BoxFuture<'a, AnyResult<Value>>;

    /// Builds a renderable item from the tool's output.
    fn render_to_ui(&self, input: &Value, output: &Value) -> AnyResult<Box<dyn RenderItem>>;

    /// Builds inference call inputs from the tool's output.
    fn render_to_interface<'a>(&self, input: &'a Value, output: &'a Value) -> AnyResult<Vec<ToolOutputContent<'a>>>;
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
        // - invalid
        // - unknown
        let mut tools: FnvHashMap<String, Box<dyn Tool>> = FnvHashMap::default();
        let sh = sh::ShTool::new(cwd.to_path_buf());
        tools.insert(sh.name().to_owned(), Box::new(sh));
        Self { tools }
    }
}