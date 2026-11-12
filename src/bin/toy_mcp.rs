//! A toy MCP server for use by unit tests. It exposes a handful of tool calls
//! available over stdio or HTTP transport. It includes tools for testing
//! handling, timeouts, and crash recovery.

use std::convert::Infallible;
use std::net::Ipv4Addr;
use std::time::Duration;

use bytes::Bytes;
use chrono::{SecondsFormat, Utc};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header;
use hyper::service::service_fn;
use hyper::{Method, Request as HttpRequest, Response as HttpResponse, StatusCode};
use hyper_util::rt::TokioIo;
use nagai::json_rpc::{Error, Request, Response, ResponseBody};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

const PROTOCOL_VERSION: &str = "2026-07-28";
const SERVER_NAME: &str = "toy-mcp";
const SERVER_VERSION: &str = "1.0.0";

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct ToolCall {
    name: String,
    #[serde(default)]
    arguments: Value,
}

impl ToolCall {
    fn parse(params: &Value) -> Result<Self, Error> {
        Self::deserialize(params)
            .map_err(|error| Error::invalid_params(error.to_string()))
    }
}

async fn handle(request: Request) -> Result<Value, Error> {
    match request.method.as_str() {
        "server/discover" => discover(),
        "tools/list" => list_tools(),
        "tools/call" => call_tool(&request.params).await,
        method => Err(Error {
            code: -32601,
            message: format!("method not found: {method}"),
            data: None,
        }),
    }
}

fn discover() -> Result<Value, Error> {
    Ok(json!({
        "resultType": "complete",
        "supportedVersions": [PROTOCOL_VERSION],
        "capabilities": {
            "tools": {},
        },
        "_meta": {
            "io.modelcontextprotocol/serverInfo": {
                "name": SERVER_NAME,
                "version": SERVER_VERSION,
            },
        },
    }))
}

fn list_tools() -> Result<Value, Error> {
    Ok(json!({
        "resultType": "complete",
        "tools": [
            {
                "name": "timestamp",
                "description": "Returns the current date/time in ISO format",
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": false,
                },
            },
            {
                "name": "echo",
                "description": "Returns the input verbatim",
                "inputSchema": {},
            },
            {
                "name": "panic",
                "description": "Causes the server to panic",
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": false,
                },
            },
            {
                "name": "stall",
                "description": "Sleeps forever, never returns",
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": false,
                },
            },
            {
                "name": "abort",
                "description": "Causes the server to exit with code 1",
                "inputSchema": {
                    "type": "object",
                    "additionalProperties": false,
                },
            },
            {
                "name": "error",
                "description": "Returns the given JSON-RPC error as the response",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "integer" },
                        "message": { "type": "string" },
                        "data": {},
                    },
                    "required": ["code", "message"],
                },
            },
        ],
    }))
}

async fn call_tool(params: &Value) -> Result<Value, Error> {
    let call = ToolCall::parse(params)?;
    match call.name.as_str() {
        "timestamp" => Ok(timestamp()),
        "echo" => Ok(echo(call.arguments)),
        "panic" => panic!("panic tool invoked"),
        "stall" => {
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
        }
        "abort" => std::process::exit(1),
        "error" => {
            let error = serde_json::from_value(call.arguments)
                .map_err(|error| Error::invalid_params(error.to_string()))?;
            Err(error)
        }
        name => Err(Error::invalid_params(format!("unknown tool: {name}"))),
    }
}

fn tool_result(text: String, structured_content: Value) -> Value {
    json!({
        "resultType": "complete",
        "content": [
            {
                "type": "text",
                "text": text,
            },
        ],
        "structuredContent": structured_content,
        "isError": false,
    })
}

fn timestamp() -> Value {
    let timestamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    tool_result(timestamp.clone(), json!({ "timestamp": timestamp }))
}

fn echo(arguments: Value) -> Value {
    let text = serde_json::to_string(&arguments).unwrap();
    tool_result(text, arguments)
}

async fn respond(line: &str) -> Option<Response> {
    let request = match serde_json::from_str::<Request>(line) {
        Ok(request) => request,
        Err(error) => {
            return Some(Response {
                body: ResponseBody::Error(Error {
                    code: -32700,
                    message: format!("parse error: {error}"),
                    data: None,
                }),
                id: Value::Null,
            });
        }
    };
    let id = request.id.clone()?;
    let body = match handle(request).await {
        Ok(result) => ResponseBody::Result(result),
        Err(error) => ResponseBody::Error(error),
    };
    Some(Response {
        body,
        id,
    })
}

async fn serve_stdio() -> Result<(), Error> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let Some(response) = respond(&line).await else {
            continue;
        };
        let response = serde_json::to_string(&response)?;
        stdout.write_all(response.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }
    Ok(())
}

const USAGE: &str = "usage: toy_mcp <stdio|http [port]>";
const HTTP_HOST: Ipv4Addr = Ipv4Addr::LOCALHOST;
const HTTP_DEFAULT_PORT: u16 = 8080;
const HTTP_PATH: &str = "/mcp";

async fn serve_http(port: u16) -> Result<(), Error> {
    let listener = TcpListener::bind((HTTP_HOST, port)).await?;
    let addr = listener.local_addr()?;
    eprintln!("listening on http://{addr}{HTTP_PATH}");
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service_fn(handle_http))
                .await;
        });
    }
}

