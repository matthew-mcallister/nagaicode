use std::borrow::Cow;
use std::sync::{Arc, RwLock};

use fnv::FnvHashMap;
use log::{trace, warn};
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::cwd::Cwd;
use crate::error::AnyResult;
use crate::interface::{ToolInfo, ToolOutputContent as InterfaceToolOutputContent};
use crate::item::{
    ToolCallContent, ToolCompletion, ToolOutput, ToolOutputContent, ToolRenderInfo,
};
use crate::mcp::tool::McpRenderer;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::request::DefaultClient;
use crate::ui::render_item::{ErrorRenderItem, SystemRenderItem, RenderItem};

pub mod edit;
pub mod glob;
pub mod read;
pub mod sh;
pub mod web_fetch;
pub mod write;

pub trait ToolRenderer: std::fmt::Debug + DataQuery + Send + Sync {
    /// Name used to identify, must be unique
    fn name(&self) -> &str;

    /// Builds a renderable item from the tool's output.
    fn render_to_ui(&self, info: &ToolRenderInfo) -> AnyResult<Box<dyn RenderItem>>;

    /// Builds inference call inputs from the tool's output.
    fn render_to_model(&self, info: &ToolRenderInfo) -> AnyResult<InterfaceToolOutput>;
}

/// Renderer used when no renderer is available for a tool's output.
#[derive(Debug)]
pub struct UnknownRenderer;

const UNKNOWN_RENDERER: UnknownRenderer = UnknownRenderer;

impl UnknownRenderer {
    fn render_ui(tool_name: &str) -> Box<dyn RenderItem> {
        let tool_name = if tool_name.is_empty() { "<missing name>" } else { tool_name };
        Box::new(SystemRenderItem::new(format!("Called '{}'", tool_name)))
    }

    fn render_model() -> InterfaceToolOutput {
        // FIXME: should render raw JSON instead of a useless error message
        InterfaceToolOutput {
            content: vec![InterfaceToolOutputContent::Text {
                text: Cow::Owned("error: could not parse output".into()),
            }],
        }
    }
}

impl DataQuery for UnknownRenderer {
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

impl ToolRenderer for UnknownRenderer {
    fn name(&self) -> &str {
        "unknown"
    }

    fn render_to_ui(&self, info: &ToolRenderInfo) -> AnyResult<Box<dyn RenderItem>> {
        Ok(Self::render_ui(info.tool_name))
    }

    fn render_to_model(&self, _info: &ToolRenderInfo) -> AnyResult<InterfaceToolOutput> {
        Ok(Self::render_model())
    }
}

/// Interfaces for interacting with tools and rendering their output.
pub trait Tool: std::fmt::Debug + DataQuery + Send + Sync {
    /// Name provided to the agent and persisted to the database. When multiple
    /// tools have the same name, the tool loaded last shadows the other tools.
    fn name(&self) -> &str;

    /// Description provided to the agent.
    fn description(&self) -> &str;

    /// Name of the renderer used for this tool's outputs.
    fn renderer(&self) -> &str;

    /// Whether the tool is advertised to the model.
    fn is_visible(&self) -> bool {
        true
    }

    /// JSON schema used by the model to generate tool calls. Must obey
    /// "strict" rules: all inputs required, no additional fields allowed.
    fn input_schema(&self) -> &Value;

    /// Invokes the tool on input.
    fn call<'a>(
        &'a self,
        context: &'a ToolCallContext,
        input: &'a Value,
    ) -> BoxFuture<'a, AnyResult<Value>>;

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
    pub content: Vec<InterfaceToolOutputContent<'static>>,
}

/// Args passed at tool creation time.
#[derive(Debug)]
pub struct ToolCreateArgs<'a> {
    pub(crate) cwd: &'a Arc<Cwd>,
    pub(crate) client: &'a DefaultClient,
}

#[derive(Debug)]
pub struct ToolCallContext {
    pub session_id: i32,
}

/// Maps tool names to tools.
#[derive(Debug)]
pub struct ToolRegistry {
    tools: RwLock<FnvHashMap<String, Arc<dyn Tool>>>,
    renderers: FnvHashMap<String, Box<dyn ToolRenderer>>,
}

