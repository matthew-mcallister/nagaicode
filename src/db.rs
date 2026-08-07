use std::fs;
use std::path::PathBuf;

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use diesel_migrations::MigrationHarness;

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

pub fn open() -> Result<SqliteConnection, Box<dyn std::error::Error>> {
    let mut path = db_dir()?;
    path.push(DB_FILE_NAME);

    let mut conn = SqliteConnection::establish(path.to_str().unwrap())?;
    conn.batch_execute(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;",
    )?;

    let migrations = diesel_migrations::FileBasedMigrations::find_migrations_directory()
        .map_err(|e| e.to_string())?;
    conn.run_pending_migrations(migrations)
        .map_err(|e| e.to_string())?;

    Ok(conn)
}
