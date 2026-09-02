pub mod openai;
pub mod stream;

use std::borrow::Cow;
use std::str::FromStr;

use anyhow::anyhow;
use futures::Stream;

use serde_json::Value;

use crate::error::{AnyError, AnyResult};
use crate::interface::openai::OpenaiInterface;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::{Item, ItemType};
use crate::tools::ToolRegistry;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum InterfaceId {
    Openai,
    Openrouter,
    Deepseek,
}

impl FromStr for InterfaceId {
    type Err = AnyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "openai" => Ok(Self::Openai),
            "openrouter" => Ok(Self::Openrouter),
            "deepseek" => Ok(Self::Deepseek),
            _ => Err(anyhow!("unknown interface: '{}'", s)),
        }
    }
}

impl InterfaceId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Openrouter => "openrouter",
            Self::Deepseek => "deepseek",
        }
    }
}

impl std::fmt::Display for InterfaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceModel {
    pub id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

/// Output from a tool call.
// XXX: Get rid of lifetime
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(tag = "type")]
pub enum ToolOutputContent<'a> {
    #[serde(rename = "input_text")]
    Text {
        #[serde(rename = "text")]
        text: Cow<'a, str>,
    },
    #[serde(rename = "input_file")]
    File {
        #[serde(rename = "filename")]
        filepath: &'a str,
        // Base64-encoded binary
        #[serde(rename = "file_data")]
        data: &'a str,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChatMessage<'a> {
    /// A user prompt.
    Message { content: &'a str },
    /// A visible assistant response.
    Response { content: &'a str },
    /// Assistant reasoning ("thought") content.
    Reasoning { content: &'a str },
    /// A tool invocation requested by the assistant.
    ToolCall {
        call_id: &'a str,
        name: &'a str,
        arguments: &'a str,
    },
    /// The output of a tool invocation.
    ToolOutput {
        call_id: &'a str,
        output: Vec<ToolOutputContent<'a>>,
    },
}

/// Builds the conversation history from a session's items.
///
/// Text items are always included; reasoning items are only included when
/// `include_reasoning` is true, preferring the summary over the raw text.
///
/// Tool calls are rendered as a call/response pair, with a fallback for
/// incomplete output.
pub fn build_history<'a>(
    tools: &ToolRegistry,
    items: &'a [Item],
    include_reasoning: bool,
) -> AnyResult<Vec<ChatMessage<'a>>> {
    let mut messages = Vec::with_capacity(items.len());
    for item in items {
        match item.ty()? {
            ItemType::UserText => {
                if let Some(text) = item.text.as_deref() {
                    messages.push(ChatMessage::Message { content: text });
                }
            }
            ItemType::ResponseText => {
                if let Some(text) = item.text.as_deref() {
                    messages.push(ChatMessage::Response { content: text });
                }
            }
            ItemType::Reasoning => {
                if !include_reasoning {
                    continue;
                }
                let content = item.summary.as_deref().or(item.text.as_deref());
                if let Some(content) = content {
                    messages.push(ChatMessage::Reasoning { content });
                }
            }
            ItemType::ToolCall => {
                let (Some(call_id), Some(name)) =
                    (item.upstream_call_id.as_deref(), item.text.as_deref())
                else {
                    // Fallback in case database is corrupted
                    continue;
                };
                messages.push(ChatMessage::ToolCall {
                    call_id,
                    name,
                    arguments: item.tool_args.as_deref().unwrap_or(""),
                });
                let output = if item.tool_output()?.is_some() {
                    tools.render_to_interface(item).content
                } else {
                    // Fallback for calls which never produced output
                    vec![ToolOutputContent::Text {
                        text: Cow::Borrowed("error: tool call interrupted"),
                    }]
                };
                messages.push(ChatMessage::ToolOutput { call_id, output });
            }
        }
    }
    Ok(messages)
}

/// Describes a tool in human- and model-readable format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Parameters for inference.
#[derive(Clone, Debug, PartialEq)]
pub struct InferenceParams<'a> {
    pub model_id: &'a str,
    pub system_prompt: &'a str,
    pub temperature: f32,
    /// Reasoning effort. `Some(ReasoningEffort::None)` means *no* reasoning,
    /// while `None` means *default* reasoning.
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Tools advertised to the model.
    pub tools: Vec<ToolInfo>,
    pub input: &'a [ChatMessage<'a>],
}

/// Data returned by API when response is initially created.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseCreated {
    pub id: String,
    pub status: String,
}

/// Token usage
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub total_tokens: u64,
}

