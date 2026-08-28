pub mod openai;

use std::str::FromStr;

use anyhow::anyhow;
use futures::Stream;

use serde_json::Value;

use crate::error::{AnyError, AnyResult};
use crate::interface::openai::OpenaiInterface;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::{Item, ItemType};

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
}

/// Builds the conversation history from a session's items.
///
/// Text items are always included; reasoning items are only included when
/// `include_reasoning` is true, preferring the summary over the raw text.
/// Tool call and output items are ignored for now.
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
            ItemType::ToolCall | ItemType::ToolOutput => {}
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
