use futures::{Stream, StreamExt};
use reqwest_eventsource::Event;
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;

#[allow(unused_imports)]
use crate::request::{Client as _, Response as _};
use crate::error::AnyResult;
use crate::interface::{
    ChatRole, InferenceEvent, InferenceParams, InterfaceModel, ReasoningEffort,
    ResponseCompleted, ResponseCreated, Usage,
};
use crate::request::DefaultClient;

/// HTTP client wrapping a [`DefaultClient`] with auth and a base URL.
#[derive(Debug)]
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
        let request = self
            .builder
            .get(format!("{}{}", self.base_url, endpoint))
            .bearer_auth(&self.api_key)
            .build()?;
        let response = self.inner.execute(request).await?;
        let response = response.error_for_status()?;
        Ok(response.json().await?)
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
struct InputMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct CreateResponseRequest<'a> {
    model: &'a str,
    input: Vec<InputMessage<'a>>,
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
            .map(|msg| {
                let role = match msg.role {
                    ChatRole::User => "user",
                    ChatRole::Assistant => "assistant",
                };
                InputMessage {
                    role,
                    content: msg.content,
                }
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
    Created {
        response: ResponsePayload,
    },
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        delta: String,
    },
    #[serde(rename = "response.reasoning_text.delta")]
    ReasoningTextDelta {
        delta: String,
    },
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryTextDelta {
        delta: String,
    },
    #[serde(rename = "response.completed")]
    Completed {
        response: ResponsePayload,
    },
    #[serde(rename = "response.failed")]
    Failed {
        response: ResponsePayload,
    },
    #[serde(rename = "error")]
    Error(ResponseError),
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
struct ResponsePayload {
    id: String,
    #[serde(default)]
    usage: Option<ResponseUsage>,
    #[serde(default)]
    error: Option<ResponseError>,
}

#[derive(Debug, Deserialize)]
struct ResponseUsage {
    input_tokens: u64,
    output_tokens: u64,
    output_tokens_details: OutputTokensDetails,
}

#[derive(Debug, Deserialize)]
struct OutputTokensDetails {
    reasoning_tokens: u64,
}

impl From<ResponseUsage> for Usage {
    fn from(value: ResponseUsage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            reasoning_tokens: value.output_tokens_details.reasoning_tokens,
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
        Ok(parsed.data.into_iter().map(|m| InterfaceModel { id: m.id }).collect())
    }

    pub async fn generate<'a>(
        &self,
        params: InferenceParams<'a>,
    ) -> impl Stream<Item = AnyResult<InferenceEvent>> {
        let req_body = CreateResponseRequest::from_params(&params);
        let event_stream_result = self.client.post_sse("/responses", &req_body);

        async_stream::try_stream! {
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
                        match serde_json::from_str::<ResponseStreamEvent>(&msg.data)? {
                            ResponseStreamEvent::Created { response } =>
                                yield InferenceEvent::Created(ResponseCreated { id: response.id }),
                            ResponseStreamEvent::OutputTextDelta { delta } =>
                                yield InferenceEvent::OutputDelta(delta),
                            ResponseStreamEvent::ReasoningTextDelta { delta }
                            | ResponseStreamEvent::ReasoningSummaryTextDelta { delta } =>
                                yield InferenceEvent::ThinkingDelta(delta),
                            ResponseStreamEvent::Completed { response } => {
                                yield InferenceEvent::Completed(ResponseCompleted {
                                    usage: response.usage.map(Usage::from),
                                });
                            }
                            ResponseStreamEvent::Error(error)
                            | ResponseStreamEvent::Failed { response: ResponsePayload { error: Some(error), .. } } => {
                                Err(error)?;
                            }
                            ResponseStreamEvent::Failed { .. } => Err("Response failed".to_owned())?,
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
    use reqwest::header::HeaderMap;
    use reqwest::StatusCode;
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
            Ok(create_message_event(r#"{"type":"response.created","response":{"id":"resp-mock-1"}}"#)),
            Ok(create_message_event(r#"{"type":"response.reasoning_text.delta","delta":"Thinking about the question..."}"#)),
            Ok(create_message_event(r#"{"type":"response.output_text.delta","delta":"Hello! "}"#)),
            Ok(create_message_event(r#"{"type":"response.output_text.delta","delta":"How can I help you today?"}"#)),
            Ok(create_message_event(r#"{"type":"response.completed","response":{"id":"resp-mock-1","usage":{"input_tokens":12,"output_tokens":18,"output_tokens_details":{"reasoning_tokens":7}}}}"#)),
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
        client.add_response(&format!("{BASE_URL}/models"), http_ok(r#"{"data":[{"id":"fake-model"}]}"#));

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
            temperature: 0.7,
            reasoning_effort: Some(ReasoningEffort::Low),
            input: &[
                ChatMessage {
                    role: ChatRole::User,
                    content: "Hello",
                },
                ChatMessage {
                    role: ChatRole::Assistant,
                    content: "Hi there!",
                },
                ChatMessage {
                    role: ChatRole::User,
                    content: "How are you?",
                },
            ],
        };

        let stream = iface.generate(params).await;
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
                    id: "resp-mock-1".into()
                }),
                InferenceEvent::ThinkingDelta("Thinking about the question...".into()),
                InferenceEvent::OutputDelta("Hello! ".into()),
                InferenceEvent::OutputDelta("How can I help you today?".into()),
                InferenceEvent::Completed(ResponseCompleted {
                    usage: Some(Usage {
                        input_tokens: 12,
                        output_tokens: 18,
                        reasoning_tokens: 7,
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
        assert_eq!(body["temperature"], 0.7);
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
            sse(vec![Ok(create_message_event(
                r#"{"type":"response.completed","response":{"id":"resp-custom-1"}}"#,
            ))]),
        );

        let iface = make_iface(client);
        let params = InferenceParams {
            model_id: "test-model",
            system_prompt: "",
            temperature: 0.0,
            reasoning_effort: None,
            input: &[],
        };

        let stream = iface.generate(params).await;
        let events: Vec<InferenceEvent> = stream
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .map(Result::unwrap)
            .collect();

        assert_eq!(events, vec![InferenceEvent::Completed(ResponseCompleted { usage: None })]);
    }

    #[tokio::test]
    async fn test_thinking_fields() {
        let mut client = DefaultClient::default();
        client.add_response(
            &format!("{BASE_URL}/responses"),
            sse(vec![
                Ok(create_message_event(r#"{"type":"response.created","response":{"id":"resp-think-1"}}"#)),
                Ok(create_message_event(r#"{"type":"response.reasoning_text.delta","delta":"step 1"}"#)),
                Ok(create_message_event(r#"{"type":"response.reasoning_summary_text.delta","delta":"step 2"}"#)),
                Ok(create_message_event(r#"{"type":"response.output_text.delta","delta":"done"}"#)),
                Ok(create_message_event(r#"{"type":"response.completed","response":{"id":"resp-think-1"}}"#)),
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

        let stream = iface.generate(params).await;
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
                    id: "resp-think-1".into()
                }),
                InferenceEvent::ThinkingDelta("step 1".into()),
                InferenceEvent::ThinkingDelta("step 2".into()),
                InferenceEvent::OutputDelta("done".into()),
                InferenceEvent::Completed(ResponseCompleted {
                    usage: None,
                }),
            ]
        );
    }

    #[tokio::test]
    async fn test_error() {
        let mut client = DefaultClient::default();
        let iface = make_iface(client.clone());
        let url = format!("{BASE_URL}/responses");

        for v in &[
            json!({
                "type": "error",
                "message": "Incorrect API key provided",
                "code":"invalid_api_key",
            }),
            json!({
                "type": "response.failed",
                "response": {
                    "id": "id1",
                    "error": {
                        "message":"Incorrect API key provided",
                        "code":"invalid_api_key",
                    },
                },
            }),
        ] {
            client.add_response(&url, sse(vec![Ok(create_message_event(&v.to_string()))]));

            let params = InferenceParams {
                model_id: "test-model",
                system_prompt: "",
                temperature: 0.0,
                reasoning_effort: None,
                input: &[],
            };

            let stream = iface.generate(params).await;
            let results: Vec<_> = stream.collect().await;
            assert_eq!(results.len(), 1);
            assert!(results[0].is_err());
            assert_eq!(results[0].as_ref().unwrap_err().to_string(), "Incorrect API key provided");
        }
    }
}
