use std::path::PathBuf;

use crate::error::AnyResult;

const APP_DIR_NAME: &str = "nagaicode";

/// Returns the app config directory, creating it if necessary.
pub fn config_dir() -> AnyResult<PathBuf> {
    let base = dirs::data_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not determine the user data directory",
        )
    })?;
    let dir = base.join(APP_DIR_NAME);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}