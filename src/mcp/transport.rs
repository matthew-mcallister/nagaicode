use std::time::Duration;

use futures::future::BoxFuture;

use crate::{config::{McpServerConfig, McpServerConfigType}, json_rpc::{Request, ResponseBody}, mcp::stdio::StdioTransport};

#[derive(Debug)]
pub(crate) enum TransportError {
    /// Request timed out
    Timeout,
    /// Unrecoverable failure. Future requests will fail.
    Unrecoverable,
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "request timed out"),
            Self::Unrecoverable => write!(f, "unrecoverable mcp failure, future requests will fail"),
        }
    }
}

impl std::error::Error for TransportError {}

/// Abstraction around an MCP transport.
pub(crate) trait Transport: std::fmt::Debug + Send + Sync {
    /// Submits a request to the MCP server.
    /// 
    /// The request will be retried until one of three outcomes occur:
    /// 1. Success
    /// 2. Timeout
    /// 3. Permanent failure (MCP server shutdown)
    fn request(
        &self,
        request: Request,
        timeout: Duration,
    ) -> BoxFuture<'static, Result<ResponseBody, TransportError>>;
}

pub(crate) fn make_transport(config: &McpServerConfig) -> Box<dyn Transport> {
    match &config.type_ {
        McpServerConfigType::Stdio => {
            Box::new(StdioTransport::new(config.name.clone(), &config.command))
        },
        McpServerConfigType::Http { url } => todo!(),
    }
}
