use serde::{Deserialize, Serialize};
use serde::ser::SerializeMap;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Error {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
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

impl Error {
    pub fn invalid_params(message: impl Into<String>) -> Error {
        Error {
            code: -32602,
            message: message.into(),
            data: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ErrorStd(pub Error);

impl std::fmt::Display for ErrorStd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "code {}: {}", self.0.code, self.0.message)
    }
}

impl std::error::Error for ErrorStd {}

pub type RequestId = Value;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct Request {
    pub method: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default)]
    pub id: Option<RequestId>,
}

impl Serialize for Request {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("jsonrpc", "2.0")?;
        map.serialize_entry("method", &self.method)?;
        map.serialize_entry("params", &self.params)?;
        map.serialize_entry("id", &self.id)?;
        map.end()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ResponseBody {
    #[serde(rename = "result")]
    Result(Value),
    #[serde(rename = "error")]
    Error(Error),
}

impl From<Error> for ResponseBody {
    fn from(value: Error) -> Self {
        Self::Error(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct Response {
    #[serde(flatten)]
    pub body: ResponseBody,
    pub id: RequestId,
}

impl Serialize for Response {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(3))?;
        map.serialize_entry("jsonrpc", "2.0")?;
        match &self.body {
            ResponseBody::Result(result) => map.serialize_entry("result", result)?,
            ResponseBody::Error(error) => map.serialize_entry("error", error)?,
        }
        map.serialize_entry("id", &self.id)?;
        map.end()
    }
}
