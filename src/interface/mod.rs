pub mod openai;

use std::error::Error;
use std::str::FromStr;

use crate::error::AnyResult;
use crate::interface::openai::OpenaiInterface;
use crate::provider::Provider;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum InterfaceId {
    Openai,
}

impl FromStr for InterfaceId {
    type Err = Box<dyn Error + Send + Sync>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "openai" => Ok(Self::Openai),
            _ => Err(From::from(format!("Invalid interface: '{}'", s))),
        }
    }
}

impl InterfaceId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
        }
    }
}

impl std::fmt::Display for InterfaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Clone, Debug)]
pub struct InterfaceModel {
    pub id: String,
}

/// Wraps around an inference API.
#[derive(Debug)]
pub enum Interface {
    Openai(OpenaiInterface),
}

impl From<OpenaiInterface> for Interface {
    fn from(value: OpenaiInterface) -> Self {
        Self::Openai(value)
    }
}

impl Interface {
    pub fn from_provider(provider: &Provider) -> AnyResult<Self> {
        let id: InterfaceId = provider.interface.parse()?;
        Ok(match id {
            InterfaceId::Openai => Self::from(OpenaiInterface::new(
                provider.api_key.clone(),
                provider.base_url.clone(),
            )),
        })
    }

    /// Fetches the list of models offered by the provider.
    pub async fn get_models(&self) -> AnyResult<Vec<InterfaceModel>> {
        match self {
            Self::Openai(iface) => iface.get_models().await,
        }
    }
}