impl ToolRegistry {
    pub fn new(args: &ToolCreateArgs<'_>) -> Self {
        let mut this = Self {
            tools: Default::default(),
            renderers: Default::default(),
        };
        
        // Builtin tools
        this.register(Box::new(sh::ShTool::new(args)));
        this.register(Box::new(read::ReadTool::new(args)));
        this.register(Box::new(edit::EditTool::new(args)));
        this.register(Box::new(write::WriteTool::new(args)));
        this.register(Box::new(glob::GlobTool::new(args)));
        this.register(Box::new(web_fetch::WebFetchTool::new(args)));

        // Builtin renderers
        this.register_renderer(Box::new(UnknownRenderer));
        this.register_renderer(Box::new(sh::ShRenderer));
        this.register_renderer(Box::new(read::ReadRenderer::new(args)));
        this.register_renderer(Box::new(edit::EditRenderer::new(args)));
        this.register_renderer(Box::new(write::WriteRenderer::new(args)));
        this.register_renderer(Box::new(glob::GlobRenderer));
        this.register_renderer(Box::new(web_fetch::WebFetchRenderer));
        this.register_renderer(Box::new(McpRenderer));

        this
    }

    fn register_renderer(&mut self, renderer: Box<dyn ToolRenderer>) {
        let name = renderer.name().to_owned();
        self.renderers.insert(name.clone(), renderer).expect("duplicate renderer");
    }

    /// Looks up the renderer with the given name, falling back on
    /// `UnknownRenderer` if there is no such renderer.
    pub fn get_renderer(&self, name: &str) -> &dyn ToolRenderer {
        match self.renderers.get(name) {
            Some(renderer) => &**renderer,
            None => &UNKNOWN_RENDERER,
        }
    }

    pub(crate) fn register(&self, tool: Box<dyn Tool>) {
        let name = tool.name().to_owned();
        let mut tools = self.tools.write().unwrap();
        let old = tools.insert(name.clone(), Arc::from(tool));
        if old.is_some() {
            warn!("duplicate tool registered: '{}'", name);
        } else {
            trace!("registered tool '{}'", name);
        }
    }

    /// Lists tools available to the model.
    pub fn list_tools(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.read().unwrap().values().filter(|t| t.is_visible()).cloned().collect()
    }

    /// Describes every tool available to the model for the inference API.
    pub fn list_tool_infos(&self) -> Vec<ToolInfo> {
        self.list_tools().iter().map(|tool| tool.tool_info()).collect()
    }

    /// Invokes the tool named by a tool call item. Handles all possible
    /// errors. Makes no DB queries.
    pub async fn call(
        &self,
        context: &ToolCallContext,
        tool_name: &str,
        input: &Value,
    ) -> ToolCompletion {
        let tool = self.tools.read().unwrap().get(tool_name).cloned();
        let Some(tool) = tool else {
            return ToolCompletion {
                renderer: UNKNOWN_RENDERER.name().to_owned(),
                output: ToolOutput::Failed {
                    error: format!("no such tool '{}'", tool_name),
                },
            };
        };
        let renderer = tool.renderer().to_owned();
        let output = match tool.call(context, input).await {
            Ok(value) => ToolOutput::Completed { value },
            Err(e) => ToolOutput::Failed { error: e.to_string() },
        };
        ToolCompletion { renderer, output }
    }

    /// Builds a render item to display a tool call. Handles all possible
    /// errors. Returns `None` if the tool call has no output.
    pub fn render_to_ui(&self, content: &ToolOutputContent) -> Option<Box<dyn RenderItem>> {
        let tool_name = &content.tool_name;
        Some(match &content.completion.output {
            ToolOutput::Completed { value } => {
                let info = ToolRenderInfo {
                    tool_name,
                    args: &content.args,
                    output: value,
                };
                self.get_renderer(&content.completion.renderer)
                    .render_to_ui(&info)
                    .inspect_err(|e| warn!("tool output render error: {e}"))
                    .unwrap_or_else(|_| UnknownRenderer::render_ui(tool_name))
            }
            ToolOutput::Failed { error } => Self::failure_ui(tool_name, &error),
        })
    }

    /// Builds input to the inference API from a tool call's output. Handles
    /// all possible errors.
    pub fn render_to_interface(&self, content: &ToolCallContent) -> InterfaceToolOutput {
        let tool_name = &content.tool_name;
        match &content.completion {
            Some(ToolCompletion {
                renderer,
                output: ToolOutput::Completed { value },
            }) => {
                let info = ToolRenderInfo {
                    tool_name,
                    args: &content.args,
                    output: value,
                };
                self.get_renderer(renderer)
                    .render_to_model(&info)
                    .inspect_err(|e| warn!("tool call parse error: {e}"))
                    .unwrap_or_else(|_| UnknownRenderer::render_model())
            }
            Some(ToolCompletion {
                output: ToolOutput::Failed { error },
                ..
            }) => Self::failure_to_interface(error),
            None => InterfaceToolOutput {
                content: vec![InterfaceToolOutputContent::Text { text: "tool call interrupted".into() }],
            },
        }
    }

