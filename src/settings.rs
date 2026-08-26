use chrono::Utc;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::AnyResult;
use crate::model::Model;
use crate::provider::Provider;
use crate::schema::setting;
use crate::schema::setting::dsl;

const CURRENT_MODEL_KEY: &str = "current_model";

/// Identifies a model.
///
/// The provider or model model may be stale! This will only cause an error
/// when submitting a prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
}

impl ModelRef {
    /// Resolves the referenced model along with its provider, returning
    /// `None` if either no longer exists.
    pub fn resolve(&self, conn: &mut SqliteConnection) -> AnyResult<Option<(Provider, Model)>> {
        let Some(provider) = Provider::get_by_name(conn, &self.provider)? else {
            return Ok(None);
        };
        let Some(model) = Model::get(conn, provider.id, &self.model)? else {
            return Ok(None);
        };
        Ok(Some((provider, model)))
    }
}

/// Persistent app settings. Loads once at start and persists all changes to DB.
pub struct Settings {
    conn: SqliteConnection,
    current_model: Option<ModelRef>,
}

impl Settings {
    /// Opens settings on the database at `db_url`, loading all values once.
    pub fn open(db_url: &str) -> AnyResult<Self> {
        let mut conn = crate::db::open(db_url)?;
        let current_model = read(&mut conn, CURRENT_MODEL_KEY)?;
        Ok(Self {
            conn,
            current_model,
        })
    }

    /// Returns the persisted model selection, if any.
    pub fn current_model(&self) -> Option<&ModelRef> {
        self.current_model.as_ref()
    }

    /// Persists the model selection and caches it. Passing `None` removes it.
    pub fn set_current_model(&mut self, value: Option<ModelRef>) -> AnyResult<()> {
        write_or_delete(&mut self.conn, CURRENT_MODEL_KEY, value.as_ref())?;
        self.current_model = value;
        Ok(())
    }
}

#[derive(Insertable)]
#[diesel(table_name = setting)]
struct NewSetting<'a> {
    key: &'a str,
    value: &'a str,
}

fn read<T>(conn: &mut SqliteConnection, key: &str) -> AnyResult<T>
where
    T: DeserializeOwned + Default,
{
    let value = dsl::setting
        .find(key)
        .select(dsl::value)
        .first::<String>(conn)
        .optional()?;
    Ok(match value {
        Some(v) => serde_json::from_str(&v).unwrap_or_default(),
        None => T::default(),
    })
}

fn write_or_delete<T: Serialize>(
    conn: &mut SqliteConnection,
    key: &str,
    value: Option<&T>,
) -> AnyResult<()> {
    match value {
        Some(value) => {
            let json = serde_json::to_string(value)?;
            diesel::insert_into(setting::table)
                .values(NewSetting { key, value: &json })
                .on_conflict(setting::key)
                .do_update()
                .set((
                    dsl::value.eq(json.as_str()),
                    dsl::updated_at.eq(Utc::now().naive_utc()),
                ))
                .execute(conn)?;
        }
        None => {
            diesel::delete(dsl::setting.find(key)).execute(conn)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::InterfaceId;

    #[test]
    fn test_settings() {
        let url = crate::db::db_url().expect("db url failed");
        let mut conn = crate::db::open(&url).expect("open failed");

        // Missing keys read as defaults.
        let mut s = Settings::open(&url).expect("open failed");
        assert_eq!(s.current_model(), None);

        // Writes are cached, written through, and survive a reopen.
        let r1 = ModelRef {
            provider: "p".into(),
            model: "m1".into(),
        };
        s.set_current_model(Some(r1.clone())).expect("set failed");
        assert_eq!(s.current_model(), Some(&r1));
        assert_eq!(
            Settings::open(&url).expect("reopen failed").current_model(),
            Some(&r1)
        );

        // Re-setting replaces the row instead of duplicating it.
        let r2 = ModelRef {
            provider: "p".into(),
            model: "m2".into(),
        };
        s.set_current_model(Some(r2.clone())).expect("set failed");
        let count = dsl::setting
            .filter(dsl::key.eq(CURRENT_MODEL_KEY))
            .count()
            .get_result::<i64>(&mut conn)
            .expect("count failed");
        assert_eq!(count, 1);
        assert_eq!(
            Settings::open(&url).expect("reopen failed").current_model(),
            Some(&r2)
        );

        // Clearing deletes the row.
        s.set_current_model(None).expect("clear failed");
        assert_eq!(s.current_model(), None);
        assert_eq!(
            Settings::open(&url).expect("reopen failed").current_model(),
            None
        );

        // Stale references resolve to None instead of failing.
        let provider =
            Provider::create(&mut conn, "p", InterfaceId::Openai, "k", None).expect("seed failed");
        Model::create(&mut conn, provider.id, "m2").expect("seed model failed");

        let found = ModelRef {
            provider: "p".into(),
            model: "m2".into(),
        }
        .resolve(&mut conn)
        .expect("resolve failed")
        .map(|(_, m)| m.id);
        assert_eq!(found, Some("m2".to_string()));

        let gone_model = ModelRef {
            provider: "p".into(),
            model: "gone".into(),
        }
        .resolve(&mut conn)
        .expect("resolve failed");
        assert!(gone_model.is_none());

        let gone_provider = ModelRef {
            provider: "gone".into(),
            model: "m2".into(),
        }
        .resolve(&mut conn)
        .expect("resolve failed");
        assert!(gone_provider.is_none());
    }
}
