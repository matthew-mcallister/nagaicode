use anyhow::anyhow;
use chrono::NaiveDateTime;
use diesel::backend::Backend;
use diesel::deserialize::{self, FromSql};
use diesel::prelude::*;
use diesel::serialize::{self, IsNull, ToSql};
use diesel::sql_types::Text;
use diesel::sqlite::Sqlite;
use diesel::sqlite::SqliteConnection;
use serde_json::{Value, json};
use std::fmt;
use std::str::FromStr;

use crate::error::{AnyError, AnyResult};
use crate::interface::Usage;
use crate::query::{DataQuery, QueryError, QueryField, ToJson};
use crate::schema::{response, session, turn};

#[derive(Debug, Clone, Copy, Eq, PartialEq, diesel::AsExpression, diesel::FromSqlRow)]
#[diesel(sql_type = Text)]
pub enum TurnType {
    User,
    Assistant,
}

impl fmt::Display for TurnType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TurnType::User => write!(f, "user"),
            TurnType::Assistant => write!(f, "assistant"),
        }
    }
}

impl FromStr for TurnType {
    type Err = AnyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(TurnType::User),
            "assistant" => Ok(TurnType::Assistant),
            other => Err(anyhow!("unknown turn type: {other}")),
        }
    }
}

impl ToSql<Text, Sqlite> for TurnType {
    fn to_sql<'b>(&'b self, out: &mut serialize::Output<'b, '_, Sqlite>) -> serialize::Result {
        out.set_value(self.to_string());
        Ok(IsNull::No)
    }
}

impl FromSql<Text, Sqlite> for TurnType {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> deserialize::Result<Self> {
        let value = <String as FromSql<Text, Sqlite>>::from_sql(bytes)?;
        value.parse().map_err(Into::into)
    }
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = session)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Session {
    pub id: i32,
    pub name: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = session)]
pub struct NewSession<'a> {
    pub name: &'a str,
}

impl Session {
    pub fn create(conn: &mut SqliteConnection, name: &str) -> AnyResult<Session> {
        let new = NewSession { name };
        let session = diesel::insert_into(session::table)
            .values(&new)
            .returning(session::all_columns)
            .get_result(conn)?;
        Ok(session)
    }

    pub fn get_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<Option<Session>> {
        let result = session::table
            .filter(session::id.eq(id))
            .first::<Session>(conn)
            .optional()?;
        Ok(result)
    }

    pub fn all(conn: &mut SqliteConnection) -> AnyResult<Vec<Session>> {
        let sessions = session::table
            .order(session::id.asc())
            .load::<Session>(conn)?;
        Ok(sessions)
    }

    pub fn delete_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<bool> {
        let count = diesel::delete(session::table.filter(session::id.eq(id))).execute(conn)?;
        Ok(count > 0)
    }
}

impl DataQuery for Session {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "id": self.query("/id")?,
                "name": self.query("/name")?,
                "created_at": self.query("/created_at")?,
                "updated_at": self.query("/updated_at")?,
            }))),
            "id" => Ok(QueryField::Value(json!(self.id))),
            "name" => Ok(QueryField::Value(json!(self.name))),
            "created_at" => Ok(QueryField::Value(self.created_at.to_json())),
            "updated_at" => Ok(QueryField::Value(self.updated_at.to_json())),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

/// A container for items. In a user-guided conversation, turn alternates
/// between user and assistant. In a subagent session (roadmap), there is only
/// one turn.
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = turn)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Turn {
    pub id: i32,
    /// Maps to TurnType
    pub ty: String,
    pub session_id: i32,
    /// Agent turn fields
    pub provider_id: Option<i32>,
    pub provider_name: Option<String>,
    pub model_id: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = turn)]
pub struct NewTurn<'a> {
    pub session_id: i32,
    pub ty: TurnType,
    pub provider_id: Option<i32>,
    pub provider_name: Option<&'a str>,
    pub model_id: Option<&'a str>,
}

impl Turn {
    pub fn create(
        conn: &mut SqliteConnection,
        session_id: i32,
        ty: TurnType,
        provider_id: Option<i32>,
        provider_name: Option<&str>,
        model_id: Option<&str>,
    ) -> AnyResult<Turn> {
        let new = NewTurn {
            session_id,
            ty,
            provider_id,
            provider_name,
            model_id,
        };
        let turn = diesel::insert_into(turn::table)
            .values(new)
            .returning(turn::all_columns)
            .get_result(conn)?;
        Ok(turn)
    }

    pub fn get_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<Option<Turn>> {
        let result = turn::table
            .filter(turn::id.eq(id))
            .first::<Turn>(conn)
            .optional()?;
        Ok(result)
    }