    fn failure_ui(tool_name: &str, error: &str) -> Box<dyn RenderItem> {
        Box::new(ErrorRenderItem::new(format!("Called '{tool_name}': {error}")))
    }

    fn failure_to_interface(error: &str) -> InterfaceToolOutput {
        InterfaceToolOutput {
            content: vec![InterfaceToolOutputContent::Text {
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
        let context = ToolCallContext { session_id: 1 };

        let completion = registry
            .call(&context, "sh", &json!({ "command": "echo hi" }))
            .await;
        assert_eq!(completion.renderer, "sh");
        let ToolOutput::Completed { value } = &completion.output else {
            panic!("expected a completed call, got {completion:?}");
        };
        assert_eq!(value["return_code"], json!(0));
        assert_eq!(value["stderr"], json!(""));
        assert_eq!(normalize(value["stdout"].as_str().unwrap()), "hi\n");

        // Failed calls record their error.
        assert_eq!(
            registry.call(&context, "sh", &json!({ "dnammoc": 123 })).await,
            ToolCompletion {
                renderer: "sh".to_owned(),
                output: ToolOutput::Failed {
                    error: "invalid arguments for 'sh': expected {\"command\": \"...\"}".to_owned(),
                },
            }
        );

        assert_eq!(
            registry.call(&context, "missing", &json!({})).await,
            ToolCompletion {
                renderer: "unknown".to_owned(),
                output: ToolOutput::Failed { error: "no such tool 'missing'".to_owned() },
            },
        );
    }

    #[tokio::test]
    async fn test_call_failure_round_trip() {
        let registry = registry();
        let mut conn = crate::db::open_new().unwrap();
        let (session, turn) = session_turn(&mut conn);
        let context = ToolCallContext { session_id: session.id };

        // Tool call expects a string
        let mut item = tool_call(&mut conn, &turn, "sh", "call123", json!({ "command": 123 }), None);
        let tc = item.content.as_tool_call().unwrap();
        let completion = registry.call(&context, &tc.tool_name, &tc.args).await;
        let error = "invalid arguments for 'sh': expected {\"command\": \"...\"}";
        assert_eq!(
            completion,
            ToolCompletion {
                renderer: "sh".to_owned(),
                output: ToolOutput::Failed { error: error.to_owned() },
            }
        );
        item.set_output(&mut conn, completion.clone()).unwrap();

        // Round trip gives expected results
        let item = Item::get(&mut conn, item.id).unwrap().unwrap();
        assert_eq!(
            item.content.as_tool_call().unwrap(),
            &ToolCallContent {
                tool_name: "sh".into(),
                call_id: "call123".into(),
                args: json!({ "command": 123 }),
                completion: Some(completion),
            },
        );
    }

    #[test]
    fn test_list_tools() {
        let registry = registry();
        let mut names: Vec<String> = registry.list_tools().iter().map(|t| t.name().to_owned()).collect();
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

        let call = ToolOutputContent {
            tool_name: "missing".into(),
            call_id: "call123".into(),
            args: json!({}),
            completion: ToolCompletion {
                renderer: "missing".into(),
                output: ToolOutput::Completed { value: json!({}) },
            },
        };
        let ui = registry.render_to_ui(&call).unwrap();
        assert_eq!(ui.query("/type").unwrap(), json!("help"));
        let out = registry.render_to_interface(&call.into_call());
        assert_eq!(out.content.len(), 1);
    }

    #[test]
    fn test_render_failed() {
        let registry = registry();

        let call = ToolOutputContent {
            tool_name: "boom".into(),
            call_id: "call123".into(),
            args: json!({}),
            completion: ToolCompletion {
                renderer: "boom".into(),
                output: ToolOutput::Failed { error: "boom".to_owned() },
            },
        };
        let ui = registry.render_to_ui(&call).unwrap();
        assert_eq!(ui.query("/type").unwrap(), json!("error"));
        let out = registry.render_to_interface(&call.into_call());
        assert_eq!(out.content.len(), 1);
    }
}
