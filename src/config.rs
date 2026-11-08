use std::path::{Path, PathBuf};

use anyhow::anyhow;

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
}

#[cfg(not(test))]
fn get_dirs() -> AnyResult<Dirs> {
    Ok(Dirs {
        data: data_dir()?,
        temp: temp_dir()?,
    })
}

#[cfg(test)]
fn get_dirs() -> AnyResult<Dirs> {
    Ok(Dirs {
        data: AppDir::new(),
        temp: AppDir::new(),
    })
}
    
pub fn data_dir() -> AnyResult<PathBuf> {
    let base = dirs::data_dir().ok_or_else(|| anyhow!("xdg data dir not available"))?;
    create_app_dir(&base)
}

pub fn temp_dir() -> AnyResult<PathBuf> {
    let base = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    create_app_dir(&base)
}
