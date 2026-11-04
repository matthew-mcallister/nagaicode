use std::borrow::Cow;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use futures::future::BoxFuture;
use reqwest::header::{ACCEPT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, USER_AGENT, HeaderMap, HeaderValue};
use reqwest::{Method, Request, StatusCode, Url};
use serde_json::{Value, json};

use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::request::{Client, DefaultClient, Response as ResponseTrait};
use crate::tools::{InterfaceToolOutput, Tool};
use crate::ui::render_item::{RenderItem, SystemRenderItem};

const MAX_CONTENT_LENGTH: u64 = 1024 * 1024;
const USER_AGENT_STRING: &str = concat!("nagaicode/", crate::version!());

/// Fetches documents from the web.
#[derive(Debug)]
pub struct WebFetchTool {
    client: DefaultClient,
    input_schema: Value,
}

impl WebFetchTool {
    /// Creates a tool that fetches documents using `client`.
    pub fn new(client: DefaultClient) -> Self {
        Self {
            client,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string" },
                    "format": { "type": "string", "enum": ["raw"] },
                    "timeout": { "type": "number", "minimum": 1, "maximum": 120 },
                },
                "required": ["url", "format", "timeout"],
                "additionalProperties": false,
            }),
        }
    }
}

impl DataQuery for WebFetchTool {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "name": self.name(),
                "is_visible": self.is_visible(),
            }))),
            "name" => Ok(QueryField::Value(json!(self.name()))),
            "is_visible" => Ok(QueryField::Value(json!(self.is_visible()))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetches a document from the web. Only text-based documents may be read."
    }

    fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    fn call<'a>(&'a self, input: &'a Value) -> BoxFuture<'a, AnyResult<Value>> {
        Box::pin(async move {
            let invalid = || anyhow!("arguments don't match schema");
            let url = input
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let format = input
                .get("format")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            if format != "raw" {
                return Err(invalid());
            }
            let timeout = input
                .get("timeout")
                .and_then(Value::as_f64)
                .ok_or_else(invalid)?;
            if !(1.0..=120.0).contains(&timeout) {
                return Err(invalid());
            }

            let url = Url::parse(url).map_err(|e| anyhow!("invalid url: {e}"))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(anyhow!("{url}: unsupported scheme '{}'", url.scheme()));
            }

            let mut request = Request::new(Method::GET, url.clone());
            request.headers_mut().insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
            request.headers_mut().insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STRING));
            *request.timeout_mut() = Some(Duration::from_secs_f64(timeout));

            let start = Instant::now();
            let response = Client::execute(&self.client, request).await?;
            let status = ResponseTrait::status(&response);
            let version = ResponseTrait::version(&response);
            let headers = header_map_to_json(ResponseTrait::headers(&response));

            validate_content_type(ResponseTrait::headers(&response))
                .map_err(|e| status_error(&url, status, &e))?;
            validate_content_length(ResponseTrait::headers(&response))
                .map_err(|e| status_error(&url, status, &e))?;

            let output = ResponseTrait::text(response)
                .await
                .map_err(|e| status_error(&url, status, &e))?;
            let response_time = start.elapsed().as_secs_f64();

            if status.is_client_error() || status.is_server_error() {
                return Err(status_error(&url, status, &output));
            }

            Ok(json!({
                "url": url.as_str(),
                "version": version,
                "status": status.as_u16(),
                "headers": headers,
                "output": output,
                "response_time": response_time,
            }))
        })
    }

    fn render_to_ui(&self, input: &Value, output: &Value) -> AnyResult<Box<dyn RenderItem>> {
        let url = input
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool input"))?;
        let message = match content_length(output) {
            Some(length) => format!("Fetched {url} ({})", format_content_length(length)),
            None => format!("Fetched {url}"),
        };
        Ok(Box::new(SystemRenderItem::new(message)))
    }

    fn render_to_interface(
        &self,
        _input: &Value,
        output: &Value,
    ) -> AnyResult<InterfaceToolOutput> {
        let text = output
            .get("output")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let status = output
            .get("status")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let mut meta = format!("status {status}");
        if let Some(length) = content_length(output) {
            meta.push_str(&format!("\nfetched {}", format_content_length(length)));
        }
        Ok(InterfaceToolOutput {
            content: vec![
                ToolOutputContent::Text {
                    text: Cow::Owned(meta),
                },
                ToolOutputContent::Text {
                    text: Cow::Owned(text.to_owned()),
                },
            ],
        })
    }
}

