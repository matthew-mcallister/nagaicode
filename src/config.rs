#![cfg_attr(test, allow(dead_code))]

use std::path::{Path, PathBuf};

use crate::error::AnyResult;

/// A directory owned by the app; will be mocked during testing.
pub(crate) use crate::cwd::Cwd as AppDir;

const APP_DIR_NAME: &str = "nagaicode";

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
