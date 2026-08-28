use std::future::Future;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::process::Command;

#[cfg(not(test))]
pub type DefaultToolServer = HostToolServer;
#[cfg(test)]
pub use self::mock::MockToolServer as DefaultToolServer;

/// Describes a tool in human- and model-readable format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// The result of a completed tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolResult {
    /// Free-form text output. Also used for tool errors.
    Text(String),
    /// Structured JSON output.
    Json(Value),
}

impl ToolResult {
    /// Creates a text error result.
    pub fn error(msg: impl Into<String>) -> Self {
        Self::Text(msg.into())
    }
}

/// Trait implemented by tool servers.
pub trait ToolServer {
    /// Returns a description of every available tool.
    fn list_tools(&self) -> impl Iterator<Item = &ToolInfo>;

    /// Executes a tool call, handling all errors internally.
    fn call(
        &mut self,
        name: &str,
        args: Value,
    ) -> impl Future<Output = ToolResult> + Send + 'static;
}

/// Executes tool calls on the host system.
#[derive(Clone, Debug)]
pub struct HostToolServer {
    tools: Arc<Vec<ToolInfo>>,
}

impl HostToolServer {
    /// Creates a new system tool server.
    pub fn new() -> Self {
        Self {
            tools: Arc::new(vec![ToolInfo {
                name: "sh".to_owned(),
                description: "Run a shell command on the host system".to_owned(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string" },
                    },
                    "required": ["command"],
                    "additionalProperties": false,
                }),
            }]),
        }
    }
}

impl ToolServer for HostToolServer {
    fn list_tools(&self) -> impl Iterator<Item = &ToolInfo> {
        self.tools.iter()
    }

    fn call(
        &mut self,
        name: &str,
        args: Value,
    ) -> impl Future<Output = ToolResult> + Send + 'static {
        let name = name.to_owned();
        async move {
            match name.as_str() {
                "sh" => {
                    let cmd: Option<String> = match args {
                        Value::Object(mut obj) => match obj.remove("command") {
                            Some(Value::String(s)) => Some(s),
                            _ => None,
                        },
                        _ => None,
                    };
                    let Some(cmd) = cmd else {
                        return ToolResult::error(
                            "invalid arguments for 'sh': expected {\"command\": \"...\"}",
                        )
                    };
                    let output = match Command::new("sh").arg("-c").arg(cmd).output().await {
                        Ok(output) => output,
                        Err(e) => return ToolResult::error(format!("failed to run 'sh': {e}")),
                    };
                    let return_code = output.status.code().unwrap_or(-1);
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    ToolResult::Json(json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "return_code": return_code,
                    }))
                }
                _ => ToolResult::error(format!("unknown tool: '{name}'")),
            }
        }
    }
}

// FIXME: tool server should not be mocked, this prevents testing tools.
// Instead consider mocking or sandboxing subcommands and file I/O directly
#[cfg(test)]
pub mod mock {
    use std::collections::VecDeque;
    use std::future::Future;
    use std::sync::{Arc, Mutex};

    use fnv::FnvHashMap;
    use serde_json::Value;

    use super::{ToolInfo, ToolResult, ToolServer};

    /// Recorded tool call.
    #[derive(Clone, Debug, PartialEq)]
    pub struct ToolCall {
        pub name: String,
        pub args: Value,
    }

    #[derive(Debug, Default)]
    struct MockToolServerInner {
        results: FnvHashMap<String, VecDeque<ToolResult>>,
        calls: Vec<ToolCall>,
    }

    /// Mock tool server for tests.
    #[derive(Clone, Debug)]
    pub struct MockToolServer {
        inner: Arc<Mutex<MockToolServerInner>>,
        tools: Vec<ToolInfo>,
    }

    impl MockToolServer {
        pub fn new() -> Self {
            Self {
                inner: Default::default(),
                tools: Default::default(),
            }
        }

        /// Enqueues a result.
        pub fn add_result(&mut self, name: &str, result: ToolResult) {
            let mut inner = self.inner.lock().unwrap();
            inner.results
                .entry(name.to_owned())
                .or_default()
                .push_back(result);
        }

        /// Enqueues multiple results.
        pub fn add_results(
            &mut self,
            name: &str,
            results: impl IntoIterator<Item = ToolResult>,
        ) {
            let mut inner = self.inner.lock().unwrap();
            let queue = inner.results.entry(name.to_owned()).or_default();
            queue.extend(results);
        }

        /// Clears all pending results.
        pub fn clear_results(&self) {
            let mut inner = self.inner.lock().unwrap();
            inner.results.clear();
        }

        /// Returns all recorded tool calls.
        pub fn get_calls(&self) -> Vec<ToolCall> {
            let inner = self.inner.lock().unwrap();
            inner.calls.clone()
        }

        /// Clears all recorded tool calls.
        pub fn clear_calls(&self) {
            let mut inner = self.inner.lock().unwrap();
            inner.calls.clear();
        }

