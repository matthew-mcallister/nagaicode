#[cfg(test)]
use std::sync::Mutex;

use serde::Deserialize;

use crate::error::AnyResult;
use crate::interface::InterfaceModel;

/// Real client
#[cfg(not(test))]
#[derive(Debug)]
pub struct Client {
    base_url: String,
    api_key: String,
    inner: reqwest::Client,
}

#[cfg(not(test))]
impl Client {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            base_url,
            api_key,
            inner: reqwest::Client::new(),
        }
    }

    pub async fn get(&self, endpoint: &str) -> AnyResult<String> {
        let response = self
            .inner
            .get(format!("{}{}", self.base_url, endpoint))
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.text().await?)
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct MockRequest {
    pub method: &'static str,
    pub url: String,
}

/// Mock client
#[cfg(test)]
#[derive(Debug)]
pub struct Client {
    base_url: String,
    #[allow(dead_code)]
    api_key: String,
    calls: Mutex<Vec<MockRequest>>,
}

#[cfg(test)]
impl Client {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            base_url,
            api_key,
            calls: Default::default(),
        }
    }

    pub async fn get(&self, endpoint: &str) -> AnyResult<String> {
        let request = MockRequest {
            method: "GET",
            url: format!("{}{}", self.base_url, endpoint),
        };
        self.calls.lock().unwrap().push(request);
        Ok(r#"{"data":[{"id":"fake-model"}]}"#.to_owned())
    }

    pub fn calls(&self) -> Vec<MockRequest> {
        self.calls.lock().unwrap().clone()
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
}

#[derive(Debug)]
pub struct OpenaiInterface {
    client: Client,
}

impl OpenaiInterface {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self {
            client: Client::new(base_url, api_key),
        }
    }

    pub async fn get_models(&self) -> AnyResult<Vec<InterfaceModel>> {
        let body = self.client.get("/models").await?;
        let parsed: ModelsResponse = serde_json::from_str(&body)?;
        Ok(parsed.data.into_iter().map(|m| InterfaceModel { id: m.id }).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(url: &str) -> MockRequest {
        MockRequest { method: "GET", url: url.to_owned() }
    }

    // Also tests get_models
    #[tokio::test]
    async fn test_list_models() {
        // Normal function
        let iface = OpenaiInterface::new("https://example.test/v1".into(), "sk-test".into());
        let models = iface.get_models().await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "fake-model");
        assert_eq!(iface.client.calls(), vec![get("https://example.test/v1/models")]);
    }
}
