use std::time::Duration;

use chrono::{SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{Value, json};

const PROTOCOL_VERSION: &str = "2026-07-28";
const SERVER_NAME: &str = "toy-mcp";
const SERVER_VERSION: &str = "1.0.0";

#[derive(Clone, Debug, PartialEq, Eq)]
struct Error {
    code: i64,
    message: String,
    data: Option<Value>,
}

impl<E> From<E> for Error
    where E: std::error::Error
{
    fn from(value: E) -> Self {
        Self {
            code: -1,
            message: value.to_string(),
            data: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Request {
    jsonrpc: String,
    method: String,
    params: Value,
    id: Value,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct ToolCall {
    name: String,
    #[serde(default)]
    arguments: Value,
}

impl ToolCall {
    fn parse(params: &Value) -> Result<Self, Error> {
        Self::deserialize(params).map_err(|error| invalid_params(error.to_string()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ResponseBody {
    Result(Value),
    Error(Error),
}

impl From<Error> for ResponseBody {
    fn from(value: Error) -> Self {
        Self::Error(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Response {
    jsonrpc: String,
    body: ResponseBody,
    id: Value,
}

fn handle(request: Request) -> Result<Value, Error> {
    match request.method.as_str() {
        "server/discover" => discover(),
        "tools/list" => list_tools(),
        "tools/call" => call_tool(&request.params),
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
        ],
    }))
}

fn call_tool(params: &Value) -> Result<Value, Error> {
    let call = ToolCall::parse(params)?;
    match call.name.as_str() {
        "timestamp" => Ok(timestamp()),
        "echo" => Ok(echo(call.arguments)),
        "panic" => panic!("panic tool invoked"),
        "stall" => {
            loop {
                std::thread::sleep(Duration::from_secs(3600));
            }
        }
        name => Err(invalid_params(format!("unknown tool: {name}"))),
    }
}

fn invalid_params(message: impl Into<String>) -> Error {
    Error {
        code: -32602,
        message: message.into(),
        data: None,
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

fn main() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, params: Value) -> Request {
        Request {
            jsonrpc: "2.0".to_owned(),
            method: method.to_owned(),
            params,
            id: json!(1),
        }
    }

    fn call(name: &str, arguments: Value) -> Result<Value, Error> {
        handle(request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        ))
    }

    #[test]
    fn test_discover() {
        let result = handle(request("server/discover", json!({}))).unwrap();
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

    #[test]
    fn test_list_tools() {
        let result = handle(request("tools/list", json!({}))).unwrap();
        assert_eq!(result["resultType"], json!("complete"));
        let tools = result["tools"].as_array().unwrap();
        let names: Vec<_> = tools.iter().map(|tool| tool["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["timestamp", "echo", "panic", "stall"]);
        for tool in tools {
            assert!(tool["description"].is_string());
            assert!(tool["inputSchema"].is_object());
        }
    }

    #[test]
    fn test_unknown_method() {
        let error = handle(request("no_such_method", json!({}))).unwrap_err();
        assert_eq!(error.code, -32601);
    }

    #[test]
    fn test_unknown_tool() {
        let error = call("no_such_tool", json!({})).unwrap_err();
        assert_eq!(error.code, -32602);
    }

    #[test]
    fn test_missing_tool_name() {
        let error = handle(request("tools/call", json!({}))).unwrap_err();
        assert_eq!(error.code, -32602);
    }

    #[test]
    fn test_timestamp() {
        let result = call("timestamp", json!({})).unwrap();
        assert_eq!(result["resultType"], json!("complete"));
        assert_eq!(result["isError"], json!(false));
        assert_eq!(result["content"][0]["type"], json!("text"));
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed = chrono::DateTime::parse_from_rfc3339(text).unwrap();
        let age = Utc::now().signed_duration_since(parsed);
        assert!(age.num_seconds().abs() < 5);
        assert_eq!(result["structuredContent"]["timestamp"], json!(text));
    }

    #[test]
    fn test_echo() {
        let values = [
            json!({ "a": [1, 2, { "b": null }] }),
            json!("hello"),
            json!([1, true, "x"]),
            json!(null),
        ];
        for value in values {
            let result = call("echo", value.clone()).unwrap();
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

    #[test]
    fn test_echo_missing_arguments() {
        let result = handle(request("tools/call", json!({ "name": "echo" }))).unwrap();
        assert_eq!(result["structuredContent"], Value::Null);
        assert_eq!(result["content"][0]["text"], json!("null"));
    }

    #[test]
    #[should_panic]
    fn test_panic() {
        let _ = call("panic", json!({}));
    }
}
