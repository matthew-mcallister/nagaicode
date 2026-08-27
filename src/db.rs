use diesel::sqlite::SqliteConnection;
use diesel_migrations::MigrationHarness;

use crate::error::AnyResult;

#[cfg(not(test))]
pub use self::real::{db_url, open};
#[cfg(test)]
pub use self::mock::{db_url, open, open_new};

fn run_migrations(conn: &mut SqliteConnection) -> AnyResult<()> {
    let migrations = diesel_migrations::FileBasedMigrations::find_migrations_directory()?;
    conn.run_pending_migrations(migrations)
        .map_err(anyhow::Error::from_boxed)?;
    Ok(())
}

#[cfg(not(test))]
mod real {
    use diesel::connection::SimpleConnection;
    use diesel::{Connection, SqliteConnection};

    use crate::config::data_dir;
    use crate::error::AnyResult;
    use super::run_migrations;

    const DB_FILE_NAME: &str = "db.sqlite";

    pub fn db_url() -> AnyResult<String> {
        let mut path = data_dir()?;
        path.push(DB_FILE_NAME);
        Ok(path.to_str().unwrap().to_string())
    }

    pub fn open(url: &str) -> AnyResult<SqliteConnection> {
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

    use crate::error::AnyResult;
    use super::run_migrations;

    /// Returns a URL for a fresh in-memory database.
    pub fn db_url() -> AnyResult<String> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_name = format!("{:?}:{}", std::thread::current().id(), timestamp);
        Ok(format!("file:{db_name}?mode=memory&cache=shared"))
    }

    /// Opens an in-memory database connection.
    pub fn open(url: &str) -> AnyResult<SqliteConnection> {
        let mut conn = SqliteConnection::establish(url)?;
        conn.batch_execute("PRAGMA foreign_keys = ON;")?;
        run_migrations(&mut conn)?;
        Ok(conn)
    }

    /// Opens a new in-memory database connection.
    pub fn open_new() -> AnyResult<SqliteConnection> {
        let url = db_url()?;
        open(&url)
    }
}
