// TODO: Track recently used models (requires upsert but can do client-side)
use anyhow::anyhow;
use chrono::{Duration, NaiveDateTime, Utc};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use futures::future::join_all;

use crate::error::{AnyError, AnyResult};
use crate::interface::InterfaceModel;
use crate::provider::Provider;
use crate::query::{DataQuery, QueryError, QueryField, ToJson};
use crate::request::DefaultClient;
use crate::schema::model;
use crate::schema::model::dsl;
use crate::tasks::{Task, TaskContext};
use serde_json::json;

/// Model from a provider. We fetch and cache these periodically.
///
/// primary key: (provider_id, id)
#[derive(Debug, Clone, Queryable, Selectable)]
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
            )) => Err(anyhow!(
                "a model with id '{id}' already exists for provider {provider_id}"
            )),
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

    pub fn delete_all(conn: &mut SqliteConnection) -> AnyResult<usize> {
        let count = diesel::delete(dsl::model).execute(conn)?;
        Ok(count)
    }

    pub fn create_all(
        conn: &mut SqliteConnection,
        new_models: Vec<NewModel>,
    ) -> AnyResult<()> {
        if new_models.is_empty() {
            return Ok(());
        }
        diesel::insert_into(model::table)
            .values(&new_models)
            .execute(conn)?;
        Ok(())
    }
}

impl DataQuery for Model {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "provider_id": self.query("/provider_id")?,
                "id": self.query("/id")?,
                "created_at": self.query("/created_at")?,
                "updated_at": self.query("/updated_at")?,
            }))),
            "provider_id" => Ok(QueryField::Value(json!(self.provider_id))),
            "id" => Ok(QueryField::Value(json!(self.id))),
            "created_at" => Ok(QueryField::Value(self.created_at.to_json())),
            "updated_at" => Ok(QueryField::Value(self.updated_at.to_json())),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

pub async fn revalidate_models(conn: &mut SqliteConnection) -> AnyResult<()> {
    // XXX: This function is racey (e.g. provider is deleted while models are
    // fetching) but risk is pretty low
    let cutoff = Utc::now().naive_utc() - Duration::hours(24);

    let stale_exists = dsl::model
        .filter(dsl::updated_at.lt(cutoff))
        .first::<Model>(conn)
        .optional()?
        .is_some();
    let is_empty: bool = dsl::model.count().get_result::<i64>(conn)? == 0;

    if !is_empty && !stale_exists {
        return Ok(());
    }

    let providers = Provider::all(conn)?;
    let client = DefaultClient::default();
    let interfaces = providers.iter()
        .map(|p| p.create_interface(&client))
        .collect::<AnyResult<Vec<_>>>()?;
    let fetches: Vec<_> = interfaces.iter().map(|i| i.get_models()).collect();
    let results: Vec<AnyResult<Vec<InterfaceModel>>> = join_all(fetches).await;

    let mut new_models = Vec::new();
    for (provider, result) in providers.iter().zip(results) {
        let models = result?;
        for m in models {
            new_models.push(NewModel {
                provider_id: provider.id,
                id: m.id,
            });
        }
    }

    conn.transaction::<_, AnyError, _>(|conn| {
        Model::delete_all(conn)?;
        Model::create_all(conn, new_models)?;
        Ok(())
    })?;

    Ok(())
}

/// Background task that refreshes cached provider model lists.
pub struct RevalidateModelsTask;

impl RevalidateModelsTask {
    async fn process(self, context: &mut TaskContext) -> AnyResult<()> {
        revalidate_models(context.connection()?).await
    }
}

impl Task for RevalidateModelsTask {
    type Output = ();

    async fn run(self, context: &mut TaskContext) {
        if let Err(e) = self.process(context).await {
            log::error!("failed to revalidate models: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::InterfaceId;
    use crate::provider::Provider;

    fn seed_provider(conn: &mut SqliteConnection) -> Provider {
        Provider::create(conn, "test", InterfaceId::Openai, "key123", None).expect("create provider failed")
    }

    #[test]
    fn test_model() {
        let mut conn = crate::db::open_new().expect("failed to open in-memory db");
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
        let mut conn = crate::db::open_new().expect("failed to open in-memory db");
        let p1 = Provider::create(&mut conn, "p1", InterfaceId::Openai, "k1", None).expect("create p1");
        let p2 = Provider::create(&mut conn, "p2", InterfaceId::Openai, "k2", None).expect("create p2");

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
        let mut conn = crate::db::open_new().expect("failed to open in-memory db");
        let provider = seed_provider(&mut conn);
        Model::create(&mut conn, provider.id, "gpt-4").expect("create model");

        Provider::delete_by_name(&mut conn, "test").expect("delete provider");
        let models = Model::list_by_provider(&mut conn, provider.id).expect("list models");
        assert!(
            models.is_empty(),
            "models should be cascade-deleted with their provider"
        );
    }

    #[test]
    fn test_model_query() {
        use serde_json::json;

        let mut conn = crate::db::open_new().expect("failed to open in-memory db");
        let provider = seed_provider(&mut conn);
        let model = Model::create(&mut conn, provider.id, "gpt-4").expect("create failed");

        assert_eq!(model.query("/").unwrap(), json!({
            "provider_id": model.provider_id,
            "id": model.id,
            "created_at": model.created_at.to_json(),
            "updated_at": model.updated_at.to_json(),
        }));
        assert_eq!(model.query("/provider_id").unwrap(), json!(model.provider_id));
        assert_eq!(model.query("/id").unwrap(), json!(model.id));
        assert_eq!(model.query("/created_at").unwrap(), model.created_at.to_json());
        assert_eq!(model.query("/updated_at").unwrap(), model.updated_at.to_json());
    }
}
