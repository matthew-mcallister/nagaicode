use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::error::AnyResult;
use crate::interface::{Interface, InterfaceId};
use crate::schema::provider;
use crate::schema::provider::dsl;

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
        interface: InterfaceId,
        api_key: &str,
        base_url: Option<&str>,
    ) -> AnyResult<Provider> {
        use diesel::result::Error as DieselError;

        let new = NewProvider {
            name,
            interface: interface.as_str(),
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

    pub fn get_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<Option<Provider>> {
        let result = dsl::provider
            .filter(dsl::id.eq(id))
            .first::<Provider>(conn)
            .optional()?;
        Ok(result)
    }

    pub fn get_by_name(conn: &mut SqliteConnection, name: &str) -> AnyResult<Option<Provider>> {
        let result = dsl::provider
            .filter(dsl::name.eq(name))
            .first::<Provider>(conn)
            .optional()?;
        Ok(result)
    }

    pub fn delete_by_name(conn: &mut SqliteConnection, name: &str) -> AnyResult<bool> {
        let count = diesel::delete(dsl::provider.filter(dsl::name.eq(name))).execute(conn)?;
        Ok(count > 0)
    }

    pub fn all(conn: &mut SqliteConnection) -> AnyResult<Vec<Provider>> {
        let providers = dsl::provider
            .order(dsl::id.asc())
            .load::<Provider>(conn)?;
        Ok(providers)
    }

    pub fn create_interface(&self) -> AnyResult<Interface> {
        Interface::from_provider(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider() {
        let mut conn = crate::db::open_in_memory().expect("failed to open in-memory db");

        let created = Provider::create(&mut conn, "test", InterfaceId::Openai, "key123", None)
            .expect("create failed");
        assert_eq!(created.name, "test");

        let fetched = Provider::get_by_id(&mut conn, created.id)
            .expect("get_by_id failed")
            .expect("provider not found");
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.name, "test");
        assert_eq!(fetched.api_key, "key123");

        let deleted = Provider::delete_by_name(&mut conn, "test").expect("delete failed");
        assert!(deleted);

        let gone = Provider::get_by_id(&mut conn, created.id).expect("get_by_id failed");
        assert!(gone.is_none());

        let by_name = Provider::get_by_name(&mut conn, "test").expect("get_by_name failed");
        assert!(by_name.is_none(), "provider should be gone");

        let already_deleted = Provider::delete_by_name(&mut conn, "test").expect("delete failed");
        assert!(!already_deleted);
    }
}