/// An output item, emitted when an output item is added to or completed in
/// the response.
#[derive(Clone, Debug, PartialEq)]
pub struct OutputItemEvent {
    /// Position of the item in the response's output array. Add, delta, and
    /// done events for the same item share this index.
    pub output_index: i64,
    /// Upstream item id, e.g. "msg_...", "rs_...".
    pub id: String,
    /// Upstream item type, e.g. "message", "reasoning", "function_call".
    pub ty: String,
    /// Upstream tool call id, e.g. "call_...", present on function call items.
    pub call_id: Option<String>,
    /// Full raw JSON of the output item.
    pub raw: Value,
}

/// An incremental text update for an output item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemDelta {
    /// Position of the item in the response's output array.
    pub output_index: i64,
    pub delta: String,
}

/// Data returned by API after response is finished streaming.
#[derive(Clone, Debug, PartialEq)]
pub struct ResponseCompleted {
    pub status: String,
    pub usage: Option<Usage>,
    pub raw_response: Value,
}

/// Data returned by API when a response fails.
#[derive(Clone, Debug, PartialEq)]
pub struct ResponseFailed {
    pub status: String,
    pub error_message: String,
    pub usage: Option<Usage>,
    pub raw_response: Value,
}

/// An event yielded when streaming an inference response.
#[derive(Clone, Debug, PartialEq)]
pub enum InferenceEvent {
    Created(ResponseCreated),
    OutputItemAdded(OutputItemEvent),
    OutputItemDone(OutputItemEvent),
    ReasoningTextDelta(ItemDelta),
    ReasoningSummaryDelta(ItemDelta),
    OutputTextDelta(ItemDelta),
    FunctionCallArgsDelta(ItemDelta),
    Completed(ResponseCompleted),
    Failed(ResponseFailed),
}

/// Wraps around an inference API.
#[derive(Debug)]
pub enum Interface {
    Openai(OpenaiInterface),
}

impl From<OpenaiInterface> for Interface {
    fn from(value: OpenaiInterface) -> Self {
        Self::Openai(value)
    }
}

impl Interface {
    pub fn from_provider(provider: &Provider, client: &DefaultClient) -> AnyResult<Self> {
        let id: InterfaceId = provider.interface.parse()?;
        Ok(match id {
            InterfaceId::Openai | InterfaceId::Openrouter | InterfaceId::Deepseek => {
                let fallback_url = match id {
                    InterfaceId::Openai => "https://api.openai.com/v1",
                    InterfaceId::Openrouter => "https://openrouter.ai/api/v1",
                    InterfaceId::Deepseek => "https://api.deepseek.com",
                };
                Self::from(OpenaiInterface::new(
                    provider
                        .base_url_normalized()
                        .unwrap_or(fallback_url)
                        .to_owned(),
                    provider.api_key.clone(),
                    client.clone(),
                ))
            }
        })
    }

    /// Fetches the list of models offered by the provider.
    pub async fn get_models(&self) -> AnyResult<Vec<InterfaceModel>> {
        match self {
            Self::Openai(iface) => iface.get_models().await,
        }
    }

    pub fn generate(
        &self,
        params: InferenceParams<'_>,
    ) -> impl Stream<Item = AnyResult<InferenceEvent>> + use<> {
        match self {
            Self::Openai(iface) => iface.generate(params),
        }
    }

