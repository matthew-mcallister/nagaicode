pub mod openai;

use std::error::Error;
use std::str::FromStr;

use futures::Stream;

use crate::error::AnyResult;
use crate::interface::openai::OpenaiInterface;
use crate::provider::Provider;
use crate::request::DefaultClient;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum InterfaceId {
    Openai,
    Openrouter,
    Deepseek,
}

impl FromStr for InterfaceId {
    type Err = Box<dyn Error + Send + Sync>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "openai" => Ok(Self::Openai),
            "openrouter" => Ok(Self::Openrouter),
            "deepseek" => Ok(Self::Deepseek),
            _ => Err(From::from(format!("unknown interface: '{}'", s))),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatMessage<'a> {
    pub role: ChatRole,
    pub content: &'a str,
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
}

/// Token usage
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    /// Total output tokens (reasoning included)
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
}

/// Data returned by API after response is finished streaming.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResponseCompleted {
    pub usage: Option<Usage>,
}

/// An event yielded when streaming an inference response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InferenceEvent {
    Created(ResponseCreated),
    ThinkingDelta(String),
    OutputDelta(String),
    Completed(ResponseCompleted),
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
            InterfaceId::Openai
            | InterfaceId::Openrouter
            | InterfaceId::Deepseek => {
                let fallback_url = match id {
                    InterfaceId::Openai => "https://api.openai.com/v1",
                    InterfaceId::Openrouter => "https://openrouter.ai/api/v1",
                    InterfaceId::Deepseek => "https://api.deepseek.com",
                };
                Self::from(OpenaiInterface::new(
                    provider.base_url_normalized()
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
}
