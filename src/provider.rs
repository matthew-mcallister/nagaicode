use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::error::AnyResult;
use crate::interface::{Interface, InterfaceId};
use crate::query::{DataQuery, QueryError, QueryField, datetime_to_json};
use crate::request::DefaultClient;
use crate::schema::provider;
use crate::schema::provider::dsl;
use serde_json::json;

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

    pub fn create_interface(&self, client: &DefaultClient) -> AnyResult<Interface> {
        Interface::from_provider(self, client)
    }

pub fn base_url_normalized(&self) -> Option<&str> {
        self.base_url
            .as_ref()
            .filter(|url| !url.is_empty())
            .map(|s| s.trim_end_matches('/'))
    }
}

/// Exposed fields:
/// - id: number
/// - name: string
/// - interface: string
/// - base_url: string | null
/// - created_at: string (ISO 8601)
/// - updated_at: string (ISO 8601)
///
/// api_key is intentionally not exposed.
impl DataQuery for Provider {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "id": self.id,
                "name": self.name,
                "interface": self.interface,
                "base_url": self.base_url,
                "created_at": datetime_to_json(self.created_at),
                "updated_at": datetime_to_json(self.updated_at),
            }))),
            "id" => Ok(QueryField::Value(json!(self.id))),
            "name" => Ok(QueryField::Value(json!(self.name))),
            "interface" => Ok(QueryField::Value(json!(self.interface))),
            "base_url" => Ok(QueryField::Value(json!(self.base_url))),
            "created_at" => Ok(QueryField::Value(datetime_to_json(self.created_at))),
            "updated_at" => Ok(QueryField::Value(datetime_to_json(self.updated_at))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;

    use super::*;

    #[test]
    fn base_url_normalized_handles_normal_empty_and_trailing_slash() {
        fn make(base_url: Option<&str>) -> Provider {
            Provider {
                id: 0,
                name: "test".into(),
                interface: "openai".into(),
                api_key: "key".into(),
                base_url: base_url.map(str::to_owned),
                created_at: DateTime::from_timestamp(0, 0).unwrap().naive_utc(),
                updated_at: DateTime::from_timestamp(0, 0).unwrap().naive_utc(),
            }
        }

        assert_eq!(make(Some("https://example.test/v1")).base_url_normalized(), Some("https://example.test/v1"));
        assert_eq!(make(Some("")).base_url_normalized(), None);
        assert_eq!(make(Some("https://example.test/v1/")).base_url_normalized(), Some("https://example.test/v1"));
        assert_eq!(make(None).base_url_normalized(), None);
    }

    #[test]
    fn test_provider() {
        let mut conn = crate::db::open_new().expect("failed to open in-memory db");

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

    #[test]
    fn test_provider_query() {
        use serde_json::json;

        let mut conn = crate::db::open_new().expect("failed to open in-memory db");
        let provider = Provider::create(
            &mut conn,
            "test",
            InterfaceId::Openai,
            "key123",
            Some("https://example.test/v1"),
        )
        .expect("create failed");

        assert_eq!(provider.query("/").unwrap(), json!({
            "id": provider.id,
            "name": provider.name,
            "interface": provider.interface,
            "base_url": provider.base_url,
            "created_at": datetime_to_json(provider.created_at),
            "updated_at": datetime_to_json(provider.updated_at),
        }));
        assert_eq!(provider.query("/id").unwrap(), json!(provider.id));
        assert_eq!(provider.query("/name").unwrap(), json!(provider.name));
        assert_eq!(provider.query("/interface").unwrap(), json!(provider.interface));
        assert_eq!(provider.query("/base_url").unwrap(), json!(provider.base_url));
        assert_eq!(provider.query("/created_at").unwrap(), datetime_to_json(provider.created_at));
        assert_eq!(provider.query("/updated_at").unwrap(), datetime_to_json(provider.updated_at));
    }
}
