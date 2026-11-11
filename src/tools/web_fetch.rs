use std::borrow::Cow;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use fnv::FnvHashMap;
use futures::future::BoxFuture;
use reqwest::header::{ACCEPT_ENCODING, USER_AGENT, HeaderMap, HeaderValue};
use reqwest::{Method, Request, Url};
use serde_json::{Value, json};

use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::request::{Client, DefaultClient, Response as ResponseTrait};
use crate::tools::{InterfaceToolOutput, Tool, ToolCallContext, ToolCreateArgs};
use crate::tools::read::paginate;
use crate::ui::render_item::{RenderItem, SystemRenderItem};

const USER_AGENT_STRING: &str = concat!("nagaicode/", crate::version!());
const TIMEOUT_SECS: u64 = 120;
const MAX_CONTENT_LENGTH: u64 = 1_000_000;

/// Fetches text documents from the web, including HTML. HTML documents are
/// automatically converted to Markdown. Documents are paginated just as text
/// files are.
/// 
/// Limits:
/// - Content-Length must be present and <= 1e6 bytes
/// - Content-Type must be present and specify a text file type
/// - Lines truncated at 2000 bytes
// TODO: Cache downloaded documents.
#[derive(Debug)]
pub struct WebFetchTool {
    client: DefaultClient,
    input_schema: Value,
}

impl WebFetchTool {
    /// Creates a tool that fetches documents using `args.client`.
    pub fn new(args: &ToolCreateArgs<'_>) -> Self {
        Self {
            client: args.client.clone(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string" },
                    "start_line": { "type": "integer", "minimum": 1 },
                    "max_lines": { "type": "integer", "minimum": 1 },
                },
                "required": ["url", "start_line", "max_lines"],
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
        "Fetches a text document from the web. Start line is 1-indexed. \
        Output includes next line offset for pagination. Lines truncated at \
        2000 bytes max. HTML will be converted to markdown."
    }

    fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    fn call<'a>(
        &'a self,
        _context: &'a ToolCallContext,
        input: &'a Value,
    ) -> BoxFuture<'a, AnyResult<Value>> {
        Box::pin(async move {
            let invalid = || anyhow!("arguments don't match schema");
            let url = input
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let start_line = input
                .get("start_line")
                .and_then(Value::as_i64)
                .ok_or_else(invalid)?;
            let max_lines = input
                .get("max_lines")
                .and_then(Value::as_i64)
                .ok_or_else(invalid)?;
            if max_lines < 1 {
                return Err(invalid());
            }

            let url = Url::parse(url).map_err(|e| anyhow!("invalid url: {e}"))?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(anyhow!("{url}: unsupported scheme '{}'", url.scheme()));
            }

            let mut request = Request::new(Method::GET, url.clone());
            request.headers_mut().insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
            request.headers_mut().insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_STRING));
            *request.timeout_mut() = Some(Duration::from_secs(TIMEOUT_SECS));

            let start = Instant::now();
            let response = Client::execute(&self.client, request).await?;
            let status = ResponseTrait::status(&response);
            let version = ResponseTrait::version(&response);
            let headers = header_map_to_json(ResponseTrait::headers(&response));
            let content_type = header_value(&headers, "content-type")
                .ok_or_else(|| anyhow!("{url}: missing content-type"))?;
            if !is_text_content_type(content_type) {
                return Err(anyhow!("{url}: unsupported content type '{content_type}'"));
            }
            let length = content_length(&headers)
                .ok_or_else(|| anyhow!("{url}: missing content-length"))?;
            if length > MAX_CONTENT_LENGTH {
                return Err(anyhow!(
                    "{url}: content too large ({})",
                    format_content_length(length)
                ));
            }

            let body = ResponseTrait::text(response)
                .await
                .map_err(|e| anyhow!("{url}: (status {}) {e}", status.as_u16()))?;
            let response_time = start.elapsed().as_secs_f64();

            let body = if is_html_content_type(content_type) {
                html_to_markdown_rs::convert(&body, None)
                    .map_err(|e| anyhow!("{url}: {e}"))?
                    .content
                    .ok_or_else(|| anyhow!("{url}: conversion produced no output"))?
            } else {
                body
            };

            let (content, num_lines, next_line) =
                paginate(body.split_inclusive('\n').map(Ok), start_line, max_lines)?;
            let mut output = json!({
                "url": url.as_str(),
                "version": version,
                "status": status.as_u16(),
                "headers": headers,
                "response_time": response_time,
                "content": content,
                "num_lines": num_lines,
            });
            if let Some(next_line) = next_line {
                output["next_line"] = json!(next_line);
            }
            Ok(output)
        })
    }

    fn render_to_ui(&self, input: &Value, output: &Value) -> AnyResult<Box<dyn RenderItem>> {
        let url = input
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool input"))?;
        let message = match get_content_length(output) {
            Some(length) => format!("Fetched {url} ({})", format_content_length(length)),
            None => format!("Fetched {url}"),
        };
        // FIXME: Render as error message if status is >= 400
        Ok(Box::new(SystemRenderItem::new(message)))
    }

    fn render_to_interface(
        &self,
        _input: &Value,
        output: &Value,
    ) -> AnyResult<InterfaceToolOutput> {
        let status = output
            .get("status")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let content = output
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let num_lines = output
            .get("num_lines")
            .and_then(Value::as_u64)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let mut meta = format!("status {status}");
        if let Some(length) = get_content_length(output) {
            meta.push_str(&format!("\nfetched {}", format_content_length(length)));
        }
        if let Some(content_type) = get_header(output, "content-type") {
            meta.push_str(&format!("\nfile type: {content_type}"));
        }
        meta.push_str(&format!("\nread {num_lines} lines"));
        match output.get("next_line").and_then(Value::as_i64) {
            Some(next_line) => meta.push_str(&format!("\nnext line: {next_line}")),
            None => meta.push_str("\nreached end of document"),
        }
        // TODO maybe: Infer filetype if MIME missing
        Ok(InterfaceToolOutput {
            content: vec![
                ToolOutputContent::Text {
                    text: Cow::Owned(meta),
                },
                ToolOutputContent::Text {
                    text: Cow::Owned(content.to_owned()),
                },
            ],
        })
    }
}

