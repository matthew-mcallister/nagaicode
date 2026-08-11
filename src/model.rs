use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::error::AnyResult;
use crate::schema::model;
use crate::schema::model::dsl;

/// Model from a provider. We fetch and cache these periodically.
///
/// primary key: (provider_id, id)
#[derive(Debug, Queryable, Selectable)]
#[diesel(table_name = model)]
#[diesel(primary_key(provider_id, id))]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Model {
    pub provider_id: i32,
    pub id: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = model)]
#[diesel(primary_key(provider_id, id))]
pub struct NewModel {
    pub provider_id: i32,
    pub id: String,
}

impl Model {
    pub fn create(conn: &mut SqliteConnection, provider_id: i32, id: &str) -> AnyResult<Model> {
        use diesel::result::Error as DieselError;

        let new = NewModel {
            provider_id,
            id: id.to_string(),
        };

        match diesel::insert_into(model::table)
            .values(&new)
            .returning(model::all_columns)
            .get_result(conn)
        {
            Ok(m) => Ok(m),
            Err(DieselError::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                _,
            )) => Err(format!(
                "a model with id '{id}' already exists for provider {provider_id}"
            )
            .into()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get(conn: &mut SqliteConnection, provider_id: i32, id: &str) -> AnyResult<Option<Model>> {
        let result = dsl::model
            .filter(dsl::provider_id.eq(provider_id))
            .filter(dsl::id.eq(id))
            .first::<Model>(conn)
            .optional()?;
        Ok(result)
    }

    pub fn list_by_provider(
        conn: &mut SqliteConnection,
        provider_id: i32,
    ) -> AnyResult<Vec<Model>> {
        let results = dsl::model
            .filter(dsl::provider_id.eq(provider_id))
            .order(dsl::id.asc())
            .load::<Model>(conn)?;
        Ok(results)
    }

    pub fn delete(
        conn: &mut SqliteConnection,
        provider_id: i32,
        id: &str,
    ) -> AnyResult<bool> {
        let count = diesel::delete(
            dsl::model
                .filter(dsl::provider_id.eq(provider_id))
                .filter(dsl::id.eq(id)),
        )
        .execute(conn)?;
        Ok(count > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Provider;

    fn seed_provider(conn: &mut SqliteConnection) -> Provider {
        Provider::create(conn, "test", "openai", "key123", None).expect("create provider failed")
    }

    #[test]
    fn test_model() {
        let mut conn = crate::db::open_in_memory().expect("failed to open in-memory db");
        let provider = seed_provider(&mut conn);

        let created =
            Model::create(&mut conn, provider.id, "gpt-4").expect("create failed");
        assert_eq!(created.id, "gpt-4");
        assert_eq!(created.provider_id, provider.id);

        let fetched = Model::get(&mut conn, provider.id, "gpt-4")
            .expect("get failed")
            .expect("model not found");
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.provider_id, provider.id);

        let listed = Model::list_by_provider(&mut conn, provider.id).expect("list failed");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "gpt-4");

        let dup = Model::create(&mut conn, provider.id, "gpt-4");
        assert!(dup.is_err());

        let deleted = Model::delete(&mut conn, provider.id, "gpt-4").expect("delete failed");
        assert!(deleted);

        let gone = Model::get(&mut conn, provider.id, "gpt-4").expect("get failed");
        assert!(gone.is_none());

        let already_deleted =
            Model::delete(&mut conn, provider.id, "gpt-4").expect("delete failed");
        assert!(!already_deleted);
    }

    #[test]
    fn test_model_isolation_by_provider() {
        let mut conn = crate::db::open_in_memory().expect("failed to open in-memory db");
        let p1 = Provider::create(&mut conn, "p1", "openai", "k1", None).expect("create p1");
        let p2 = Provider::create(&mut conn, "p2", "openai", "k2", None).expect("create p2");

        // The same model id can exist for two different providers.
        Model::create(&mut conn, p1.id, "gpt-4").expect("create p1 model");
        Model::create(&mut conn, p2.id, "gpt-4").expect("create p2 model");

        let p1_models = Model::list_by_provider(&mut conn, p1.id).expect("list p1");
        let p2_models = Model::list_by_provider(&mut conn, p2.id).expect("list p2");
        assert_eq!(p1_models.len(), 1);
        assert_eq!(p2_models.len(), 1);
    }

    #[test]
    fn test_model_cascade_delete_with_provider() {
        let mut conn = crate::db::open_in_memory().expect("failed to open in-memory db");
        let provider = seed_provider(&mut conn);
        Model::create(&mut conn, provider.id, "gpt-4").expect("create model");

        Provider::delete_by_name(&mut conn, "test").expect("delete provider");
        let models = Model::list_by_provider(&mut conn, provider.id).expect("list models");
        assert!(
            models.is_empty(),
            "models should be cascade-deleted with their provider"
        );
    }
}
