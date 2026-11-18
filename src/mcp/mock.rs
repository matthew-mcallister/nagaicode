#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use tokio::sync::Mutex;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::json_rpc::{Request, ResponseBody};
use crate::mcp::transport::{Transport, TransportError};

#[derive(Debug)]
pub(crate) struct MockTransport {
    responses: Arc<Mutex<UnboundedReceiver<Result<ResponseBody, TransportError>>>>,
    requests: UnboundedSender<Request>,
}

impl MockTransport {
    pub(crate) fn new(
        responses: UnboundedReceiver<Result<ResponseBody, TransportError>>,
        requests: UnboundedSender<Request>,
    ) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses)),
            requests,
        }
    }
}

impl Transport for MockTransport {
    fn request(
        &self,
        request: Request,
        timeout: Duration,
    ) -> BoxFuture<'static, Result<ResponseBody, TransportError>> {
        let requests = self.requests.clone();
        let responses = Arc::clone(&self.responses);
        Box::pin(async move {
            if requests.send(request).is_err() {
                return Err(TransportError::Unrecoverable);
            }
            let mut responses = responses.lock().await;
            match tokio::time::timeout(timeout, responses.recv()).await {
                Ok(Some(response)) => response,
                Ok(None) => Err(TransportError::Unrecoverable),
                Err(_) => Err(TransportError::Timeout),
            }
        })
    }
}
