use std::cell::Cell;
use std::fs;
use std::path::PathBuf;

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use diesel_migrations::MigrationHarness;

use crate::error::AnyResult;

thread_local! {
    static DB_NUM: Cell<u64> = const { Cell::new(0) };
}

fn run_migrations(conn: &mut SqliteConnection) -> AnyResult<()> {
    let migrations = diesel_migrations::FileBasedMigrations::find_migrations_directory()
        .map_err(|e| e.to_string())?;
    conn.run_pending_migrations(migrations)
        .map_err(|e| e.to_string())?;
    Ok(())
}

const APP_DIR_NAME: &str = "nagaicode";
const DB_FILE_NAME: &str = "db.sqlite";

fn db_dir() -> AnyResult<PathBuf> {
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

fn open_disk() -> AnyResult<SqliteConnection> {
    let mut path = db_dir()?;
    path.push(DB_FILE_NAME);

    let mut conn = SqliteConnection::establish(path.to_str().unwrap())?;
    conn.batch_execute(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;",
    )?;

    run_migrations(&mut conn)?;

    Ok(conn)
}

/// Opens an in-memory database connection.
pub fn open_in_memory() -> AnyResult<SqliteConnection> {
    let db_name = format!("{:?}/{}", std::thread::current().id(), DB_NUM.get());
    let uri = format!("file:{db_name}?mode=memory&cache=shared");
    let mut conn = SqliteConnection::establish(&uri)?;
    conn.batch_execute("PRAGMA foreign_keys = ON;")?;
    run_migrations(&mut conn)?;
    Ok(conn)
}

/// Resets the in-memory database counter for the current thread.
pub fn reset() {
    DB_NUM.set(DB_NUM.get() + 1);
}

/// Opens a new in-memory database connection.
pub fn open_new() -> AnyResult<SqliteConnection> {
    reset();
    open_in_memory()
}

/// Opens a database connection.
pub fn open() -> AnyResult<SqliteConnection> {
    if cfg!(test) {
        open_in_memory()
    } else {
        open_disk()
    }
}