async fn handle_http(
    request: HttpRequest<Incoming>,
) -> Result<HttpResponse<Full<Bytes>>, Infallible> {
    if request.method() != Method::POST || request.uri().path() != HTTP_PATH {
        return Ok(text_response(StatusCode::NOT_FOUND, "not found"));
    }
    let body = match request.into_body().collect().await {
        Ok(body) => body.to_bytes(),
        Err(error) => return Ok(text_response(StatusCode::BAD_REQUEST, &error.to_string())),
    };
    let Ok(body) = std::str::from_utf8(&body) else {
        return Ok(text_response(StatusCode::BAD_REQUEST, "invalid utf-8"));
    };
    let Some(response) = respond(body).await else {
        return Ok(HttpResponse::builder()
            .status(StatusCode::ACCEPTED)
            .body(Full::new(Bytes::new()))
            .unwrap());
    };
    let json = serde_json::to_vec(&response).unwrap();
    Ok(HttpResponse::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(json)))
        .unwrap())
}

fn text_response(status: StatusCode, message: &str) -> HttpResponse<Full<Bytes>> {
    HttpResponse::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Full::new(Bytes::from(message.to_owned())))
        .unwrap()
}

fn parse_port(value: Option<&str>) -> u16 {
    let Some(value) = value else {
        return HTTP_DEFAULT_PORT;
    };
    match value.parse() {
        Ok(port) => port,
        Err(_) => {
            eprintln!("invalid port: {value}");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("stdio") => serve_stdio().await.unwrap(),
        Some("http") => serve_http(parse_port(args.next().as_deref())).await.unwrap(),
        Some(command) => {
            eprintln!("unknown command: {command}");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
        None => println!("{USAGE}"),
    }
}

#[cfg(test)]
mod tests {
    use nagai::json_rpc::{Error, Request};

use super::*;

    fn request(method: &str, params: Value) -> Request {
        Request {
            method: method.to_owned(),
            params,
            id: Some(json!(1)),
        }
    }

    async fn call(name: &str, arguments: Value) -> Result<Value, Error> {
        handle(request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        ))
        .await
    }

    #[tokio::test]
    async fn test_discover() {
        let result = handle(request("server/discover", json!({}))).await.unwrap();
        assert_eq!(result["resultType"], json!("complete"));
        assert_eq!(result["supportedVersions"], json!([PROTOCOL_VERSION]));
        assert_eq!(result["capabilities"], json!({ "tools": {} }));
        assert_eq!(
            result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            json!(SERVER_NAME),
        );
        assert_eq!(
            result["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
            json!(SERVER_VERSION),
        );
        assert!(result.get("instructions").is_none());
    }

    #[tokio::test]
    async fn test_list_tools() {
        let result = handle(request("tools/list", json!({}))).await.unwrap();
        assert_eq!(result["resultType"], json!("complete"));
        let tools = result["tools"].as_array().unwrap();
        let names: Vec<_> = tools.iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["timestamp", "echo", "panic", "stall", "abort", "error"]);
        for tool in tools {
            assert!(tool["description"].is_string());
            assert!(tool["inputSchema"].is_object());
        }
    }

    #[tokio::test]
    async fn test_unknown_method() {
        let error = handle(request("no_such_method", json!({}))).await.unwrap_err();
        assert_eq!(error.code, -32601);
    }

    #[tokio::test]
    async fn test_unknown_tool() {
        let error = call("no_such_tool", json!({})).await.unwrap_err();
        assert_eq!(error.code, -32602);
    }

    #[tokio::test]
    async fn test_missing_tool_name() {
        let error = handle(request("tools/call", json!({}))).await.unwrap_err();
        assert_eq!(error.code, -32602);
    }

    #[tokio::test]
    async fn test_timestamp() {
        let result = call("timestamp", json!({})).await.unwrap();
        assert_eq!(result["resultType"], json!("complete"));
        assert_eq!(result["isError"], json!(false));
        assert_eq!(result["content"][0]["type"], json!("text"));
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(text).unwrap();
        let age = Utc::now().signed_duration_since(parsed);
        assert!(age.num_seconds().abs() < 5);
        assert_eq!(result["structuredContent"]["timestamp"], json!(text));
    }

    #[tokio::test]
    async fn test_echo() {
        let values = [
            json!({ "a": [1, 2, { "b": null }] }),
            json!("hello"),
            json!([1, true, "x"]),
            json!(null),
        ];
        for value in values {
            let result = call("echo", value.clone()).await.unwrap();
            assert_eq!(result["resultType"], json!("complete"));
            assert_eq!(result["isError"], json!(false));
            assert_eq!(result["content"][0]["type"], json!("text"));
            assert_eq!(result["structuredContent"], value);
            assert_eq!(
                result["content"][0]["text"],
                json!(serde_json::to_string(&value).unwrap()),
            );
        }
    }

    #[tokio::test]
    async fn test_echo_missing_arguments() {
        let result = handle(request("tools/call", json!({ "name": "echo" })))
            .await
            .unwrap();
        assert_eq!(result["structuredContent"], Value::Null);
        assert_eq!(result["content"][0]["text"], json!("null"));
    }

    #[tokio::test]
    #[should_panic]
    async fn test_panic() {
        let _ = call("panic", json!({})).await;
    }
}