fn get_header<'a>(output: &'a Value, name: &str) -> Option<&'a str> {
    header_value(output.get("headers")?, name)
}

fn header_value<'a>(headers: &'a Value, name: &str) -> Option<&'a str> {
    match headers.get(name)? {
        Value::String(value) => Some(value),
        Value::Array(values) => values.first()?.as_str(),
        _ => None,
    }
}

fn get_content_length(output: &Value) -> Option<u64> {
    content_length(output.get("headers")?)
}

fn content_length(headers: &Value) -> Option<u64> {
    header_value(headers, "content-length")?.trim().parse().ok()
}

fn is_text_content_type(content_type: &str) -> bool {
    let mime = mime_type(content_type);
    if mime.starts_with("text/") {
        return true;
    }
    let Some((ty, subtype)) = mime.split_once('/') else {
        return false;
    };
    subtype.ends_with("+json")
        || subtype.ends_with("+xml")
        || matches!(
            (ty, subtype),
            ("application", "json")
                | ("application", "xml")
                | ("application", "javascript")
                | ("application", "x-javascript")
                | ("application", "toml")
                | ("application", "yaml")
                | ("application", "x-yaml")
        )
}

fn is_html_content_type(content_type: &str) -> bool {
    matches!(mime_type(content_type), "text/html" | "application/xhtml+xml")
}

fn mime_type(content_type: &str) -> &str {
    content_type.split(';').next().unwrap_or("").trim()
}

fn format_content_length(length: u64) -> String {
    if length <= 9_999 {
        format!("{length} bytes")
    } else {
        format!("{:.1} kb", length as f64 / 1000.0)
    }
}

