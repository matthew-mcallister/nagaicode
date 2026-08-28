use futures::{Stream, StreamExt};
use log::debug;
use reqwest_eventsource::Event;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::AnyResult;
use crate::interface::{
    ChatMessage, InferenceEvent, InferenceParams, InterfaceModel, ItemDelta, OutputItemEvent,
    ReasoningEffort, ResponseCompleted, ResponseCreated, ResponseFailed, Usage,
};
use crate::request::DefaultClient;
#[allow(unused_imports)]
use crate::request::{Client as _, Response as _};

/// HTTP client wrapping a [`DefaultClient`] with auth and a base URL.
#[derive(Clone, Debug)]
pub struct Client {
    base_url: String,
    api_key: String,
    inner: DefaultClient,
    builder: reqwest::Client,
}

impl Client {
    pub fn new(base_url: String, api_key: String, inner: DefaultClient) -> Self {
        Self {
            base_url,
            api_key,
            inner,
            builder: reqwest::Client::new(),
        }
    }

    /// Returns the underlying client.
    pub fn inner(&self) -> &DefaultClient {
        &self.inner
    }

    /// Performs an authenticated GET and deserializes the JSON body.
    pub async fn get<T: DeserializeOwned>(&self, endpoint: &str) -> AnyResult<T> {
        let url = format!("{}{}", self.base_url, endpoint);
        let request = self
            .builder
            .get(url.clone())
            .bearer_auth(&self.api_key)
            .build()?;
        debug!("GET {}", request.url());
        let response = self.inner.execute(request).await?;
        let response = response.error_for_status()?;
        let status = response.status();
        let body = response.text().await?;
        debug!("GET {url} -> {status} {body}");
        Ok(serde_json::from_str::<T>(&body)?)
    }

    /// Performs an authenticated POST and returns the SSE event stream.
    pub fn post_sse<T: Serialize>(
        &self,
        endpoint: &str,
        body: &T,
    ) -> AnyResult<impl Stream<Item = AnyResult<Event>> + use<'_, T>> {
        let request = self
            .builder
            .post(format!("{}{}", self.base_url, endpoint))
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .json(body)
            .build()?;
        let request_body = request.body().and_then(|b| b.as_bytes()).unwrap_or(&[]);
        debug!(
            "POST {} {}",
            request.url(),
            String::from_utf8(request_body.to_vec())?,
        );
        Ok(self.inner.stream(request))
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
enum RequestReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl From<ReasoningEffort> for RequestReasoningEffort {
    fn from(effort: ReasoningEffort) -> Self {
        match effort {
            ReasoningEffort::None => Self::None,
            ReasoningEffort::Minimal => Self::Minimal,
            ReasoningEffort::Low => Self::Low,
            ReasoningEffort::Medium => Self::Medium,
            ReasoningEffort::High => Self::High,
            ReasoningEffort::Xhigh => Self::Xhigh,
            ReasoningEffort::Max => Self::Max,
        }
    }
}

#[derive(Debug, Serialize)]
struct RequestReasoning {
    effort: RequestReasoningEffort,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum InputItem<'a> {
    Message {
        role: &'static str,
        content: &'a str,
    },
    Reasoning {
        #[serde(rename = "type")]
        r#type: &'static str,
        summary: Vec<ReasoningSummary<'a>>,
    },
}

#[derive(Debug, Serialize)]
struct ReasoningSummary<'a> {
    #[serde(rename = "type")]
    r#type: &'static str,
    text: &'a str,
}

#[derive(Debug, Serialize)]
struct CreateResponseRequest<'a> {
    model: &'a str,
    input: Vec<InputItem<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<RequestReasoning>,
    stream: bool,
}