    pub fn list_by_session(conn: &mut SqliteConnection, session_id: i32) -> AnyResult<Vec<Turn>> {
        let turns = turn::table
            .filter(turn::session_id.eq(session_id))
            .order(turn::id.asc())
            .load::<Turn>(conn)?;
        Ok(turns)
    }

    pub fn delete_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<bool> {
        let count = diesel::delete(turn::table.filter(turn::id.eq(id))).execute(conn)?;
        Ok(count > 0)
    }

    pub fn ty(&self) -> AnyResult<TurnType> {
        TurnType::from_str(&self.ty)
    }
}

impl DataQuery for Turn {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "id": self.query("/id")?,
                "ty": self.query("/ty")?,
                "session_id": self.query("/session_id")?,
                "provider_id": self.query("/provider_id")?,
                "provider_name": self.query("/provider_name")?,
                "model_id": self.query("/model_id")?,
                "created_at": self.query("/created_at")?,
                "updated_at": self.query("/updated_at")?,
            }))),
            "id" => Ok(QueryField::Value(json!(self.id))),
            "ty" => Ok(QueryField::Value(json!(self.ty))),
            "session_id" => Ok(QueryField::Value(json!(self.session_id))),
            "provider_id" => Ok(QueryField::Value(json!(self.provider_id))),
            "provider_name" => Ok(QueryField::Value(json!(self.provider_name))),
            "model_id" => Ok(QueryField::Value(json!(self.model_id))),
            "created_at" => Ok(QueryField::Value(self.created_at.to_json())),
            "updated_at" => Ok(QueryField::Value(self.updated_at.to_json())),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

/// A response from the upstream API. Contains text, reasoning, and tool call
/// outputs.
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = response)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Response {
    pub id: i32,
    pub session_id: i32,
    pub turn_id: i32,
    pub upstream_id: Option<String>,
    pub upstream_status: Option<String>,
    // Usage data
    pub input_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    // Raw event data
    // TODO: There needs to be a config item to disable storing this
    pub raw_request: Option<String>,
    pub raw_response: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = response)]
pub struct NewResponse<'a> {
    pub session_id: i32,
    pub turn_id: i32,
    pub upstream_id: Option<&'a str>,
    pub upstream_status: Option<&'a str>,
}

impl Response {
    pub fn create(
        conn: &mut SqliteConnection,
        session_id: i32,
        turn_id: i32,
        upstream_id: Option<&str>,
        upstream_status: Option<&str>,
    ) -> AnyResult<Response> {
        let new = NewResponse {
            session_id,
            turn_id,
            upstream_id,
            upstream_status,
        };
        let response = diesel::insert_into(response::table)
            .values(new)
            .returning(response::all_columns)
            .get_result(conn)?;
        Ok(response)
    }

    pub fn get_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<Option<Response>> {
        let result = response::table
            .filter(response::id.eq(id))
            .first::<Response>(conn)
            .optional()?;
        Ok(result)
    }

    pub fn list_by_turn(conn: &mut SqliteConnection, turn_id: i32) -> AnyResult<Vec<Response>> {
        let responses = response::table
            .filter(response::turn_id.eq(turn_id))
            .order(response::id.asc())
            .load::<Response>(conn)?;
        Ok(responses)
    }

    pub fn delete_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<bool> {
        let count = diesel::delete(response::table.filter(response::id.eq(id))).execute(conn)?;
        Ok(count > 0)
    }

    /// Update the response after receiving a 'completed' or 'failed' event
    pub fn finish(
        conn: &mut SqliteConnection,
        id: i32,
        upstream_status: &str,
        usage: Option<&Usage>,
        raw_response: Option<&Value>,
    ) -> AnyResult<()> {
        use crate::schema::response::dsl;
        let (input_tokens, cached_input_tokens, output_tokens, reasoning_tokens, total_tokens) =
            match usage {
                Some(u) => (
                    Some(u.input_tokens as i64),
                    Some(u.cached_input_tokens as i64),
                    Some(u.output_tokens as i64),
                    Some(u.reasoning_tokens as i64),
                    Some(u.total_tokens as i64),
                ),
                None => (None, None, None, None, None),
            };
        diesel::update(dsl::response.filter(dsl::id.eq(id)))
            .set((
                dsl::upstream_status.eq(upstream_status),
                dsl::input_tokens.eq(input_tokens),
                dsl::cached_input_tokens.eq(cached_input_tokens),
                dsl::output_tokens.eq(output_tokens),
                dsl::reasoning_tokens.eq(reasoning_tokens),
                dsl::total_tokens.eq(total_tokens),
                dsl::raw_response.eq(raw_response.map(|v| v.to_string())),
            ))
            .execute(conn)?;
        Ok(())
    }
}

