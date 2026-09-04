pub mod openai;
pub mod stream;

use std::borrow::Cow;
use std::str::FromStr;

use anyhow::anyhow;
use futures::Stream;

use serde_json::Value;

use crate::error::{AnyError, AnyResult};
use crate::interface::openai::OpenaiInterface;
use crate::item::{Item, ItemContent};
use crate::provider::Provider;
use crate::request::DefaultClient;
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
        filepath: Cow<'a, str>,
        // Base64-encoded binary
        #[serde(rename = "file_data")]
        data: Cow<'a, str>,
        // MIME type of the decoded data. Only used to build the data URI
        // accepted by the inference API.
        #[serde(skip)]
        mime: Cow<'a, str>,
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
        /// Name of the called tool.
        name: Cow<'a, str>,
        /// Arguments as JSON. Owned, since it is re-serialized from the
        /// stored value.
        arguments: Cow<'a, str>,
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
/// Tool calls are rendered as a call/output pair. ToolRegistry handles
/// fallback for incomplete/failed calls.
pub fn build_history<'a>(
    tools: &ToolRegistry,
    items: &'a [Item],
    include_reasoning: bool,
) -> Vec<ChatMessage<'a>> {
    let mut messages = Vec::with_capacity(items.len());
    for item in items {
        match &item.content {
            ItemContent::UserText(text) => {
                messages.push(ChatMessage::Message { content: text });
            }
            ItemContent::ResponseText(text) => {
                messages.push(ChatMessage::Response { content: text });
            }
            ItemContent::Reasoning(content) => {
                if !include_reasoning {
                    continue;
                }
                let text = content.summary.as_deref().or(content.text.as_deref());
                if let Some(text) = text {
                    messages.push(ChatMessage::Reasoning { content: text });
                }
            }
            ItemContent::ToolCall(content) => {
                let output = tools.render_to_interface(content);
                messages.push(ChatMessage::ToolCall {
                    call_id: &content.call_id,
                    name: content.tool_name.as_str().into(),
                    arguments: content.args.to_string().into(),
                });
                messages.push(ChatMessage::ToolOutput {
                    call_id: &content.call_id,
                    output: output.content,
                });
            }
        }
    }
    messages
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
#[derive(Clone, Debug, PartialEq, Default)]
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
    /// Tool name
    pub tool_name: Option<String>,
    /// Tool call arguments
    pub tool_args: Option<String>,
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
    use crate::item::{NewItem, ReasoningContent, ToolOutput};
    use crate::testing::{session_turn, tool_call, tool_registry};
    use diesel::sqlite::SqliteConnection;

    fn create_item(
        conn: &mut SqliteConnection,
        session_id: i32,
        turn_id: i32,
        content: ItemContent,
    ) -> Item {
        Item::create(
            conn,
            NewItem {
                session_id,
                turn_id,
                response_id: None,
                provider_id: None,
                upstream_id: None,
                seqno: None,
                content,
            },
        ).unwrap()
    }

    #[test]
    fn test_build_history() {
        let registry = tool_registry();
        let mut conn = db::open_new().unwrap();
        let (_, turn) = session_turn(&mut conn);
        let mut create = |content| create_item(&mut conn, turn.session_id, turn.id, content);

        let user_text = create(ItemContent::UserText("hello".to_owned()));
        let reasoning = create(ItemContent::Reasoning(ReasoningContent {
            text: Some("thinking".to_owned()),
            summary: Some("summarizing".to_owned()),
            encrypted: None,
        }));
        let response_text = create(ItemContent::ResponseText("hi there".to_owned()));
        let sh_item = tool_call(
            &mut conn,
            &turn,
            "sh",
            "call_1",
            json!({ "command": "echo hi" }),
            Some(ToolOutput::Completed {
                value: json!({ "stdout": "hi\n", "stderr": "", "return_code": 0 }),
            }),
        );
        // Calls to unknown tools render a placeholder
        let cat_item = tool_call(
            &mut conn,
            &turn,
            "cat",
            "call_2",
            json!({ "path": "a.txt" }),
            Some(ToolOutput::Completed {
                value: json!({ "error": "file not found" }),
            }),
        );
        // A failed call records its error as output; the model still sees the
        // name it called.
        let failed_item = tool_call(
            &mut conn,
            &turn,
            "sh",
            "call_3",
            json!({ "command": 123 }),
            Some(ToolOutput::Failed { error: "boom".to_owned() }),
        );

        let items = [
            user_text,
            reasoning,
            response_text,
            sh_item,
            cat_item,
            failed_item,
        ];

        let sh_call = ChatMessage::ToolCall {
            call_id: "call_1",
            name: Cow::Borrowed("sh"),
            arguments: r#"{"command":"echo hi"}"#.into(),
        };
        let sh_output = ChatMessage::ToolOutput {
            call_id: "call_1",
            output: vec![
                ToolOutputContent::Text { text: Cow::Owned("stdout:\nhi\n".to_owned()) },
                ToolOutputContent::Text { text: Cow::Owned("return code: 0".to_owned()) },
            ],
        };

        assert_eq!(
            build_history(&registry, &items, true),
            vec![
                ChatMessage::Message { content: "hello" },
                ChatMessage::Reasoning { content: "summarizing" },
                ChatMessage::Response { content: "hi there" },
                sh_call.clone(),
                sh_output.clone(),
                ChatMessage::ToolCall {
                    call_id: "call_2",
                    name: Cow::Borrowed("cat"),
                    arguments: r#"{"path":"a.txt"}"#.into(),
                },
                ChatMessage::ToolOutput {
                    call_id: "call_2",
                    output: vec![ToolOutputContent::Text {
                        text: Cow::Borrowed("error: could not parse output"),
                    }],
                },
                ChatMessage::ToolCall {
                    call_id: "call_3",
                    name: Cow::Borrowed("sh"),
                    arguments: r#"{"command":123}"#.into(),
                },
                ChatMessage::ToolOutput {
                    call_id: "call_3",
                    output: vec![ToolOutputContent::Text {
                        text: Cow::Owned("error: boom".to_owned()),
                    }],
                },
            ]
        );

        assert_eq!(
            build_history(&registry, &items, false),
            vec![
                ChatMessage::Message { content: "hello" },
                ChatMessage::Response { content: "hi there" },
                sh_call,
                sh_output,
                ChatMessage::ToolCall {
                    call_id: "call_2",
                    name: Cow::Borrowed("cat"),
                    arguments: r#"{"path":"a.txt"}"#.into(),
                },
                ChatMessage::ToolOutput {
                    call_id: "call_2",
                    output: vec![ToolOutputContent::Text {
                        text: Cow::Borrowed("error: could not parse output"),
                    }],
                },
                ChatMessage::ToolCall {
                    call_id: "call_3",
                    name: Cow::Borrowed("sh"),
                    arguments: r#"{"command":123}"#.into(),
                },
                ChatMessage::ToolOutput {
                    call_id: "call_3",
                    output: vec![ToolOutputContent::Text {
                        text: Cow::Owned("error: boom".to_owned()),
                    }],
                },
            ]
        );
    }
}
