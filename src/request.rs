//! Async HTTP request wrapper library with mock support.

use reqwest::header::HeaderMap;
use reqwest_eventsource::{Event, EventSource};
use futures::{Stream, StreamExt};
use reqwest::{Request, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;

use crate::error::AnyResult;

#[cfg(not(test))]
pub type DefaultClient = reqwest::Client;
#[cfg(test)]
pub use self::test_client::DefaultClient;

/// Wrapper around reqwest's response type.
pub trait Response: Sized {
    fn status(&self) -> StatusCode;

    fn headers(&self) -> &HeaderMap;

    #[allow(async_fn_in_trait)]
    async fn text(self) -> AnyResult<String>;

    #[allow(async_fn_in_trait)]
    async fn json<T: DeserializeOwned>(self) -> AnyResult<T>;

    fn error_for_status(self) -> AnyResult<Self>;
}

impl Response for reqwest::Response {
    fn status(&self) -> StatusCode {
        self.status()
    }

    fn headers(&self) -> &HeaderMap {
        self.headers()
    }

    async fn text(self) -> AnyResult<String> {
        Ok(self.text().await?)
    }

    async fn json<T: DeserializeOwned>(self) -> AnyResult<T> {
        Ok(self.json().await?)
    }

    fn error_for_status(self) -> AnyResult<Self> {
        Ok(self.error_for_status()?)
    }
}

pub trait Client {
    type Response: Response;

    /// Executes a standard HTTP request.
    #[allow(async_fn_in_trait)]
    async fn execute(&self, request: Request) -> AnyResult<Self::Response>;

    /// Constructs a pollable event source for an SSE endpoint.
    fn stream(&self, request: Request) -> impl Stream<Item = AnyResult<Event>>;
}

impl Client for reqwest::Client {
    type Response = reqwest::Response;

    async fn execute(&self, request: Request) -> AnyResult<Self::Response> {
        Ok(self.execute(request).await?)
    }

    fn stream(&self, request: Request) -> impl Stream<Item = AnyResult<Event>> {
        EventSource::new(RequestBuilder::from_parts(self.clone(), request))
            .unwrap()
            .map(|res| Ok(res?))
    }
}

#[cfg(test)]
pub mod test_client {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use fnv::FnvHashMap;
    use futures::Stream;
    use reqwest::{Request, StatusCode};
    use reqwest::header::HeaderMap;
    use reqwest_eventsource::Event;

    use crate::error::AnyResult;
    use crate::testing::QueueStream;
    use super::Client;
    use reqwest_eventsource::Error as SseError;

    #[derive(Clone, Debug)]
    pub struct Response {
        pub body: String,
        pub status: StatusCode,
        pub headers: HeaderMap,
    }

    impl super::Response for Response {
        fn status(&self) -> StatusCode {
            self.status
        }

        fn headers(&self) -> &HeaderMap {
            &self.headers
        }

        async fn text(self) -> AnyResult<String> {
            Ok(self.body.clone())
        }

        async fn json<T: serde::de::DeserializeOwned>(self) -> AnyResult<T> {
            Ok(serde_json::from_str(&self.body)?)
        }

        fn error_for_status(self) -> AnyResult<Self> {
            if self.status.is_client_error() || self.status.is_server_error() {
                Err(format!("HTTP status error: {}", self.status).into())
            } else {
                Ok(self)
            }
        }
    }

    #[derive(Debug)]
    pub enum ResponseData {
        Http(AnyResult<Response>),
        Sse(QueueStream<AnyResult<Event>>),
    }

    #[derive(Debug, Default)]
    struct DefaultClientInner {
        /// Maps endpoints to queued responses
        response_data: FnvHashMap<String, VecDeque<ResponseData>>,
        /// Logs all requests
        requests: Vec<Request>,
    }

    /// Mock client for tests. Yields response data based on URL.
    ///
    /// Popping the wrong kind of event data for a request (a stream for a
    /// standard request or a standard response for an SSE request) causes a
    /// panic.
    #[derive(Clone, Debug, Default)]
    pub struct DefaultClient {
        inner: Arc<Mutex<DefaultClientInner>>,
    }

    impl DefaultClient {
        /// Enqueues a response.
        pub fn add_response(
            &mut self,
            url: &str,
            response: ResponseData,
        ) {
            let mut inner = self.inner.lock().unwrap();
            inner.response_data
                .entry(url.to_owned())
                .or_default()
                .push_back(response);
        }

        /// Enqueues multiple responses.
        pub fn add_responses(
            &mut self,
            url: &str,
            responses: impl IntoIterator<Item = ResponseData>,
        ) {
            let mut inner = self.inner.lock().unwrap();
            let queue = inner.response_data.entry(url.to_owned()).or_default();
            queue.extend(responses);
        }

        /// Clears all pending responses.
        pub fn clear_responses(&self) {
            let mut inner = self.inner.lock().unwrap();
            inner.response_data.clear();
        }

        /// Returns (a copy of) all requests made to the client.
        pub fn get_requests(&self) -> Vec<Request> {
            let inner = self.inner.lock().unwrap();
            inner.requests
                .iter()
                .filter_map(|r| r.try_clone())
                .collect()
        }

        /// Clears all recorded requests.
        pub fn clear_requests(&self) {
            let mut inner = self.inner.lock().unwrap();
            inner.requests.clear();
        }
    }

    impl Client for DefaultClient {
        type Response = Response;

        async fn execute(&self, request: Request) -> AnyResult<Self::Response> {
            let url = request.url().to_string();
            let mut inner = self.inner.lock().unwrap();
            inner.requests.push(request);
            let data = inner
                .response_data
                .get_mut(&url)
                .and_then(VecDeque::pop_front);
            match data {
                Some(ResponseData::Http(res)) => res,
                Some(ResponseData::Sse(_)) => panic!("wrong response type url={url}"),
                None => panic!("empty queue url={url}"),
            }
        }

        fn stream(&self, request: Request) -> impl Stream<Item = AnyResult<Event>> {
            let url = request.url().to_string();
            let mut inner = self.inner.lock().unwrap();
            inner.requests.push(request);
            let data = inner
                .response_data
                .get_mut(&url)
                .and_then(VecDeque::pop_front);
            match data {
                Some(ResponseData::Sse(mut stream)) => {
                    stream.0.push_back(Err(SseError::StreamEnded.into()));
                    stream
                }
                Some(ResponseData::Http(_)) => panic!("wrong response type url={url}"),
                None => panic!("empty queue url={url}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DefaultClient, Response as ResponseTrait};
    use super::test_client::{ResponseData, Response};
    use crate::error::AnyResult;
    use crate::request::Client;
    use crate::testing::QueueStream;
    use futures::StreamExt;
    use reqwest::{Method, StatusCode};
    use reqwest::header::HeaderMap;
    use reqwest_eventsource::Event;
    use serde::Deserialize;

    fn build_request(method: Method, url: &str) -> reqwest::Request {
        reqwest::Client::new()
            .request(method, url)
            .build()
            .unwrap()
    }

    fn make_response(status: StatusCode, body: impl Into<String>) -> ResponseData {
        ResponseData::Http(Ok(Response {
            body: body.into(),
            status,
            headers: HeaderMap::new(),
        }))
    }

    #[tokio::test]
    async fn test_mock_client() {
        #[derive(Deserialize)]
        struct Body {
            key: String,
        }

        let mut client = DefaultClient::default();

        let ok_url = "https://example.com/api";
        client.add_response(ok_url, make_response(StatusCode::OK, r#"{"key":"value"}"#));

        // test GET 200
        let response = client.execute(build_request(Method::GET, ok_url)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().is_empty());

        let response = response.error_for_status().unwrap();
        let body: Body = response.json().await.unwrap();
        assert_eq!(body.key, "value");

        let requests = client.get_requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method(), Method::GET);
        assert_eq!(requests[0].url().as_str(), ok_url);

        client.clear_requests();
        assert!(client.get_requests().is_empty());

        // test 404
        let err_url = "https://example.com/missing";
        client.add_response(err_url, make_response(StatusCode::NOT_FOUND, String::new()));
        let err_response = client.execute(build_request(Method::GET, err_url)).await.unwrap();
        assert_eq!(err_response.status(), StatusCode::NOT_FOUND);
        assert!(err_response.error_for_status().is_err());

        // test SSE
        let event = eventsource_stream::Event {
            event: "my-event".into(),
            data: r#"{"the":"data"}"#.into(),
            id: "the-id".into(),
            retry: None,
        };
        let stream_url = "https://example.com/stream";
        let events: Vec<AnyResult<Event>> = vec![Ok(Event::Open), Ok(Event::Message(event.clone()))];
        client.add_response(
            stream_url,
            ResponseData::Sse(QueueStream::from(events)),
        );
        let stream = client.stream(build_request(Method::POST, stream_url));
        let collected: Vec<AnyResult<Event>> = stream.collect().await;
        assert_eq!(collected.len(), 3);
        assert!(matches!(collected[0], Ok(Event::Open)));
        assert_eq!(collected[1].as_ref().ok(), Some(&Event::Message(event)));
        assert_eq!(
            collected[2].as_ref().unwrap_err().to_string(),
            "Stream ended",
        );

        let requests = client.get_requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].method(), Method::POST);
        assert_eq!(requests[1].url().as_str(), stream_url);

        // multiple responses served in order
        let multi_url = "https://example.com/multi";
        client.add_responses(multi_url, [
            make_response(StatusCode::OK, "1"),
            make_response(StatusCode::OK, "2"),
        ]);
        for expected in ["1", "2"] {
            let resp = client.execute(build_request(Method::GET, multi_url)).await.unwrap();
            assert_eq!(resp.body, expected);
        }

        // clear_responses
        client.add_response(multi_url, make_response(StatusCode::OK, "stale"));
        client.clear_responses();
        client.add_response(multi_url, make_response(StatusCode::OK, "fresh"));
        let resp = client.execute(build_request(Method::GET, multi_url)).await.unwrap();
        assert_eq!(resp.body, "fresh");
    }
}
