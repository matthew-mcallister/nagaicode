// XXX: At some point should set up separate database for cache
use diesel::sqlite::SqliteConnection;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};

use crate::error::AnyResult;

#[cfg(not(test))]
pub(crate) use self::real::{db_url, open};
#[cfg(test)]
pub(crate) use self::mock::{db_url, memory_url, open, open_new};

pub const MIGRATIONS: EmbeddedMigrations = diesel_migrations::embed_migrations!("migrations");

fn run_migrations(conn: &mut SqliteConnection) -> AnyResult<()> {
    conn.run_pending_migrations(MIGRATIONS)
        .map_err(anyhow::Error::from_boxed)?;
    Ok(())
}

#[cfg(not(test))]
mod real {
    use diesel::connection::SimpleConnection;
    use diesel::{Connection, SqliteConnection};

    use crate::config::Dirs;
    use crate::error::AnyResult;
    use super::run_migrations;

    const DB_FILE_NAME: &str = "db.sqlite";

    pub(crate) fn db_url(dirs: &Dirs) -> AnyResult<String> {
        let mut path = dirs.data().to_path_buf();
        path.push(DB_FILE_NAME);
        Ok(path.to_str().unwrap().to_string())
    }

    pub(crate) fn open(url: &str) -> AnyResult<SqliteConnection> {
        let mut conn = SqliteConnection::establish(url)?;
        conn.batch_execute(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;",
        )?;

        run_migrations(&mut conn)?;

        Ok(conn)
    }
}

#[cfg(test)]
mod mock {
    use diesel::connection::SimpleConnection;
    use diesel::{Connection, SqliteConnection};

    use crate::config::Dirs;
    use crate::error::AnyResult;
    use super::run_migrations;

    /// Returns the URL of the app database, which is in-memory in tests.
    pub(crate) fn db_url(_dirs: &Dirs) -> AnyResult<String> {
        Ok(memory_url())
    }

    /// Opens an in-memory database connection.
    pub(crate) fn open(url: &str) -> AnyResult<SqliteConnection> {
        let mut conn = SqliteConnection::establish(url)?;
        conn.batch_execute("PRAGMA foreign_keys = ON;")?;
        run_migrations(&mut conn)?;
        Ok(conn)
    }

    /// Opens a new in-memory database connection.
    pub fn open_new() -> AnyResult<SqliteConnection> {
        let url = memory_url();
        open(&url)
    }

    /// Returns a URL for a fresh in-memory database.
    pub(crate) fn memory_url() -> String {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_name = format!("{:?}:{}", std::thread::current().id(), timestamp);
        format!("file:{db_name}?mode=memory&cache=shared")
    }
}