fn content_length(output: &Value) -> Option<u64> {
    let value = output.get("headers")?.get("content-length")?;
    let value = match value {
        Value::String(value) => value.as_str(),
        Value::Array(values) => values.first()?.as_str()?,
        _ => return None,
    };
    value.trim().parse().ok()
}

fn format_content_length(length: u64) -> String {
    if length <= 9_999 {
        format!("{length} bytes")
    } else {
        format!("{:.1} kb", length as f64 / 1000.0)
    }
}

fn status_error(url: &Url, status: StatusCode, error: &impl std::fmt::Display) -> anyhow::Error {
    anyhow!("{url}: HTTP {}: {error}", status.as_u16())
}

fn validate_content_type(headers: &HeaderMap) -> AnyResult<()> {
    let value = headers
        .get(CONTENT_TYPE)
        .ok_or_else(|| anyhow!("missing content-type"))?;
    let value = value
        .to_str()
        .map_err(|_| anyhow!("invalid content-type"))?;
    let mime: mime::Mime = value
        .parse()
        .map_err(|_| anyhow!("invalid content-type '{value}'"))?;
    let is_text = mime.type_() == mime::TEXT
        || (mime.type_() == mime::APPLICATION && is_text_application(mime.subtype().as_str()));
    if is_text {
        Ok(())
    } else {
        Err(anyhow!("unsupported content-type '{mime}'"))
    }
}

fn is_text_application(subtype: &str) -> bool {
    matches!(subtype, "json" | "xml" | "xhtml+xml" | "javascript")
        || subtype.ends_with("+json")
        || subtype.ends_with("+xml")
}

fn validate_content_length(headers: &HeaderMap) -> AnyResult<()> {
    let value = headers
        .get(CONTENT_LENGTH)
        .ok_or_else(|| anyhow!("missing content-length"))?;
    let value = value
        .to_str()
        .map_err(|_| anyhow!("invalid content-length"))?;
    let length: u64 = value
        .parse()
        .map_err(|_| anyhow!("invalid content-length '{value}'"))?;
    if length > MAX_CONTENT_LENGTH {
        return Err(anyhow!(
            "content-length {length} exceeds {MAX_CONTENT_LENGTH} bytes"
        ));
    }
    Ok(())
}

