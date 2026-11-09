#![cfg_attr(test, allow(dead_code))]

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::AnyResult;

/// A directory owned by the app; will be mocked during testing.
pub(crate) use crate::cwd::Cwd as AppDir;

const APP_DIR_NAME: &str = "nagaicode";
const CONFIG_FILE_NAME: &str = "config.toml";

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

    pub(crate) fn session_storage(&self, session_id: i32) -> PathBuf {
        self.temp()
            .join("session_storage")
            .join(session_id.to_string())
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

#[derive(Debug, Default, Deserialize)]
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
        Ok(toml::from_str(&text)?)
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct McpConfig {
    #[serde(rename = "server", default)]
    pub(crate) server: Vec<McpServerConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(crate) enum McpServerConfigType {
    Stdio { path: String },
    Http { url: String },
}

#[derive(Debug, Deserialize)]
pub(crate) struct McpServerConfig {
    #[serde(flatten)]
    pub(crate) type_: McpServerConfigType,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) include: Vec<String>,
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
                type = "stdio"
                path = "/usr/bin/mcp-server"
                enabled = true
                include = ["sound"]
                exclude = ["fury"]

                [[mcp.server]]
                type = "http"
                url = "http://localhost:1234"
            "#
        ).unwrap();

        let config = Config::load(&dirs).unwrap();

        assert_eq!(config.mcp.server.len(), 2);

        let stdio = &config.mcp.server[0];
        match &stdio.type_ {
            McpServerConfigType::Stdio { path } => assert_eq!(path, "/usr/bin/mcp-server"),
            other => panic!("unexpected server type: {other:?}"),
        }
        assert!(stdio.enabled);
        assert_eq!(stdio.include, vec!["sound".to_string()]);
        assert_eq!(stdio.exclude, vec!["fury".to_string()]);

        let http = &config.mcp.server[1];
        match &http.type_ {
            McpServerConfigType::Http { url } => assert_eq!(url, "http://localhost:1234"),
            other => panic!("unexpected server type: {other:?}"),
        }
        assert!(!http.enabled);
        assert!(http.include.is_empty());
        assert!(http.exclude.is_empty());
    }

    #[test]
    fn test_load_missing_file() {
        let dirs = Dirs::new().unwrap();
        let config = Config::load(&dirs).unwrap();
        assert!(config.mcp.server.is_empty());
    }
}
