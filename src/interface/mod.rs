pub mod openai;
pub mod stream;

use std::str::FromStr;

use anyhow::anyhow;
use futures::Stream;

use serde_json::Value;

use crate::error::{AnyError, AnyResult};
use crate::interface::openai::OpenaiInterface;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::{Item, ItemType};
use crate::tool::ToolServer;

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
    ToolOutput { call_id: &'a str, output: &'a str },
}

/// Builds the conversation history from a session's items.
///
/// Text items are always included; reasoning items are only included when
/// `include_reasoning` is true, preferring the summary over the raw text.
/// Tool call and output items are only included when they carry an upstream
/// call id, which the API requires to correlate them. Incomplete tool outputs
/// render with a fixed tool-call-interrupted error message.
pub fn build_history<'a>(
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
                    continue;
                };
                messages.push(ChatMessage::ToolCall {
                    call_id,
                    name,
                    arguments: item.json.as_deref().unwrap_or(""),
                });
            }
            ItemType::ToolOutput => {
                let Some(call_id) = item.upstream_call_id.as_deref() else {
                    continue;
                };
                let output = if item.completed {
                    item.json.as_deref().or(item.text.as_deref()).unwrap_or("")
                } else {
                    "error: tool call interrupted"
                };
                messages.push(ChatMessage::ToolOutput { call_id, output });
            }
        }
    }
    Ok(messages)
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

    pub fn generate<T: ToolServer + ?Sized>(
        &self,
        params: InferenceParams<'_>,
        tools: &T,
    ) -> impl Stream<Item = AnyResult<InferenceEvent>> + use<T> {
        match self {
            Self::Openai(iface) => iface.generate(params, tools),
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
                completed: Some(true),
                ..Default::default()
            },
        )
        .expect("create item")
    }

    #[test]
    fn test_build_history() {
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
        let tool_call =
            create_item(&mut conn, session.id, turn.id, ItemType::ToolCall, Some("add"), Some("call_1"));
        Item::update_json(&mut conn, tool_call.id, r#"{"a":1}"#).expect("update json");
        let tool_output = create_item(
            &mut conn,
            session.id,
            turn.id,
            ItemType::ToolOutput,
            None,
            Some("call_1"),
        );
        Item::update_json(&mut conn, tool_output.id, r#"{"result":3}"#).expect("update json");
        let text_tool_call = create_item(
            &mut conn,
            session.id,
            turn.id,
            ItemType::ToolCall,
            Some("cat"),
            Some("call_2"),
        );
        Item::update_json(&mut conn, text_tool_call.id, r#"{"path":"a.txt"}"#).expect("update json");
        let text_tool_output = create_item(
            &mut conn,
            session.id,
            turn.id,
            ItemType::ToolOutput,
            Some("file contents"),
            Some("call_2"),
        );
        let orphan_call =
            create_item(&mut conn, session.id, turn.id, ItemType::ToolCall, Some("add"), None);
        let orphan_output =
            create_item(&mut conn, session.id, turn.id, ItemType::ToolOutput, Some("add"), None);
        let interrupted_call = create_item(
            &mut conn,
            session.id,
            turn.id,
            ItemType::ToolCall,
            Some("rm"),
            Some("call_3"),
        );
        let interrupted_output = Item::create(
            &mut conn,
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(turn.id),
                ty: Some(ItemType::ToolOutput),
                upstream_call_id: Some("call_3"),
                completed: Some(false),
                ..Default::default()
            },
        )
        .expect("create item");

        let ids = [
            user_text.id,
            reasoning.id,
            response_text.id,
            tool_call.id,
            tool_output.id,
            text_tool_call.id,
            text_tool_output.id,
            orphan_call.id,
            orphan_output.id,
            interrupted_call.id,
            interrupted_output.id,
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
            build_history(&items, true).unwrap(),
            vec![
                ChatMessage::Message { content: "hello" },
                ChatMessage::Reasoning {
                    content: "summarizing"
                },
                ChatMessage::Response { content: "hi there" },
                ChatMessage::ToolCall {
                    call_id: "call_1",
                    name: "add",
                    arguments: r#"{"a":1}"#,
                },
                ChatMessage::ToolOutput {
                    call_id: "call_1",
                    output: r#"{"result":3}"#,
                },
                ChatMessage::ToolCall {
                    call_id: "call_2",
                    name: "cat",
                    arguments: r#"{"path":"a.txt"}"#,
                },
                ChatMessage::ToolOutput {
                    call_id: "call_2",
                    output: "file contents",
                },
                ChatMessage::ToolCall {
                    call_id: "call_3",
                    name: "rm",
                    arguments: "",
                },
                ChatMessage::ToolOutput {
                    call_id: "call_3",
                    output: "error: tool call interrupted",
                },
            ]
        );

        assert_eq!(
            build_history(&items, false).unwrap(),
            vec![
                ChatMessage::Message { content: "hello" },
                ChatMessage::Response { content: "hi there" },
                ChatMessage::ToolCall {
                    call_id: "call_1",
                    name: "add",
                    arguments: r#"{"a":1}"#,
                },
                ChatMessage::ToolOutput {
                    call_id: "call_1",
                    output: r#"{"result":3}"#,
                },
                ChatMessage::ToolCall {
                    call_id: "call_2",
                    name: "cat",
                    arguments: r#"{"path":"a.txt"}"#,
                },
                ChatMessage::ToolOutput {
                    call_id: "call_2",
                    output: "file contents",
                },
                ChatMessage::ToolCall {
                    call_id: "call_3",
                    name: "rm",
                    arguments: "",
                },
                ChatMessage::ToolOutput {
                    call_id: "call_3",
                    output: "error: tool call interrupted",
                },
            ]
        );
    }
}
