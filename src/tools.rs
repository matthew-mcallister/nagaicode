use std::process::Command;

use serde_json::Value;

use crate::error::AnyResult;

#[cfg(not(test))]
pub type DefaultToolServer = HostToolServer;
#[cfg(test)]
pub use self::mock::MockToolServer as DefaultToolServer;

/// Describes a tool in human- and model-readable format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
}

/// The result of a completed tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolResult {
    pub content: Value,
    pub is_error: bool,
}

impl ToolResult {
    pub fn success(content: Value) -> Self {
        Self { content, is_error: false }
    }

    pub fn error(content: Value) -> Self {
        Self { content, is_error: true }
    }
}

/// Trait implemented by tool servers.
pub trait ToolServer {
    /// Attempts to execute a tool call and return the result.
    ///
    /// If the tool call was completed, then `Ok(_)` is returned, even if the
    /// tool itself encountered an error. `Err(_)` is only returned when there
    /// was an issue calling the tool, such as an invalid tool name, invalid
    /// arguments, an I/O error, etc.
    fn call(&mut self, name: &str, args: Value) -> AnyResult<ToolResult>;
}

/// Executes tool calls on the host system.
#[derive(Clone, Copy, Debug, Default)]
pub struct HostToolServer;

impl HostToolServer {
    /// Creates a new system tool server.
    pub fn new() -> Self {
        Self
    }
}

impl ToolServer for HostToolServer {
    fn call(&mut self, name: &str, args: Value) -> AnyResult<ToolResult> {
        match name {
            "sh" => {
                let cmd = match args {
                    Value::String(s) => s,
                    _ => return Err("invalid arguments for 'sh': expected a string".into()),
                };
                let output = Command::new("sh")
                    .arg("-c")
                    .arg(cmd)
                    .output()?;
                let mut content = String::from_utf8_lossy(&output.stdout).into_owned();
                content.push_str(&String::from_utf8_lossy(&output.stderr));
                Ok(ToolResult {
                    content: Value::String(content),
                    is_error: !output.status.success(),
                })
            }
            _ => Err(format!("unknown tool: '{name}'").into()),
        }
    }
}

#[cfg(test)]
pub mod mock {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use fnv::FnvHashMap;
    use serde_json::Value;

    use crate::error::AnyResult;
    use super::{ToolResult, ToolServer};

    /// Recorded tool call.
    #[derive(Clone, Debug, PartialEq)]
    pub struct ToolCall {
        pub name: String,
        pub args: Value,
    }

    #[derive(Debug, Default)]
    struct MockToolServerInner {
        results: FnvHashMap<String, VecDeque<AnyResult<ToolResult>>>,
        calls: Vec<ToolCall>,
    }

    /// Mock tool server for tests.
    #[derive(Clone, Debug, Default)]
    pub struct MockToolServer {
        inner: Arc<Mutex<MockToolServerInner>>,
    }

    impl MockToolServer {
        /// Enqueues a result.
        pub fn add_result(
            &mut self,
            name: &str,
            result: AnyResult<ToolResult>,
        ) {
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
            results: impl IntoIterator<Item = AnyResult<ToolResult>>,
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
    }

    impl ToolServer for MockToolServer {
        fn call(&mut self, name: &str, args: Value) -> AnyResult<ToolResult> {
            let mut inner = self.inner.lock().unwrap();
            inner.calls.push(ToolCall {
                name: name.to_owned(),
                args,
            });
            let data = inner
                .results
                .get_mut(name)
                .and_then(VecDeque::pop_front);
            match data {
                Some(res) => res,
                None => panic!("empty queue tool={name}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use super::mock::{MockToolServer, ToolCall};

    #[test]
    fn test_mock_tool_server() {
        let mut server = MockToolServer::default();

        server.add_result("sh", Ok(ToolResult::success(json!("output 1"))));

        let res = server.call("sh", json!("echo test")).unwrap();
        assert_eq!(res.content, json!("output 1"));
        assert!(!res.is_error);

        let calls = server.get_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0],
            ToolCall {
                name: "sh".to_owned(),
                args: json!("echo test"),
            }
        );

        server.clear_calls();
        assert!(server.get_calls().is_empty());

        server.add_result("sh", Ok(ToolResult::error(json!("command failed"))));
        let res = server.call("sh", json!("false")).unwrap();
        assert_eq!(res.content, json!("command failed"));
        assert!(res.is_error);

        server.add_result("sh", Err("execution failed".into()));
        let err = server.call("sh", json!("bad command")).unwrap_err();
        assert_eq!(err.to_string(), "execution failed");

        let calls = server.get_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].args, json!("false"));
        assert_eq!(calls[1].args, json!("bad command"));

        server.add_results("sh", [
            Ok(ToolResult::success(json!("first"))),
            Ok(ToolResult::success(json!("second"))),
        ]);
        let res1 = server.call("sh", json!("1")).unwrap();
        assert_eq!(res1.content, json!("first"));
        let res2 = server.call("sh", json!("2")).unwrap();
        assert_eq!(res2.content, json!("second"));

        server.add_result("sh", Ok(ToolResult::success(json!("stale"))));
        server.clear_results();
        server.add_result("sh", Ok(ToolResult::success(json!("fresh"))));
        let res = server.call("sh", json!("fresh test")).unwrap();
        assert_eq!(res.content, json!("fresh"));

        let mut sys_server = HostToolServer::new();
        let sys_res = sys_server.call("sh", json!("printf 'hello'")).unwrap();
        assert_eq!(sys_res.content, json!("hello"));
        assert!(!sys_res.is_error);

        let sys_err_res = sys_server.call("sh", json!("exit 1")).unwrap();
        assert!(sys_err_res.is_error);

        assert!(sys_server.call("unknown", json!("test")).is_err());
        assert!(sys_server.call("sh", json!(123)).is_err());
    }
}
