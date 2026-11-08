use std::borrow::Cow;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
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
use crate::tools::{InterfaceToolOutput, Tool, ToolCreateArgs};
use crate::ui::render_item::{RenderItem, SystemRenderItem};

const USER_AGENT_STRING: &str = concat!("nagaicode/", crate::version!());

/// Fetches documents from the web.
#[derive(Debug)]
pub struct WebFetchTool {
    client: DefaultClient,
    dir: PathBuf,
    input_schema: Value,
}

impl WebFetchTool {
    /// Creates a tool that fetches documents using `args.client`.
    pub fn new(args: &ToolCreateArgs<'_>) -> Self {
        Self {
            client: args.client.clone(),
            dir: args.dirs.temp().join("web_fetch"),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string" },
                    "timeout": { "type": "number", "minimum": 1, "maximum": 120 },
                },
                "required": ["url", "timeout"],
                "additionalProperties": false,
            }),
        }
    }

    fn dir(&self) -> AnyResult<&Path> {
        std::fs::create_dir_all(&self.dir)?;
        Ok(&self.dir)
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
        "Fetches and saves a document from the web, returning a path to the
        saved file."
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

            let body = ResponseTrait::text(response)
                .await
                .map_err(|e| anyhow!("{url}: (status {}) {e}", status.as_u16()))?;
            let response_time = start.elapsed().as_secs_f64();

            let filepath = write_no_clobber(self.dir()?, &file_name(&url), &body)?;
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

fn is_replace_char(c: char) -> bool {
    matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()
}

/// Compute a save file name for the given URL
fn file_name(url: &Url) -> String {
    let segment = url
        .path_segments()
        .and_then(|segments| segments.filter(|s| !s.is_empty()).last());
    let Some(segment) = segment else {
        return "index".to_owned();
    };
    let decoded = percent_encoding::percent_decode_str(segment).decode_utf8_lossy();
    let name: String = decoded.chars()
        .map(|c| if is_replace_char(c) { '_' } else { c })
        .collect();
    if name.is_empty() || name == "." || name == ".." {
        "index".to_owned()
    } else {
        name
    }
}

/// Write a file to disk. If a file with that name already exists, appends
/// (1), (2), ... to the filename until it succeeds.
fn write_no_clobber(dir: &Path, name: &str, body: &str) -> AnyResult<PathBuf> {
    let mut attempt = 0;
    loop {
        let path = dir.join(add_attempt_number(name, attempt));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(body.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => attempt += 1,
            Err(e) => return Err(e.into()),
        }
    }
}

/// Add a suffix "(n)" to a filename.
fn add_attempt_number(name: &str, n: u32) -> String {
    if n == 0 {
        return name.to_owned();
    }
    let path = Path::new(name);
    match (path.file_stem(), path.extension()) {
        (Some(stem), Some(extension)) => {
            format!(
                "{}({n}).{}",
                stem.to_string_lossy(),
                extension.to_string_lossy()
            )
        }
        _ => format!("{name}({n})"),
    }
}

fn get_header<'a>(output: &'a Value, name: &str) -> Option<&'a str> {
    let value = output.get("headers")?.get(name)?;
    match value {
        Value::String(value) => Some(value),
        Value::Array(values) => values.first()?.as_str(),
        _ => None,
    }
}

fn get_content_length(output: &Value) -> Option<u64> {
    get_header(output, "content-length")?.trim().parse().ok()
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
        json!({ "url": url, "timeout": 30 })
    }

    fn get_filepath(output: &Value) -> PathBuf {
        PathBuf::from(output["filepath"].as_str().unwrap())
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
            self.tool.call(&input(url)).await
        }
    }

    #[tokio::test]
    async fn test_success() {
        let mut f = Fixture::new();
        let headers = [("content-type", "text/plain"), ("content-length", "5")];
        let output = f.fetch("https://example.com/", StatusCode::OK, "hello", &headers).await.unwrap();
        assert_eq!(output.get("status"), Some(&Value::Number(200.into())));
        assert_eq!(std::fs::read_to_string(&get_filepath(&output)).unwrap(), "hello");
    }

    #[tokio::test]
    async fn test_file_name() {
        let mut f = Fixture::new();
        let url = "https://example.com/a.html";
        let headers = [("content-type", "text/plain"), ("content-length", "4")];
        for expected in ["a.html", "a(1).html", "a(2).html"] {
            let output = f.fetch(url, StatusCode::OK, "blah", &headers).await.unwrap();
            let path = get_filepath(&output);
            assert_eq!(path.file_name().unwrap().to_str().unwrap(), expected);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "blah");
        }
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
        let output = f.fetch(url, StatusCode::INTERNAL_SERVER_ERROR, "hello", &headers).await.unwrap();
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
