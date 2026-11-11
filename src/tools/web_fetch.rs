use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use fnv::FnvHashMap;
use futures::future::BoxFuture;
use reqwest::header::{ACCEPT_ENCODING, USER_AGENT, HeaderMap, HeaderValue};
use reqwest::{Method, Request, Url};
use serde_json::{Value, json};

use crate::config::Dirs;
use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::request::{Client, DefaultClient, Response as ResponseTrait};
use crate::tools::{InterfaceToolOutput, Tool, ToolCallContext, ToolCreateArgs};
use crate::ui::render_item::{RenderItem, SystemRenderItem};

const USER_AGENT_STRING: &str = concat!("nagaicode/", crate::version!());
const TIMEOUT_SECS: u64 = 120;
const MAX_CONTENT_LENGTH: u64 = 1_000_000;

/// Fetches documents from the web.
#[derive(Debug)]
pub struct WebFetchTool {
    client: DefaultClient,
    dirs: Arc<Dirs>,
    input_schema: Value,
}

impl WebFetchTool {
    /// Creates a tool that fetches documents using `args.client`.
    pub fn new(args: &ToolCreateArgs<'_>) -> Self {
        Self {
            client: args.client.clone(),
            dirs: Arc::clone(args.dirs),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string" },
                    "filename": { "type": "string" },
                },
                "required": ["url", "filename"],
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
        "Fetches and saves a document from the web into session storage,
        returning a path to the saved file. Any existing file with the same
        filename is overwritten."
    }

    fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    fn call<'a>(
        &'a self,
        context: &'a ToolCallContext,
        input: &'a Value,
    ) -> BoxFuture<'a, AnyResult<Value>> {
        Box::pin(async move {
            let invalid = || anyhow!("arguments don't match schema");
            let url = input
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let filename = input
                .get("filename")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;

            // TODO: support sandboxed relative filepaths
            if !is_valid_filename(filename) {
                return Err(anyhow!("invalid filename: {filename}"));
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

            let dir = self.dirs.session_storage(context.session_id);
            std::fs::create_dir_all(&dir)?;
            let filepath = dir.join(filename);
            std::fs::write(&filepath, &body)?;
            Ok(json!({
                "url": url.as_str(),
                "version": version,
                "status": status.as_u16(),
                "headers": headers,
                "filepath": filepath.display().to_string(),
                "response_time": response_time,
            }))
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
        let filepath = output
            .get("filepath")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let mut meta = format!("status {status}");
        if let Some(length) = get_content_length(output) {
            meta.push_str(&format!("\nfetched {}", format_content_length(length)));
        }
        meta.push_str(&format!("\nsaved to {filepath}"));
        if let Some(content_type) = get_header(output, "content-type") {
            meta.push_str(&format!("\nfile type: {content_type}"));
        }
        // TODO maybe: Infer filetype if MIME missing
        Ok(InterfaceToolOutput {
            content: vec![ToolOutputContent::Text {
                text: Cow::Owned(meta),
            }],
        })
    }
}

/// Whether `name` is a single, normal path component.
fn is_valid_filename(name: &str) -> bool {
    let mut components = Path::new(name).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
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
    let mime = content_type.split(';').next().unwrap_or("").trim();
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

    fn input(url: &str, filename: &str) -> Value {
        json!({ "url": url, "filename": filename })
    }

    fn get_filepath(output: &Value) -> &Path {
        Path::new(output["filepath"].as_str().unwrap())
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
            filename: &str,
            status: StatusCode,
            body: &str,
            headers: &[(&str, &str)],
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
            self.tool.call(&context, &input(url, filename)).await
        }
    }

    #[tokio::test]
    async fn test_success() {
        let mut f = Fixture::new();
        let url = "https://example.com/";
        let headers = [("content-type", "text/plain"), ("content-length", "5")];
        let output = f.fetch(url, "index.html", StatusCode::OK, "hello", &headers).await.unwrap();
        assert_eq!(output.get("status"), Some(&Value::Number(200.into())));
        assert_eq!(get_filepath(&output).file_name().unwrap().to_str().unwrap(), "index.html");
        assert_eq!(std::fs::read_to_string(get_filepath(&output)).unwrap(), "hello");
    }

    #[tokio::test]
    async fn test_overwrite() {
        let mut f = Fixture::new();
        let url = "https://example.com/a.html";
        let headers = [("content-type", "text/plain"), ("content-length", "4")];
        let first = f.fetch(url, "a.html", StatusCode::OK, "blah", &headers).await.unwrap();
        let second = f.fetch(url, "a.html", StatusCode::OK, "blah", &headers).await.unwrap();
        assert_eq!(get_filepath(&first), get_filepath(&second));
        assert_eq!(std::fs::read_to_string(get_filepath(&second)).unwrap(), "blah");
    }

    #[tokio::test]
    async fn test_invalid_filename() {
        let mut f = Fixture::new();
        let url = "https://example.com/";
        for filename in ["", ".", "..", "a/b", "/a"] {
            let result = f.fetch(url, filename, StatusCode::OK, "blah", &[]).await;
            assert!(result.is_err(), "{filename:?} should be rejected");
        }
    }

    #[tokio::test]
    async fn test_content_size() {
        let mut f = Fixture::new();
        let url = "https://example.com/";
        let input = input(url, "index.html");
        let headers = [("content-type", "text/plain"), ("content-length", "1234")];
        let output = f.fetch(url, "index.html", StatusCode::OK, "hello", &headers).await.unwrap();
        let item = f.tool.render_to_ui(&input, &output).unwrap();
        assert_eq!(item.query("/type").unwrap(), json!("help"));
        assert_eq!(
            item.query("/value").unwrap(),
            json!("Fetched https://example.com/ (1234 bytes)")
        );

        let headers = [("content-type", "text/plain"), ("content-length", "10000")];
        let output = f.fetch(url, "index.html", StatusCode::OK, "hello", &headers).await.unwrap();
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
        let input = input(url, "index.html");
        let headers = [("content-type", "text/plain"), ("content-length", "1234")];
        let output = f.fetch(url, "index.html", StatusCode::OK, "hello", &headers).await.unwrap();
        let filepath = output["filepath"].as_str().unwrap();
        let result = f.tool.render_to_interface(&input, &output).unwrap();
        assert_eq!(
            result.content,
            vec![ToolOutputContent::Text {
                text: Cow::Owned(format!("\
                    status 200\n\
                    fetched 1234 bytes\n\
                    saved to {filepath}\n\
                    file type: text/plain\
                ")),
            }]
        );

        let headers = [("content-type", "text/plain"), ("content-length", "10000")];
        let output = f.fetch(url, "index.html", StatusCode::INTERNAL_SERVER_ERROR, "hello", &headers).await.unwrap();
        let filepath = output["filepath"].as_str().unwrap();
        let result = f.tool.render_to_interface(&input, &output).unwrap();
        assert_eq!(
            result.content,
            vec![ToolOutputContent::Text {
                text: Cow::Owned(format!("\
                    status 500\n\
                    fetched 10.0 kb\n\
                    saved to {filepath}\n\
                    file type: text/plain\
                ")),
            }]
        );
    }
}
