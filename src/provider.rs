use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::error::AnyResult;
use crate::schema::provider;

#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = provider)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Provider {
    pub id: i32,
    pub name: String,
    pub interface: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = provider)]
pub struct NewProvider<'a> {
    pub name: &'a str,
    pub interface: &'a str,
    pub api_key: &'a str,
    pub base_url: Option<&'a str>,
}

impl Provider {
    pub fn create(
        conn: &mut SqliteConnection,
        name: &str,
        interface: &str,
        api_key: &str,
        base_url: Option<&str>,
    ) -> AnyResult<Provider> {
        use diesel::result::Error as DieselError;

        let new = NewProvider {
            name,
            interface,
            api_key,
            base_url,
        };

        match diesel::insert_into(provider::table)
            .values(&new)
            .returning(provider::all_columns)
            .get_result(conn)
        {
            Ok(p) => Ok(p),
            Err(DieselError::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                _,
            )) => Err(format!("a provider named '{name}' already exists").into()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn by_name(conn: &mut SqliteConnection, name: &str) -> AnyResult<Provider> {
        use provider::dsl;

        let p = dsl::provider
            .filter(dsl::name.eq(name))
            .first(conn)?;
        Ok(p)
    }

    pub fn all(conn: &mut SqliteConnection) -> AnyResult<Vec<Provider>> {
        use provider::dsl;

        let rows = dsl::provider
            .order_by(dsl::name)
            .load::<Provider>(conn)?;
        Ok(rows)
    }
}
