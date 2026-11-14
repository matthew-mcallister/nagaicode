#![cfg_attr(test, allow(dead_code))]

use std::path::{Path, PathBuf};

use fnv::FnvHashMap;
use serde::Deserialize;

use crate::error::AnyResult;

/// A directory owned by the app; will be mocked during testing.
pub(crate) use crate::cwd::Cwd as AppDir;

const APP_DIR_NAME: &str = "nagaicode";
pub(crate) const CONFIG_FILE_NAME: &str = "config.toml";

fn create_app_dir(base_dir: &Path) -> AnyResult<PathBuf> {
    let dir = base_dir.join(APP_DIR_NAME);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[derive(Debug)]
pub(crate) struct Dirs {
    data: AppDir,
    temp: AppDir,
    // TODO
    //state: AppDir,
}

impl Dirs {
    pub(crate) fn new() -> AnyResult<Self> {
        get_dirs()
    }
    
    pub(crate) fn data(&self) -> &Path {
        &self.data
    }

    pub(crate) fn temp(&self) -> &Path {
        &self.temp
    }
}

#[cfg(not(test))]
fn get_dirs() -> AnyResult<Dirs> {
    use anyhow::anyhow;

    let base = dirs::data_dir().ok_or_else(|| anyhow!("xdg data dir not available"))?;
    let data = create_app_dir(&base)?;

    let base = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    let temp = create_app_dir(&base)?;

    Ok(Dirs {
        data,
        temp,
    })
}

#[cfg(test)]
fn get_dirs() -> AnyResult<Dirs> {
    Ok(Dirs {
        data: AppDir::new(),
        temp: AppDir::new(),
    })
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Config {
    #[serde(default)]
    pub(crate) mcp: McpConfig,
}

impl Config {
    pub(crate) fn load(dirs: &Dirs) -> AnyResult<Self> {
        let path = dirs.data().join(CONFIG_FILE_NAME);
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err.into()),
        };
        let config: Self = toml::from_str(&text)?;

        for (index, server) in config.mcp.server.iter().enumerate() {
            if matches!(server.type_, McpServerConfigType::Stdio) && server.command.is_empty() {
                anyhow::bail!("mcp.server[{index}]: stdio server requires a non-empty `command`");
            }
        }

        Ok(config)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub(crate) struct McpConfig {
    #[serde(rename = "server", default)]
    pub(crate) server: Vec<McpServerConfig>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum McpServerConfigType {
    Stdio,
    Http { url: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub(crate) struct McpServerConfig {
    pub(crate) name: String,
    #[serde(flatten)]
    pub(crate) type_: McpServerConfigType,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) command: Vec<String>,
    #[serde(default)]
    pub(crate) include: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) exclude: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load() {
        let dirs = Dirs::new().unwrap();
        std::fs::write(
            dirs.data().join(CONFIG_FILE_NAME),
            r#"
                [[mcp.server]]
                name = "stdio"
                type = "stdio"
                command = ["/usr/bin/mcp-server", "--foo"]
                enabled = true
                include = ["sound"]
                exclude = ["fury"]

                [[mcp.server]]
                name = "http"
                type = "http"
                url = "http://localhost:1234"
            "#
        ).unwrap();

        let config = Config::load(&dirs).unwrap();

        assert_eq!(config.mcp.server.len(), 2);

        let mcp = &config.mcp.server[0];
        let McpServerConfigType::Stdio = mcp.type_ else { panic!() };
        assert_eq!(mcp.name, "stdio");
        assert_eq!(&mcp.command, &["/usr/bin/mcp-server", "--foo"]);
        assert!(mcp.enabled);
        assert_eq!(mcp.include, Some(vec!["sound".to_string()]));
        assert_eq!(mcp.exclude, vec!["fury".to_string()]);

        let mcp = &config.mcp.server[1];
        let McpServerConfigType::Http { url } = &mcp.type_ else { panic!() };
        assert_eq!(mcp.name, "http");
        assert_eq!(url, "http://localhost:1234");
        assert!(!mcp.enabled);
        assert!(mcp.include.is_none());
        assert!(mcp.exclude.is_empty());
    }

    #[test]
    fn test_load_missing_file() {
        let dirs = Dirs::new().unwrap();
        let config = Config::load(&dirs).unwrap();
        assert!(config.mcp.server.is_empty());
    }
}