fn header_map_to_json(headers: &HeaderMap) -> Value {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, value) in headers {
        map.entry(name.as_str().to_owned())
            .or_default()
            .push(String::from_utf8_lossy(value.as_bytes()).into_owned());
    }
    let map: serde_json::Map<String, Value> = map
        .into_iter()
        .map(|(name, mut values)| {
            let value = if values.len() == 1 {
                Value::String(values.pop().expect("one value"))
            } else {
                Value::Array(values.into_iter().map(Value::String).collect())
            };
            (name, value)
        })
        .collect();
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

    use crate::request::DefaultClient;
    use crate::request::test_client::{Response, ResponseData};

    fn input(url: &str) -> Value {
        json!({ "url": url, "format": "raw", "timeout": 30 })
    }

    async fn fetch(
        client: &mut DefaultClient,
        input: &Value,
        status: StatusCode,
        body: &str,
        headers: &[(&str, &str)],
    ) -> AnyResult<Value> {
        let mut header_map = HeaderMap::new();
        for (name, value) in headers {
            header_map.insert(
                HeaderName::from_bytes(name.as_bytes()).expect("header name"),
                HeaderValue::from_str(value).expect("header value"),
            );
        }
        let url = input["url"].as_str().unwrap();
        client.add_response(url, ResponseData::Http(Ok(Response {
            body: body.to_owned(),
            status,
            headers: header_map,
        })));
        let tool = WebFetchTool::new((*client).clone());
        tool.call(input).await
    }

    #[tokio::test]
    async fn test_success() {
        let input = input("https://example.com/");
        let mut client = DefaultClient::default();
        let output = fetch(
            &mut client,
            &input,
            StatusCode::OK,
            "hello",
            &[("content-type", "text/plain"), ("content-length", "5")],
        ).await.unwrap();
        assert_eq!(output["output"], json!("hello"));
    }

    #[tokio::test]
    async fn test_failed_status() {
        let input = input("https://example.com/");
        let mut client = DefaultClient::default();
        let error = fetch(
            &mut client,
            &input,
            StatusCode::BAD_REQUEST,
            "bad request",
            &[("content-type", "text/plain"), ("content-length", "11")],
        ).await.unwrap_err();
        assert_eq!(error.to_string(), "https://example.com/: HTTP 400: bad request");
    }

    #[tokio::test]
    async fn test_non_text_content_type() {
        let input = input("https://example.com/");
        let mut client = DefaultClient::default();
        let error = fetch(
            &mut client,
            &input,
            StatusCode::OK,
            "<svg/>",
            &[("content-type", "image/svg+xml"), ("content-length", "6")],
        ).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            "https://example.com/: HTTP 200: unsupported content-type 'image/svg+xml'"
        );
    }

    #[tokio::test]
    async fn test_oversized_content_length() {
        let input = input("https://example.com/");
        let mut client = DefaultClient::default();
        let content_length = (MAX_CONTENT_LENGTH + 1).to_string();
        let error = fetch(
            &mut client,
            &input,
            StatusCode::OK,
            "",
            &[
                ("content-type", "text/plain"),
                ("content-length", content_length.as_str()),
            ],
        ).await.unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("https://example.com/: HTTP 200: content-length {content_length} exceeds {MAX_CONTENT_LENGTH} bytes")
        );
    }

    #[tokio::test]
    async fn test_missing_content_length() {
        let input = input("https://example.com/");
        let mut client = DefaultClient::default();
        let error = fetch(
            &mut client,
            &input,
            StatusCode::OK,
            "hello",
            &[("content-type", "text/plain")],
        ).await.unwrap_err();
        assert_eq!(error.to_string(), "https://example.com/: HTTP 200: missing content-length");
    }

    #[tokio::test]
    async fn test_render_ui() {
        let tool = WebFetchTool::new(DefaultClient::default());
        let input = input("https://example.com/");
        let mut client = DefaultClient::default();

        let output = fetch(
            &mut client,
            &input,
            StatusCode::OK,
            "hello",
            &[("content-type", "text/plain"), ("content-length", "1234")],
        ).await.unwrap();
        let item = tool.render_to_ui(&input, &output).unwrap();
        assert_eq!(item.query("/type").unwrap(), json!("help"));
        assert_eq!(
            item.query("/value").unwrap(),
            json!("Fetched https://example.com/ (1234 bytes)")
        );

        let output = fetch(
            &mut client,
            &input,
            StatusCode::OK,
            "hello",
            &[("content-type", "text/plain"), ("content-length", "10000")],
        ).await.unwrap();
        let item = tool.render_to_ui(&input, &output).unwrap();
        assert_eq!(item.query("/type").unwrap(), json!("help"));
        assert_eq!(
            item.query("/value").unwrap(),
            json!("Fetched https://example.com/ (10.0 kb)")
        );
    }

    #[tokio::test]
    async fn test_render_interface() {
        let tool = WebFetchTool::new(DefaultClient::default());
        let input = input("https://example.com/");
        let mut client = DefaultClient::default();

        let output = fetch(
            &mut client,
            &input,
            StatusCode::OK,
            "hello",
            &[("content-type", "text/plain"), ("content-length", "1234")],
        ).await.unwrap();
        let result = tool.render_to_interface(&input, &output).unwrap();
        assert_eq!(
            result.content,
            vec![
                ToolOutputContent::Text { text: Cow::Owned("status 200\nfetched 1234 bytes".into()) },
                ToolOutputContent::Text { text: Cow::Owned("hello".into()) },
            ]
        );

        let output = fetch(
            &mut client,
            &input,
            StatusCode::OK,
            "hello",
            &[("content-type", "text/plain"), ("content-length", "10000")],
        ).await.unwrap();
        let result = tool.render_to_interface(&input, &output).unwrap();
        assert_eq!(
            result.content,
            vec![
                ToolOutputContent::Text { text: Cow::Owned("status 200\nfetched 10.0 kb".into()) },
                ToolOutputContent::Text { text: Cow::Owned("hello".into()) },
            ]
        );
    }
}