impl<'a> CreateResponseRequest<'a> {
    fn from_params(params: &InferenceParams<'a>) -> Self {
        let input = params
            .input
            .iter()
            .map(|msg| match msg {
                ChatMessage::Message { content } => InputItem::Message {
                    role: "user",
                    content,
                },
                ChatMessage::Response { content } => InputItem::Message {
                    role: "assistant",
                    content,
                },
                ChatMessage::Reasoning { content } => InputItem::Reasoning {
                    r#type: "reasoning",
                    summary: vec![ReasoningSummary {
                        r#type: "summary_text",
                        text: content,
                    }],
                },
            })
            .collect();

        let instructions = if params.system_prompt.is_empty() {
            None
        } else {
            Some(params.system_prompt)
        };

        Self {
            model: params.model_id,
            input,
            instructions,
            temperature: Some(params.temperature),
            reasoning: params.reasoning_effort.map(|effort| RequestReasoning {
                effort: effort.into(),
            }),
            stream: true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ResponseStreamEvent {
    #[serde(rename = "response.created")]
    Created { response: ResponsePayload },
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        #[serde(default)]
        output_index: i64,
        item: OutputItemPayload,
    },
    #[serde(rename = "response.output_item.done")]
    OutputItemDone {
        #[serde(default)]
        output_index: i64,
        item: OutputItemPayload,
    },
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        #[serde(default)]
        output_index: i64,
        delta: String,
    },
    #[serde(rename = "response.reasoning_text.delta")]
    ReasoningTextDelta {
        #[serde(default)]
        output_index: i64,
        delta: String,
    },
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryTextDelta {
        #[serde(default)]
        output_index: i64,
        delta: String,
    },
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgsDelta {
        #[serde(default)]
        output_index: i64,
        delta: String,
    },
    #[serde(rename = "response.completed")]
    Completed { response: ResponsePayload },
    #[serde(rename = "response.incomplete")]
    Incomplete { response: ResponsePayload },
    #[serde(rename = "response.failed")]
    Failed { response: ResponsePayload },
    #[serde(rename = "error")]
    Error(ResponseError),
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct OutputItemPayload {
    #[serde(default)]
    id: String,
    #[serde(default, rename = "type")]
    ty: String,
}

#[derive(Debug, Deserialize)]
struct ResponsePayload {
    #[serde(default)]
    id: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    usage: Option<ResponseUsage>,
    #[serde(default)]
    error: Option<ResponseError>,
}

#[derive(Debug, Deserialize, Default)]
struct ResponseUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    total_tokens: u64,
    #[serde(default)]
    input_tokens_details: InputTokensDetails,
    #[serde(default)]
    output_tokens_details: OutputTokensDetails,
}

#[derive(Debug, Default, Deserialize)]
struct InputTokensDetails {
    #[serde(default)]
    cached_tokens: u64,
}

#[derive(Debug, Default, Deserialize)]
struct OutputTokensDetails {
    #[serde(default)]
    reasoning_tokens: u64,
}

impl From<ResponseUsage> for Usage {
    fn from(value: ResponseUsage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            cached_input_tokens: value.input_tokens_details.cached_tokens,
            output_tokens: value.output_tokens,
            reasoning_tokens: value.output_tokens_details.reasoning_tokens,
            total_tokens: value.total_tokens,
        }
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
struct ResponseError {
    message: String,
    #[allow(dead_code)]
    #[serde(default)]
    code: Option<String>,
}

impl std::fmt::Display for ResponseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ResponseError {}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
}

#[derive(Debug)]
pub struct OpenaiInterface {
    client: Client,
}

impl OpenaiInterface {
    pub fn new(base_url: String, api_key: String, client: DefaultClient) -> Self {
        Self {
            client: Client::new(base_url, api_key, client),
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub async fn get_models(&self) -> AnyResult<Vec<InterfaceModel>> {
        let parsed: ModelsResponse = self.client.get("/models").await?;
        Ok(parsed
            .data
            .into_iter()
            .map(|m| InterfaceModel { id: m.id })
            .collect())
    }

    // FIXME: Some providers emit output text events before reasoning events
    // causing the UI to display reasoning *after* the response. Need to do
    // some field study to determine how we can reorder these by buffering here
    // and if that's a better design than reordering on UI side.
    pub fn generate(
        &self,
        params: InferenceParams<'_>,
    ) -> impl Stream<Item = AnyResult<InferenceEvent>> + use<> {
        let req_body = serde_json::to_value(CreateResponseRequest::from_params(&params)).unwrap();
        let client = self.client.clone();

        async_stream::try_stream! {
            let event_stream_result = client.post_sse("/responses", &req_body);

            let event_source = event_stream_result?;
            futures::pin_mut!(event_source);

            while let Some(event_res) = event_source.next().await {
                match event_res? {
                    Event::Open => continue,
                    Event::Message(msg) => {
                        let trimmed = msg.data.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if trimmed == "[DONE]" {
                            return;
                        }
                        debug!("SSE event {}", trimmed);
                        let raw_event: serde_json::Value = serde_json::from_str(trimmed)?;
                        let event: ResponseStreamEvent =
                            serde_json::from_value(raw_event.clone())?;
                        match event {
                            ResponseStreamEvent::Created { response } => {
                                yield InferenceEvent::Created(ResponseCreated {
                                    id: response.id,
                                    status: response.status
                                        .unwrap_or_else(|| "in_progress".into()),
                                });
                            }
                            ResponseStreamEvent::OutputItemAdded { output_index, item } => {
                                yield InferenceEvent::OutputItemAdded(OutputItemEvent {
                                    output_index,
                                    id: item.id,
                                    ty: item.ty,
                                    raw: raw_event["item"].clone(),
                                });
                            }
                            ResponseStreamEvent::OutputItemDone { output_index, item } => {
                                yield InferenceEvent::OutputItemDone(OutputItemEvent {
                                    output_index,
                                    id: item.id,
                                    ty: item.ty,
                                    raw: raw_event["item"].clone(),
                                });
                            }
                            ResponseStreamEvent::OutputTextDelta { output_index, delta } => {
                                yield InferenceEvent::OutputTextDelta(ItemDelta {
                                    output_index,
                                    delta,
                                });
                            }
                            ResponseStreamEvent::ReasoningTextDelta { output_index, delta } => {
                                yield InferenceEvent::ReasoningTextDelta(ItemDelta {
                                    output_index,
                                    delta,
                                });
                            }
                            ResponseStreamEvent::ReasoningSummaryTextDelta {
                                output_index,
                                delta,
                            } => {
                                yield InferenceEvent::ReasoningSummaryDelta(ItemDelta {
                                    output_index,
                                    delta,
                                });
                            }
                            ResponseStreamEvent::FunctionCallArgsDelta {
                                output_index,
                                delta,
                            } => {
                                yield InferenceEvent::FunctionCallArgsDelta(ItemDelta {
                                    output_index,
                                    delta,
                                });
                            }
                            ResponseStreamEvent::Completed { response } => {
                                yield InferenceEvent::Completed(ResponseCompleted {
                                    status: response
                                        .status
                                        .unwrap_or_else(|| "completed".into()),
                                    usage: response.usage.map(Usage::from),
                                    raw_response: raw_event["response"].clone(),
                                });
                            }
                            ResponseStreamEvent::Incomplete { response } => {
                                yield InferenceEvent::Completed(ResponseCompleted {
                                    status: response
                                        .status
                                        .unwrap_or_else(|| "incomplete".into()),
                                    usage: response.usage.map(Usage::from),
                                    raw_response: raw_event["response"].clone(),
                                });
                            }
                            ResponseStreamEvent::Failed { response } => {
                                yield InferenceEvent::Failed(ResponseFailed {
                                    status: response
                                        .status
                                        .unwrap_or_else(|| "failed".into()),
                                    error_message: response
                                        .error
                                        .as_ref()
                                        .map(|e| e.message.clone())
                                        .unwrap_or_else(|| "response failed".into()),
                                    usage: response.usage.map(Usage::from),
                                    raw_response: raw_event["response"].clone(),
                                });
                                return;
                            }
                            ResponseStreamEvent::Error(error) => {
                                Err(error)?;
                            }
                            ResponseStreamEvent::Unknown => {}
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::error::AnyResult;
    use crate::interface::ChatMessage;
    use crate::request::DefaultClient;
    use crate::request::test_client::{Response, ResponseData};
    use crate::testing::QueueStream;
    use reqwest::StatusCode;
    use reqwest::header::HeaderMap;
    use reqwest_eventsource::Event;

    const BASE_URL: &str = "https://example.test/v1";

    fn make_iface(client: DefaultClient) -> OpenaiInterface {
        OpenaiInterface::new(BASE_URL.into(), "sk-test".into(), client)
    }

    fn create_message_event(data: &str) -> Event {
        Event::Message(eventsource_stream::Event {
            event: "message".to_string(),
            data: data.to_string(),
            id: "".to_string(),
            retry: None,
        })
    }

    fn default_events() -> Vec<AnyResult<Event>> {
        vec![
            Ok(Event::Open),
            Ok(create_message_event(
                r#"{"type":"response.created","response":{"id":"resp-mock-1","status":"in_progress"}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_1","type":"message","status":"in_progress","role":"assistant","content":[]}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"delta":"Hello! "}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"delta":"How can I help you today?"}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_1","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Hello! How can I help you today?"}]}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.completed","response":{"id":"resp-mock-1","status":"completed","usage":{"input_tokens":12,"output_tokens":18,"total_tokens":30,"input_tokens_details":{"cached_tokens":4},"output_tokens_details":{"reasoning_tokens":7}}}}"#,
            )),
            Ok(create_message_event("[DONE]")),
        ]
    }

    fn http_ok(body: &str) -> ResponseData {
        ResponseData::Http(Ok(Response {
            body: body.to_owned(),
            status: StatusCode::OK,
            headers: HeaderMap::new(),
        }))
    }

    fn sse(events: Vec<AnyResult<Event>>) -> ResponseData {
        ResponseData::Sse(QueueStream::from(events))
    }

    fn request_body_value(req: &reqwest::Request) -> serde_json::Value {
        let bytes = req.body().and_then(|b| b.as_bytes()).unwrap_or(&[]);
        serde_json::from_slice(bytes).unwrap()
    }

    // Also tests get_models
    #[tokio::test]
    async fn test_list_models() {
        let mut client = DefaultClient::default();
        client.add_response(
            &format!("{BASE_URL}/models"),
            http_ok(r#"{"data":[{"id":"fake-model"}]}"#),
        );

        let iface = make_iface(client);
        let models = iface.get_models().await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "fake-model");

        let requests = iface.client().inner().get_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method(), reqwest::Method::GET);
        assert_eq!(requests[0].url().as_str(), &format!("{BASE_URL}/models"));
    }

    #[tokio::test]
    async fn test_generate() {
        let mut client = DefaultClient::default();
        client.add_response(&format!("{BASE_URL}/responses"), sse(default_events()));

        let iface = make_iface(client);
        let params = InferenceParams {
            model_id: "gpt-4o",
            system_prompt: "You are a helpful assistant.",
            temperature: 0.75,
            reasoning_effort: Some(ReasoningEffort::Low),
            input: &[
                ChatMessage::Message { content: "Hello" },
                ChatMessage::Response {
                    content: "Hi there!",
                },
                ChatMessage::Message {
                    content: "How are you?",
                },
            ],
        };

        let stream = iface.generate(params);
        let events: Vec<InferenceEvent> = stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect();

        assert_eq!(
            events,
            vec![
                InferenceEvent::Created(ResponseCreated {
                    id: "resp-mock-1".into(),
                    status: "in_progress".into(),
                }),
                InferenceEvent::OutputItemAdded(OutputItemEvent {
                    output_index: 0,
                    id: "msg_1".into(),
                    ty: "message".into(),
                    raw: json!({
                        "id": "msg_1",
                        "type": "message",
                        "status": "in_progress",
                        "role": "assistant",
                        "content": [],
                    }),
                }),
                InferenceEvent::OutputTextDelta(ItemDelta {
                    output_index: 0,
                    delta: "Hello! ".into(),
                }),
                InferenceEvent::OutputTextDelta(ItemDelta {
                    output_index: 0,
                    delta: "How can I help you today?".into(),
                }),
                InferenceEvent::OutputItemDone(OutputItemEvent {
                    output_index: 0,
                    id: "msg_1".into(),
                    ty: "message".into(),
                    raw: json!({
                        "id": "msg_1",
                        "type": "message",
                        "status": "completed",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "Hello! How can I help you today?"}],
                    }),
                }),
                InferenceEvent::Completed(ResponseCompleted {
                    status: "completed".into(),
                    usage: Some(Usage {
                        input_tokens: 12,
                        cached_input_tokens: 4,
                        output_tokens: 18,
                        reasoning_tokens: 7,
                        total_tokens: 30,
                    }),
                    raw_response: json!({
                        "id": "resp-mock-1",
                        "status": "completed",
                        "usage": {
                            "input_tokens": 12,
                            "output_tokens": 18,
                            "total_tokens": 30,
                            "input_tokens_details": {"cached_tokens": 4},
                            "output_tokens_details": {"reasoning_tokens": 7},
                        },
                    }),
                }),
            ]
        );

        let requests = iface.client().inner().get_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method(), reqwest::Method::POST);
        assert_eq!(requests[0].url().as_str(), &format!("{BASE_URL}/responses"));

        let body = request_body_value(&requests[0]);
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["temperature"], 0.75);
        assert_eq!(body["reasoning"]["effort"], "low");
        assert_eq!(body["stream"], true);
        assert_eq!(body["instructions"], "You are a helpful assistant.");
        assert_eq!(
            body["input"],
            serde_json::json!([
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there!"},
                {"role": "user", "content": "How are you?"}
            ])
        );
    }

    #[tokio::test]
    async fn test_missing_usage() {
        let mut client = DefaultClient::default();
        client.add_response(
            &format!("{BASE_URL}/responses"),
            sse(vec![
                Ok(create_message_event(
                    r#"{"type":"response.completed","response":{"id":"resp-custom-1"}}"#,
                )),
                Ok(create_message_event("[DONE]")),
            ]),
        );

        let iface = make_iface(client);
        let params = InferenceParams {
            model_id: "test-model",
            system_prompt: "",
            temperature: 0.0,
            reasoning_effort: None,
            input: &[],
        };

        let stream = iface.generate(params);
        let events: Vec<InferenceEvent> = stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect();

        assert_eq!(
            events,
            vec![InferenceEvent::Completed(ResponseCompleted {
                status: "completed".into(),
                usage: None,
                raw_response: json!({"id": "resp-custom-1"}),
            })]
        );
    }

    #[tokio::test]
    async fn test_thinking_fields() {
        let mut client = DefaultClient::default();
        client.add_response(
            &format!("{BASE_URL}/responses"),
            sse(vec![
                Ok(create_message_event(
                    r#"{"type":"response.created","response":{"id":"resp-think-1","status":"in_progress"}}"#,
                )),
                Ok(create_message_event(
                    r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[]}}"#,
                )),
                Ok(create_message_event(
                    r#"{"type":"response.reasoning_text.delta","item_id":"rs_1","output_index":0,"delta":"step 1"}"#,
                )),
                Ok(create_message_event(
                    r#"{"type":"response.reasoning_summary_text.delta","item_id":"rs_1","output_index":0,"delta":"step 2"}"#,
                )),
                Ok(create_message_event(
                    r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[{"type":"summary_text","text":"step 2"}]}}"#,
                )),
                Ok(create_message_event(
                    r#"{"type":"response.output_item.added","output_index":1,"item":{"id":"msg_1","type":"message","status":"in_progress","role":"assistant","content":[]}}"#,
                )),
                Ok(create_message_event(
                    r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":1,"delta":"done"}"#,
                )),
                Ok(create_message_event(
                    r#"{"type":"response.output_item.done","output_index":1,"item":{"id":"msg_1","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"done"}]}}"#,
                )),
                Ok(create_message_event(
                    r#"{"type":"response.completed","response":{"id":"resp-think-1","status":"completed"}}"#,
                )),
                Ok(create_message_event("[DONE]")),
            ]),
        );

