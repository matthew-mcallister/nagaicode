use std::path::{Path, PathBuf};

use anyhow::anyhow;

use crate::error::AnyResult;

const APP_DIR_NAME: &str = "nagaicode";

fn create_app_dir(base_dir: &Path) -> AnyResult<PathBuf> {
    let dir = base_dir.join(APP_DIR_NAME);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Returns the app config directory, creating it if necessary.
pub fn data_dir() -> AnyResult<PathBuf> {
    let base = dirs::data_dir().ok_or_else(|| anyhow!("xdg data dir not available"))?;
    create_app_dir(&base)
}

pub fn temp_dir() -> AnyResult<PathBuf> {
    let base = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    create_app_dir(&base)
}
