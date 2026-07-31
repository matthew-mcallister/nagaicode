use std::fs;
use std::path::PathBuf;

use rusqlite::Connection;

const APP_DIR_NAME: &str = "nagaicode";
const DB_FILE_NAME: &str = "db.sqlite";

fn db_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let base = dirs::data_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "could not determine the user data directory",
        )
    })?;
    let dir = base.join(APP_DIR_NAME);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn open() -> Result<Connection, Box<dyn std::error::Error>> {
    let mut path = db_dir()?;
    path.push(DB_FILE_NAME);

    let conn = Connection::open(&path)?;
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;",
    )?;

    Ok(conn)
}