fn header_map_to_json(headers: &HeaderMap) -> Value {
    let mut map: FnvHashMap<String, Vec<String>> = Default::default();
    for (name, value) in headers {
        let key = name.as_str().to_owned();
        // XXX: Could stand to handle non-utf8 headers better
        let value = String::from_utf8_lossy(value.as_bytes()).to_lowercase();
        map.entry(key).or_default().push(value);
    }

    fn map_values(mut values: Vec<String>) -> Value {
        if values.len() == 1 {
            Value::String(values.pop().unwrap())
        } else {
            Value::Array(values.into_iter().map(Value::String).collect())
        }
    }

    let map: serde_json::Map<String, Value> = map
        .into_iter()
        .map(|(name, values)| (name, map_values(values)))
        .collect();

    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};

    use crate::app::App;
    use crate::request::DefaultClient;
    use crate::request::test_client::{Response, ResponseData};

    fn input(url: &str) -> Value {
        json!({ "url": url, "start_line": 1, "max_lines": 10 })
    }

    #[derive(Debug)]
    struct Fixture {
        client: DefaultClient,
        tool: WebFetchTool,
    }

    impl Fixture {
        fn new() -> Self {
            let mut app = App::new().unwrap();
            let tool = WebFetchTool::new(&app.tool_create_args());
            Self {
                tool,
                client: app.client_mut().clone(),
            }
        }
        
        async fn fetch(
            &mut self,
            url: &str,
            status: StatusCode,
            body: &str,
            headers: &[(&str, &str)],
        ) -> AnyResult<Value> {
            let input = input(url);
            self.fetch_with(url, status, body, headers, &input).await
        }

        async fn fetch_with(
            &mut self,
            url: &str,
            status: StatusCode,
            body: &str,
            headers: &[(&str, &str)],
            input: &Value,
        ) -> AnyResult<Value> {
            let mut header_map = HeaderMap::new();
            for (name, value) in headers {
                header_map.insert(
                    HeaderName::from_bytes(name.as_bytes()).unwrap(),
                    HeaderValue::from_str(value).unwrap(),
                );
            }
            self.client.add_response(url, ResponseData::Http(Ok(Response {
                body: body.to_owned(),
                status,
                headers: header_map,
            })));
            let context = ToolCallContext { session_id: 1 };
            self.tool.call(&context, input).await
        }
    }

    #[tokio::test]
    async fn test_success() {
        let mut f = Fixture::new();
        let url = "https://example.com/";
        let headers = [("content-type", "text/plain"), ("content-length", "5")];
        let output = f.fetch(url, StatusCode::OK, "hello", &headers).await.unwrap();
        assert_eq!(output.get("status"), Some(&Value::Number(200.into())));
        assert_eq!(output["content"], json!("hello"));
        assert_eq!(output["num_lines"], json!(1));
        assert_eq!(output.get("next_line"), None);
    }

    #[tokio::test]
    async fn test_html_conversion() {
        let mut f = Fixture::new();
        let url = "https://example.com/";
        let html = "<h1>Title</h1><p>Body</p>";
        let headers = [("content-type", "text/html; charset=utf-8"), ("content-length", "25")];

        let input = json!({ "url": url, "start_line": 1, "max_lines": 10 });
        let output = f
            .fetch_with(url, StatusCode::OK, html, &headers, &input)
            .await
            .unwrap();
        assert_eq!(output["content"], json!("# Title\n\nBody\n"));
        assert_eq!(output["num_lines"], json!(3));
        assert_eq!(output.get("next_line"), None);

        // Pagination applies to the converted markdown, not the raw HTML.
        let input = json!({ "url": url, "start_line": 1, "max_lines": 2 });
        let output = f
            .fetch_with(url, StatusCode::OK, html, &headers, &input)
            .await
            .unwrap();
        assert_eq!(output["content"], json!("# Title\n\n"));
        assert_eq!(output["num_lines"], json!(2));
        assert_eq!(output["next_line"], json!(3));
    }

    #[tokio::test]
    async fn test_content_size() {
        let mut f = Fixture::new();
        let url = "https://example.com/";
        let input = input(url);
        let headers = [("content-type", "text/plain"), ("content-length", "1234")];
        let output = f.fetch(url, StatusCode::OK, "hello", &headers).await.unwrap();
        let item = f.tool.render_to_ui(&input, &output).unwrap();
        assert_eq!(item.query("/type").unwrap(), json!("help"));
        assert_eq!(
            item.query("/value").unwrap(),
            json!("Fetched https://example.com/ (1234 bytes)")
        );

        let headers = [("content-type", "text/plain"), ("content-length", "10000")];
        let output = f.fetch(url, StatusCode::OK, "hello", &headers).await.unwrap();
        let item = f.tool.render_to_ui(&input, &output).unwrap();
        assert_eq!(item.query("/type").unwrap(), json!("help"));
        assert_eq!(
            item.query("/value").unwrap(),
            json!("Fetched https://example.com/ (10.0 kb)")
        );
    }

    #[tokio::test]
    async fn test_render_interface() {
        let mut f = Fixture::new();
        let url = "https://example.com/";
        let input = input(url);
        let headers = [("content-type", "text/plain"), ("content-length", "1234")];
        let output = f.fetch(url, StatusCode::OK, "hello", &headers).await.unwrap();
        let result = f.tool.render_to_interface(&input, &output).unwrap();
        assert_eq!(
            result.content,
            vec![
                ToolOutputContent::Text {
                    text: Cow::Owned("\
                        status 200\n\
                        fetched 1234 bytes\n\
                        file type: text/plain\n\
                        read 1 lines\n\
                        reached end of document\
                    ".to_owned()),
                },
                ToolOutputContent::Text {
                    text: Cow::Owned("hello".to_owned()),
                },
            ]
        );

        let headers = [("content-type", "text/plain"), ("content-length", "10000")];
        let output = f.fetch(url, StatusCode::INTERNAL_SERVER_ERROR, "hello", &headers).await.unwrap();
        let result = f.tool.render_to_interface(&input, &output).unwrap();
        assert_eq!(
            result.content,
            vec![
                ToolOutputContent::Text {
                    text: Cow::Owned("\
                        status 500\n\
                        fetched 10.0 kb\n\
                        file type: text/plain\n\
                        read 1 lines\n\
                        reached end of document\
                    ".to_owned()),
                },
                ToolOutputContent::Text {
                    text: Cow::Owned("hello".to_owned()),
                },
            ]
        );
    }
}
