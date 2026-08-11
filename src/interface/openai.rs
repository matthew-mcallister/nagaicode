use async_openai::Client;
use async_openai::config::OpenAIConfig;

use crate::error::AnyResult;
use crate::interface::InterfaceModel;

#[derive(Debug)]
pub struct OpenaiInterface {
    client: Client<OpenAIConfig>,
}

impl OpenaiInterface {
    pub fn new(api_key: String, base_url: Option<String>) -> Self {
        let mut config = OpenAIConfig::new().with_api_key(api_key);
        if let Some(url) = base_url && !url.is_empty() {
            config = config.with_api_base(url)
        }
        Self {
            client: Client::with_config(config),
        }
    }

    pub async fn get_models(&self) -> AnyResult<Vec<InterfaceModel>> {
        let response = self.client.models().list().await?;
        Ok(response.data.into_iter().map(|m| InterfaceModel { id: m.id }).collect())
    }
}
