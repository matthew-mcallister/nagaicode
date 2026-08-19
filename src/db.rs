use diesel::SqliteConnection;
use diesel_migrations::MigrationHarness;

use crate::error::AnyResult;

pub use detail::open;

#[cfg(test)]
pub use detail::{open_new, reset};

fn run_migrations(conn: &mut SqliteConnection) -> AnyResult<()> {
    let migrations = diesel_migrations::FileBasedMigrations::find_migrations_directory()
        .map_err(|e| e.to_string())?;
    conn.run_pending_migrations(migrations)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(not(test))]
mod detail {
    use std::fs;
    use std::path::PathBuf;

    use diesel::connection::SimpleConnection;
    use diesel::prelude::*;
    use diesel::sqlite::SqliteConnection;

    use crate::error::AnyResult;

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

    #[cfg(not(test))]
    pub fn open() -> AnyResult<SqliteConnection> {
        let mut path = db_dir()?;
        path.push(DB_FILE_NAME);

        let mut conn = SqliteConnection::establish(path.to_str().unwrap())?;
        conn.batch_execute(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;",
        )?;

        super::run_migrations(&mut conn)?;

        Ok(conn)
    }
}

#[cfg(test)]
mod detail {
    use std::cell::Cell;

    use diesel::connection::SimpleConnection;
    use diesel::prelude::*;
    use diesel::sqlite::SqliteConnection;

    use crate::error::AnyResult;

    thread_local! {
        pub static DB_NUM: Cell<u64> = const { Cell::new(0) };
    }

    pub fn reset() {
        DB_NUM.set(DB_NUM.get() + 1);
    }

    pub fn open() -> AnyResult<SqliteConnection> {
        let db_name = format!("{:?}/{}", std::thread::current().id(), DB_NUM.get());
        let uri = format!("file:{db_name}?mode=memory&cache=shared");
        let mut conn = SqliteConnection::establish(&uri)?;
        conn.batch_execute("PRAGMA foreign_keys = ON;")?;
        super::run_migrations(&mut conn)?;
        Ok(conn)
    }

    pub fn open_new() -> AnyResult<SqliteConnection> {
        reset();
        open()
    }
}