        let iface = make_iface(client);
        let params = InferenceParams {
            model_id: "test-model",
            system_prompt: "",
            temperature: 0.0,
            reasoning_effort: None,
            input: &[],
        };

        let stream = iface.generate(params);
        let events: Vec<InferenceEvent> = stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect();

        assert_eq!(
            events,
            vec![
                InferenceEvent::Created(ResponseCreated {
                    id: "resp-think-1".into(),
                    status: "in_progress".into(),
                }),
                InferenceEvent::OutputItemAdded(OutputItemEvent {
                    output_index: 0,
                    id: "rs_1".into(),
                    ty: "reasoning".into(),
                    raw: json!({"id": "rs_1", "type": "reasoning", "summary": []}),
                }),
                InferenceEvent::ReasoningTextDelta(ItemDelta {
                    output_index: 0,
                    delta: "step 1".into(),
                }),
                InferenceEvent::ReasoningSummaryDelta(ItemDelta {
                    output_index: 0,
                    delta: "step 2".into(),
                }),
                InferenceEvent::OutputItemDone(OutputItemEvent {
                    output_index: 0,
                    id: "rs_1".into(),
                    ty: "reasoning".into(),
                    raw: json!({
                        "id": "rs_1",
                        "type": "reasoning",
                        "summary": [{"type": "summary_text", "text": "step 2"}],
                    }),
                }),
                InferenceEvent::OutputItemAdded(OutputItemEvent {
                    output_index: 1,
                    id: "msg_1".into(),
                    ty: "message".into(),
                    raw: json!({
                        "id": "msg_1",
                        "type": "message",
                        "status": "in_progress",
                        "role": "assistant",
                        "content": [],
                    }),
                }),
                InferenceEvent::OutputTextDelta(ItemDelta {
                    output_index: 1,
                    delta: "done".into(),
                }),
                InferenceEvent::OutputItemDone(OutputItemEvent {
                    output_index: 1,
                    id: "msg_1".into(),
                    ty: "message".into(),
                    raw: json!({
                        "id": "msg_1",
                        "type": "message",
                        "status": "completed",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": "done"}],
                    }),
                }),
                InferenceEvent::Completed(ResponseCompleted {
                    status: "completed".into(),
                    usage: None,
                    raw_response: json!({"id": "resp-think-1", "status": "completed"}),
                }),
            ]
        );
    }