    /// Whether the interface accepts reasoning ("thought") content in its input.
    pub fn supports_reasoning_input(&self) -> bool {
        match self {
            Self::Openai(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::db;
    use crate::session::{Item, NewItem, Session, Turn, TurnType};
    use diesel::sqlite::SqliteConnection;

    fn create_item(
        conn: &mut SqliteConnection,
        session_id: i32,
        turn_id: i32,
        ty: ItemType,
        text: Option<&str>,
        upstream_call_id: Option<&str>,
    ) -> Item {
        Item::create(
            conn,
            NewItem {
                session_id: Some(session_id),
                turn_id: Some(turn_id),
                ty: Some(ty),
                text,
                upstream_call_id,
                ..Default::default()
            },
        )
        .expect("create item")
    }

    fn create_tool_call(
        conn: &mut SqliteConnection,
        session_id: i32,
        turn_id: i32,
        name: &str,
        call_id: Option<&str>,
        args: Option<&str>,
        output: Option<&Value>,
    ) -> Item {
        let call = create_item(
            conn,
            session_id,
            turn_id,
            ItemType::ToolCall,
            Some(name),
            call_id,
        );
        if let Some(args) = args {
            Item::update_tool_args(conn, call.id, args).expect("update tool args");
        }
        let mut call = Item::get_by_id(conn, call.id)
            .expect("get item")
            .expect("item not found");
        if let Some(output) = output {
            call.set_tool_output(conn, output).expect("set tool output");
        }
        call
    }

    #[test]
    fn test_build_history() {
        let registry = crate::testing::tool_registry();
        let mut conn = db::open_new().expect("failed to open in-memory db");
        let session = Session::create(&mut conn, "Session").expect("create session");
        let turn = Turn::create(&mut conn, session.id, TurnType::Assistant, None, None, None)
            .expect("create turn");

        let user_text =
            create_item(&mut conn, session.id, turn.id, ItemType::UserText, Some("hello"), None);
        let reasoning =
            create_item(&mut conn, session.id, turn.id, ItemType::Reasoning, Some("thinking"), None);
        Item::update_summary(&mut conn, reasoning.id, "summarizing").expect("update summary");
        let response_text = create_item(
            &mut conn,
            session.id,
            turn.id,
            ItemType::ResponseText,
            Some("hi there"),
            None,
        );
        let tool_call = create_tool_call(
            &mut conn,
            session.id,
            turn.id,
            "sh",
            Some("call_1"),
            Some(r#"{"command":"echo hi"}"#),
            Some(&json!({"stdout": "hi\n", "stderr": "", "return_code": 0})),
        );
        let error_tool_call = create_tool_call(
            &mut conn,
            session.id,
            turn.id,
            "cat",
            Some("call_2"),
            Some(r#"{"path":"a.txt"}"#),
            Some(&json!({"error": "file not found"})),
        );
        let orphan_call = create_tool_call(
            &mut conn,
            session.id,
            turn.id,
            "add",
            None,
            None,
            Some(&json!({"result": 3})),
        );
        let interrupted_call = create_tool_call(
            &mut conn,
            session.id,
            turn.id,
            "rm",
            Some("call_3"),
            None,
            None,
        );

        let ids = [
            user_text.id,
            reasoning.id,
            response_text.id,
            tool_call.id,
            error_tool_call.id,
            orphan_call.id,
            interrupted_call.id,
        ];
        let items: Vec<Item> = ids
            .iter()
            .map(|&id| {
                Item::get_by_id(&mut conn, id)
                    .expect("get item")
                    .expect("item not found")
            })
            .collect();

        assert_eq!(
            build_history(&registry, &items, true).unwrap(),
            vec![
                ChatMessage::Message { content: "hello" },
                ChatMessage::Reasoning {
                    content: "summarizing"
                },
                ChatMessage::Response { content: "hi there" },
                ChatMessage::ToolCall {
                    call_id: "call_1",
                    name: "sh",
                    arguments: r#"{"command":"echo hi"}"#,
                },
                ChatMessage::ToolOutput {
                    call_id: "call_1",
                    output: vec![
                        ToolOutputContent::Text {
                            text: Cow::Owned("stdout:\nhi\n".to_owned()),
                        },
                        ToolOutputContent::Text {
                            text: Cow::Owned("return code: 0".to_owned()),
                        },
                    ],
                },
                // Calls to unknown tools render a placeholder
                ChatMessage::ToolCall {
                    call_id: "call_2",
                    name: "cat",
                    arguments: r#"{"path":"a.txt"}"#,
                },
                ChatMessage::ToolOutput {
                    call_id: "call_2",
                    output: vec![ToolOutputContent::Text {
                        text: Cow::Borrowed("error: could not parse output"),
                    }],
                },
                ChatMessage::ToolCall {
                    call_id: "call_3",
                    name: "rm",
                    arguments: "",
                },
                ChatMessage::ToolOutput {
                    call_id: "call_3",
                    output: vec![ToolOutputContent::Text {
                        text: Cow::Borrowed("error: tool call interrupted"),
                    }],
                },
            ]
        );

        assert_eq!(
            build_history(&registry, &items, false).unwrap(),
            vec![
                ChatMessage::Message { content: "hello" },
                ChatMessage::Response { content: "hi there" },
                ChatMessage::ToolCall {
                    call_id: "call_1",
                    name: "sh",
                    arguments: r#"{"command":"echo hi"}"#,
                },
                ChatMessage::ToolOutput {
                    call_id: "call_1",
                    output: vec![
                        ToolOutputContent::Text {
                            text: Cow::Owned("stdout:\nhi\n".to_owned()),
                        },
                        ToolOutputContent::Text {
                            text: Cow::Owned("return code: 0".to_owned()),
                        },
                    ],
                },
                // Calls to unknown tools render a placeholder
                ChatMessage::ToolCall {
                    call_id: "call_2",
                    name: "cat",
                    arguments: r#"{"path":"a.txt"}"#,
                },
                ChatMessage::ToolOutput {
                    call_id: "call_2",
                    output: vec![ToolOutputContent::Text {
                        text: Cow::Borrowed("error: could not parse output"),
                    }],
                },
                ChatMessage::ToolCall {
                    call_id: "call_3",
                    name: "rm",
                    arguments: "",
                },
                ChatMessage::ToolOutput {
                    call_id: "call_3",
                    output: vec![ToolOutputContent::Text {
                        text: Cow::Borrowed("error: tool call interrupted"),
                    }],
                },
            ]
        );
    }
}
