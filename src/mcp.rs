use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::future::join_all;
use log::{error, trace};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config::{Config, McpServerConfig};
use crate::error::AnyResult;
use crate::json_rpc::{ErrorStd, Request, ResponseBody};
use crate::mcp::tool::{McpTool, McpToolDef};
use crate::mcp::transport::{Transport, TransportError, make_transport};
use crate::task::{Task, TaskContext};
use crate::tools::ToolRegistry;

mod stdio;
mod tool;
mod transport;

const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolsListResult {
    tools: Vec<McpToolDef>,
}

#[derive(Debug)]
struct McpDiscoveryInfo {
    tools: Vec<McpToolDef>,
}

#[derive(Debug)]
pub(crate) struct McpServer {
    config: McpServerConfig,
    transport: Box<dyn Transport>,
    request_id: AtomicU64,
}

impl McpServer {
    fn from_config(config: McpServerConfig) -> Self {
        Self {
            transport: make_transport(&config),
            config,
            request_id: AtomicU64::new(0),
        }
    }

    async fn request(
        &self,
        method: impl Into<String>,
        params: impl Into<Value>,
    ) -> Result<ResponseBody, TransportError> {
        let id = self.request_id.fetch_add(1, Ordering::Relaxed);
        let request = Request {
            method: method.into(),
            params: params.into(),
            id: Some(json!(id)),
        };
        let future = self.transport.request(request, TIMEOUT);
        future.await
    }

    async fn list_tools(&self) -> AnyResult<ToolsListResult> {
        let body = self.request("tools/list", json!({})).await?;
        let result = match body {
            ResponseBody::Result(result) => result,
            ResponseBody::Error(e) => Err(ErrorStd(e))?,
        };
        Ok(serde_json::from_value(result)?)
    }

    async fn discover(&self) -> AnyResult<McpDiscoveryInfo> {
        let ToolsListResult { tools } = self.list_tools().await?;
        let mut filtered_tools = Vec::new();
        for tool in tools {
            let mut skip = self.config.exclude.contains(&tool.name);
            if let Some(include) = &self.config.include && !include.contains(&tool.name) {
                skip = true;
            }
            if skip {
                trace!("mcp server '{}': ignored tool '{}'", self.config.name, tool.name);
            } else {
                trace!("mcp server '{}': discovered tool '{}'", self.config.name, tool.name);
                filtered_tools.push(tool);
            }
        }
        Ok(McpDiscoveryInfo {
            tools: filtered_tools,
        })
    }
}

/// Registers MCP servers. Servers will be queries in parallel, but results are
/// processed in the order in which the appear in the configuration file so the
/// results are deterministic.
pub(crate) async fn register_mcp_servers(
    config: &Config,
    tools: &ToolRegistry,
) {
    let servers: Vec<Arc<McpServer>> = config.mcp.server.iter()
        .filter(|config| config.enabled)
        .map(|config| Arc::new(McpServer::from_config(config.clone())))
        .collect();
    let discoveries = join_all(servers.iter().map(|server| server.discover())).await;
    for (server, discovery) in servers.iter().zip(discoveries) {
        match discovery {
            Ok(info) => {
                for def in info.tools {
                    tools.register(Box::new(McpTool::new(Arc::clone(server), def)));
                }
            },
            Err(e) => error!("mcp server '{}': {e}", server.config.name),
        }
    }
}

/// Background task that registers tools from configured MCP servers.
#[derive(Debug)]
pub(crate) struct RegisterMcpServers {
    config: Config,
}

impl RegisterMcpServers {
    pub(crate) fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Task for RegisterMcpServers {
    type Output = ();

    async fn run(self, context: &mut TaskContext) {
        register_mcp_servers(&self.config, context.tool_registry()).await
    }
}