    #[tokio::test]
    async fn test_tool_call_fields() {
        let mut client = DefaultClient::default();
        client.add_response(
            &format!("{BASE_URL}/responses"),
            sse(vec![
                Ok(create_message_event(
                    r#"{"type":"response.created","response":{"id":"resp-tool-1","status":"in_progress"}}"#,
                )),
                Ok(create_message_event(
                    r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"fc_1","type":"function_call","status":"in_progress","name":"get_weather","call_id":"call_1","arguments":""}}"#,
                )),
                Ok(create_message_event(
                    r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":"{\"city\":\"San Francisco\""}"#,
                )),
                Ok(create_message_event(
                    r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":"}"}"#,
                )),
                Ok(create_message_event(
                    r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"fc_1","type":"function_call","status":"completed","name":"get_weather","call_id":"call_1","arguments":"{\"city\":\"San Francisco\"}"}}"#,
                )),
                Ok(create_message_event(
                    r#"{"type":"response.completed","response":{"id":"resp-tool-1","status":"completed"}}"#,
                )),
                Ok(create_message_event("[DONE]")),
            ]),
        );

        let iface = make_iface(client);
        let params = InferenceParams {
            model_id: "test-model",
            system_prompt: "",
            temperature: 0.0,
            reasoning_effort: None,
            input: &[],
        };

        let stream = iface.generate(params);
        let events: Vec<InferenceEvent> = stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect();

        assert_eq!(
            events,
            vec![
                InferenceEvent::Created(ResponseCreated {
                    id: "resp-tool-1".into(),
                    status: "in_progress".into(),
                }),
                InferenceEvent::OutputItemAdded(OutputItemEvent {
                    output_index: 0,
                    id: "fc_1".into(),
                    ty: "function_call".into(),
                    raw: json!({
                        "id": "fc_1",
                        "type": "function_call",
                        "status": "in_progress",
                        "name": "get_weather",
                        "call_id": "call_1",
                        "arguments": "",
                    }),
                }),
                InferenceEvent::FunctionCallArgsDelta(ItemDelta {
                    output_index: 0,
                    delta: r#"{"city":"San Francisco""#.into(),
                }),
                InferenceEvent::FunctionCallArgsDelta(ItemDelta {
                    output_index: 0,
                    delta: "}".into(),
                }),
                InferenceEvent::OutputItemDone(OutputItemEvent {
                    output_index: 0,
                    id: "fc_1".into(),
                    ty: "function_call".into(),
                    raw: json!({
                        "id": "fc_1",
                        "type": "function_call",
                        "status": "completed",
                        "name": "get_weather",
                        "call_id": "call_1",
                        "arguments": r#"{"city":"San Francisco"}"#,
                    }),
                }),
                InferenceEvent::Completed(ResponseCompleted {
                    status: "completed".into(),
                    usage: None,
                    raw_response: json!({"id": "resp-tool-1", "status": "completed"}),
                }),
            ]
        );
    }

    #[tokio::test]
    async fn test_error() {
        let mut client = DefaultClient::default();
        let iface = make_iface(client.clone());
        let url = format!("{BASE_URL}/responses");

        // A top-level error event surfaces as a stream error.
        client.add_response(&url, sse(vec![Ok(create_message_event(
            r#"{"type":"error","message":"Incorrect API key provided","code":"invalid_api_key"}"#,
        ))]));

        let params = InferenceParams {
            model_id: "test-model",
            system_prompt: "",
            temperature: 0.0,
            reasoning_effort: None,
            input: &[],
        };

        let stream = iface.generate(params);
        let results: Vec<_> = stream.collect().await;
        assert_eq!(results.len(), 1);
        assert!(results[0].is_err());
        assert_eq!(
            results[0].as_ref().unwrap_err().to_string(),
            "Incorrect API key provided"
        );

        // A failed response surfaces as a Failed event carrying the raw
        // response payload.
        client.add_response(&url, sse(vec![Ok(create_message_event(
            r#"{"type":"response.failed","response":{"id":"resp-1","status":"failed","error":{"message":"Incorrect API key provided","code":"invalid_api_key"}}}"#,
        ))]));

        let params = InferenceParams {
            model_id: "test-model",
            system_prompt: "",
            temperature: 0.0,
            reasoning_effort: None,
            input: &[],
        };

        let stream = iface.generate(params);
        let results: Vec<_> = stream.collect().await;
        assert_eq!(results.len(), 1);
        match results[0].as_ref().unwrap() {
            InferenceEvent::Failed(failed) => {
                assert_eq!(failed.status, "failed");
                assert_eq!(failed.error_message, "Incorrect API key provided");
                assert_eq!(failed.raw_response["error"]["code"], "invalid_api_key");
            }
            other => panic!("expected Failed event, got {other:?}"),
        }
    }
}