        /// Replaces the advertised tool list.
        pub fn set_tools(&mut self, tools: Vec<ToolInfo>) {
            self.tools = tools;
        }
    }

    impl ToolServer for MockToolServer {
        fn list_tools(&self) -> impl Iterator<Item = &ToolInfo> {
            self.tools.iter()
        }

        fn call(
            &mut self,
            name: &str,
            args: Value,
        ) -> impl Future<Output = ToolResult> + Send + 'static {
            let inner = self.inner.clone();
            let name = name.to_owned();
            async move {
                let mut inner = inner.lock().unwrap();
                inner.calls.push(ToolCall {
                    name: name.clone(),
                    args,
                });
                match inner
                    .results
                    .get_mut(&name)
                    .and_then(VecDeque::pop_front)
                {
                    Some(res) => res,
                    None => panic!("empty queue tool={name}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::mock::{MockToolServer, ToolCall};
    use super::*;

    #[tokio::test]
    async fn test_mock_tool_server() {
        let mut server = MockToolServer::new();
        assert!(server.list_tools().next().is_none());

        let tool = ToolInfo {
            name: "sh".to_owned(),
            description: "Run a shell command".to_owned(),
            input_schema: json!({ "type": "object" }),
        };
        server.set_tools(vec![tool.clone()]);
        assert_eq!(server.list_tools().collect::<Vec<_>>(), vec![&tool]);

        let mut value = serde_json::to_value(&tool).unwrap();
        assert_eq!(
            value,
            json!({
                "name": "sh",
                "description": "Run a shell command",
                "input_schema": { "type": "object" },
            })
        );
        value["type"] = json!("function");
        let parsed: ToolInfo = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, tool);

        server.add_result("sh", ToolResult::Text("output 1".to_owned()));
        let res = server.call("sh", json!({ "command": "echo test" })).await;
        assert_eq!(res, ToolResult::Text("output 1".to_owned()));

        let calls = server.get_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            ToolCall {
                name: "sh".to_owned(),
                args: json!({ "command": "echo test" }),
            }
        );

        server.clear_calls();
        assert!(server.get_calls().is_empty());

        server.add_result("sh", ToolResult::error("command failed"));
        let res = server.call("sh", json!({ "command": "false" })).await;
        assert_eq!(res, ToolResult::Text("command failed".to_owned()));

        server.add_result("sh", ToolResult::Text("execution failed".to_owned()));
        let res = server.call("sh", json!({ "command": "bad command" })).await;
        assert_eq!(res, ToolResult::Text("execution failed".to_owned()));

        let calls = server.get_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].args, json!({ "command": "false" }));
        assert_eq!(calls[1].args, json!({ "command": "bad command" }));

        server.add_results("sh", [
            ToolResult::Text("first".to_owned()),
            ToolResult::Text("second".to_owned()),
        ]);
        let res1 = server.call("sh", json!({ "command": "1" })).await;
        assert_eq!(res1, ToolResult::Text("first".to_owned()));
        let res2 = server.call("sh", json!({ "command": "2" })).await;
        assert_eq!(res2, ToolResult::Text("second".to_owned()));

        server.add_result("sh", ToolResult::Text("stale".to_owned()));
        server.clear_results();
        server.add_result("sh", ToolResult::Text("fresh".to_owned()));
        let res = server.call("sh", json!({ "command": "fresh test" })).await;
        assert_eq!(res, ToolResult::Text("fresh".to_owned()));

        let mut sys_server = HostToolServer::new();
        let sys_tools: Vec<&ToolInfo> = sys_server.list_tools().collect();
        assert_eq!(sys_tools.len(), 1);
        assert_eq!(sys_tools[0].name, "sh");

        let sys_res = sys_server
            .call("sh", json!({ "command": "printf 'hello'" }))
            .await;
        assert_eq!(
            sys_res,
            ToolResult::Json(json!({
                "stdout": "hello",
                "stderr": "",
                "return_code": 0,
            }))
        );

        let sys_err_res = sys_server
            .call("sh", json!({ "command": "printf 'err' >&2; exit 1" }))
            .await;
        assert_eq!(
            sys_err_res,
            ToolResult::Json(json!({
                "stdout": "",
                "stderr": "err",
                "return_code": 1,
            }))
        );

        let unknown = sys_server.call("unknown", json!({})).await;
        assert_eq!(unknown, ToolResult::error("unknown tool: 'unknown'"));
        let bad_args = sys_server.call("sh", json!("echo test")).await;
        assert_eq!(
            bad_args,
            ToolResult::error("invalid arguments for 'sh': expected {\"command\": \"...\"}")
        );
        let bad_command = sys_server.call("sh", json!({ "command": 123 })).await;
        assert_eq!(
            bad_command,
            ToolResult::error("invalid arguments for 'sh': expected {\"command\": \"...\"}")
        );
    }
}