impl DataQuery for Response {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "id": self.query("/id")?,
                "session_id": self.query("/session_id")?,
                "turn_id": self.query("/turn_id")?,
                "upstream_id": self.query("/upstream_id")?,
                "upstream_status": self.query("/upstream_status")?,
                "input_tokens": self.query("/input_tokens")?,
                "cached_input_tokens": self.query("/cached_input_tokens")?,
                "output_tokens": self.query("/output_tokens")?,
                "reasoning_tokens": self.query("/reasoning_tokens")?,
                "total_tokens": self.query("/total_tokens")?,
                "raw_request": self.query("/raw_request")?,
                "raw_response": self.query("/raw_response")?,
                "created_at": self.query("/created_at")?,
                "updated_at": self.query("/updated_at")?,
            }))),
            "id" => Ok(QueryField::Value(json!(self.id))),
            "session_id" => Ok(QueryField::Value(json!(self.session_id))),
            "turn_id" => Ok(QueryField::Value(json!(self.turn_id))),
            "upstream_id" => Ok(QueryField::Value(json!(self.upstream_id))),
            "upstream_status" => Ok(QueryField::Value(json!(self.upstream_status))),
            "input_tokens" => Ok(QueryField::Value(json!(self.input_tokens))),
            "cached_input_tokens" => Ok(QueryField::Value(json!(self.cached_input_tokens))),
            "output_tokens" => Ok(QueryField::Value(json!(self.output_tokens))),
            "reasoning_tokens" => Ok(QueryField::Value(json!(self.reasoning_tokens))),
            "total_tokens" => Ok(QueryField::Value(json!(self.total_tokens))),
            "raw_request" => Ok(QueryField::Value(json!(self.raw_request))),
            "raw_response" => Ok(QueryField::Value(json!(self.raw_response))),
            "created_at" => Ok(QueryField::Value(self.created_at.to_json())),
            "updated_at" => Ok(QueryField::Value(self.updated_at.to_json())),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_data_query() {
        use serde_json::json;

        let mut conn = crate::db::open_new().unwrap();
        let session = Session::create(&mut conn, "Session 1").unwrap();
        assert_eq!(
            session.query("/").unwrap(),
            json!({
                "id": session.id,
                "name": session.name,
                "created_at": session.created_at.to_json(),
                "updated_at": session.updated_at.to_json(),
            })
        );

        let turn = Turn::create(
            &mut conn,
            session.id,
            TurnType::Assistant,
            Some(1),
            Some("openai"),
            Some("gpt-4o"),
        ).unwrap();
        assert_eq!(
            turn.query("/").unwrap(),
            json!({
                "id": turn.id,
                "ty": turn.ty,
                "session_id": turn.session_id,
                "provider_id": turn.provider_id,
                "provider_name": turn.provider_name,
                "model_id": turn.model_id,
                "created_at": turn.created_at.to_json(),
                "updated_at": turn.updated_at.to_json(),
            })
        );
        assert_eq!(turn.query("/ty").unwrap(), json!(turn.ty));
        assert_eq!(
            turn.query("/provider_name").unwrap(),
            json!(turn.provider_name)
        );
        assert_eq!(turn.query("/model_id").unwrap(), json!(turn.model_id));

        let response = Response::create(
            &mut conn,
            session.id,
            turn.id,
            Some("resp-1"),
            Some("in_progress"),
        ).unwrap();
        let usage = Usage {
            input_tokens: 12,
            cached_input_tokens: 4,
            output_tokens: 18,
            reasoning_tokens: 7,
            total_tokens: 30,
        };
        let raw_response = json!({"id": "resp-1", "status": "completed"});
        Response::finish(
            &mut conn,
            response.id,
            "completed",
            Some(&usage),
            Some(&raw_response),
        ).unwrap();
        let response = Response::get_by_id(&mut conn, response.id).unwrap().unwrap();
        assert_eq!(
            response.query("/").unwrap(),
            json!({
                "id": response.id,
                "session_id": response.session_id,
                "turn_id": response.turn_id,
                "upstream_id": response.upstream_id,
                "upstream_status": response.upstream_status,
                "input_tokens": response.input_tokens,
                "cached_input_tokens": response.cached_input_tokens,
                "output_tokens": response.output_tokens,
                "reasoning_tokens": response.reasoning_tokens,
                "total_tokens": response.total_tokens,
                "raw_request": response.raw_request,
                "raw_response": response.raw_response,
                "created_at": response.created_at.to_json(),
                "updated_at": response.updated_at.to_json(),
            })
        );
        assert_eq!(
            response.query("/upstream_id").unwrap(),
            json!(response.upstream_id)
        );
        assert_eq!(
            response.query("/upstream_status").unwrap(),
            json!(response.upstream_status)
        );
        assert_eq!(
            response.query("/total_tokens").unwrap(),
            json!(response.total_tokens)
        );
        assert_eq!(
            response.query("/raw_response").unwrap(),
            json!(response.raw_response)
        );
    }
}
