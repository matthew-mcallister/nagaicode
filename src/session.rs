use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use serde_json::{Value, json};
use std::fmt;
use std::str::FromStr;

use crate::error::AnyResult;
use crate::interface::Usage;
use crate::query::{DataQuery, QueryError, QueryField, ToJson};
use crate::schema::{item, response, session, turn};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
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
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(TurnType::User),
            "assistant" => Ok(TurnType::Assistant),
            other => Err(format!("unknown turn type: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ItemType {
    UserText,
    ResponseText,
    Reasoning,
}

impl fmt::Display for ItemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ItemType::UserText => write!(f, "user_text"),
            ItemType::ResponseText => write!(f, "response_text"),
            ItemType::Reasoning => write!(f, "reasoning"),
        }
    }
}

impl FromStr for ItemType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user_text" => Ok(ItemType::UserText),
            "response_text" => Ok(ItemType::ResponseText),
            "reasoning" => Ok(ItemType::Reasoning),
            other => Err(format!("unknown item type: {other}")),
        }
    }
}

impl ItemType {
    pub fn from_upstream(ty: &str) -> Option<Self> {
        match ty {
            "message" => Some(ItemType::ResponseText),
            "reasoning" => Some(ItemType::Reasoning),
            _ => None,
        }
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

/// Exposed fields:
/// - id: number
/// - name: string
/// - created_at: string (ISO 8601)
/// - updated_at: string (ISO 8601)
impl DataQuery for Session {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "id": self.id,
                "name": self.name,
                "created_at": self.created_at.to_json(),
                "updated_at": self.updated_at.to_json(),
            }))),
            "id" => Ok(QueryField::Value(json!(self.id))),
            "name" => Ok(QueryField::Value(json!(self.name))),
            "created_at" => Ok(QueryField::Value(self.created_at.to_json())),
            "updated_at" => Ok(QueryField::Value(self.updated_at.to_json())),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = turn)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Turn {
    pub id: i32,
    #[diesel(column_name = "type_")]
    pub ty: String,
    pub session_id: i32,
    pub provider_id: Option<i32>,
    pub provider_name: Option<String>,
    pub model_id: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug)]
pub struct NewTurn<'a> {
    pub session_id: i32,
    pub ty: TurnType,
    pub provider_id: Option<i32>,
    pub provider_name: Option<&'a str>,
    pub model_id: Option<&'a str>,
}

impl<'a> diesel::insertable::Insertable<turn::table> for NewTurn<'a> {
    type Values = <(
        Option<diesel::dsl::Eq<turn::session_id, i32>>,
        Option<diesel::dsl::Eq<turn::type_, String>>,
        Option<diesel::dsl::Eq<turn::provider_id, i32>>,
        Option<diesel::dsl::Eq<turn::provider_name, &'a str>>,
        Option<diesel::dsl::Eq<turn::model_id, &'a str>>,
    ) as diesel::insertable::Insertable<turn::table>>::Values;

    fn values(self) -> Self::Values {
        diesel::insertable::Insertable::<turn::table>::values((
            Some(turn::session_id.eq(self.session_id)),
            Some(turn::type_.eq(self.ty.to_string())),
            self.provider_id.map(|x| turn::provider_id.eq(x)),
            self.provider_name.map(|x| turn::provider_name.eq(x)),
            self.model_id.map(|x| turn::model_id.eq(x)),
        ))
    }
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
        Ok(TurnType::from_str(&self.ty)?)
    }
}

/// Exposed fields:
/// - id: number
/// - ty: string
/// - session_id: number
/// - provider_id: number | null
/// - provider_name: string | null
/// - model_id: string | null
/// - created_at: string (ISO 8601)
/// - updated_at: string (ISO 8601)
impl DataQuery for Turn {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "id": self.id,
                "ty": self.ty,
                "session_id": self.session_id,
                "provider_id": self.provider_id,
                "provider_name": self.provider_name,
                "model_id": self.model_id,
                "created_at": self.created_at.to_json(),
                "updated_at": self.updated_at.to_json(),
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

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = response)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Response {
    pub id: i32,
    pub session_id: i32,
    pub turn_id: i32,
    pub upstream_id: Option<String>,
    pub upstream_status: Option<String>,
    pub input_tokens: Option<i64>,
    pub cached_input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub reasoning_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub raw_request: Option<Value>,
    pub raw_response: Option<Value>,
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
                dsl::raw_response.eq(raw_response.cloned()),
            ))
            .execute(conn)?;
        Ok(())
    }
}

/// Exposed fields:
/// - id: number
/// - session_id: number
/// - turn_id: number
/// - upstream_id: string | null
/// - upstream_status: string | null
/// - input_tokens: number | null
/// - cached_input_tokens: number | null
/// - output_tokens: number | null
/// - reasoning_tokens: number | null
/// - total_tokens: number | null
/// - raw_request: JSON | null
/// - raw_response: JSON | null
/// - created_at: string (ISO 8601)
/// - updated_at: string (ISO 8601)
impl DataQuery for Response {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "id": self.id,
                "session_id": self.session_id,
                "turn_id": self.turn_id,
                "upstream_id": self.upstream_id,
                "upstream_status": self.upstream_status,
                "input_tokens": self.input_tokens,
                "cached_input_tokens": self.cached_input_tokens,
                "output_tokens": self.output_tokens,
                "reasoning_tokens": self.reasoning_tokens,
                "total_tokens": self.total_tokens,
                "raw_request": self.raw_request,
                "raw_response": self.raw_response,
                "created_at": self.created_at.to_json(),
                "updated_at": self.updated_at.to_json(),
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

#[derive(Debug, Clone, Eq, PartialEq, Queryable, Selectable)]
#[diesel(table_name = item)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Item {
    pub id: i32,
    pub session_id: i32,
    pub turn_id: i32,
    pub response_id: Option<i32>,
    pub provider_id: Option<i32>,
    #[diesel(column_name = "type_")]
    pub ty: String,
    pub upstream_id: Option<String>,
    pub upstream_type: Option<String>,
    pub upstream_call_id: Option<String>,
    pub text: Option<String>,
    pub summary: Option<String>,
    pub encrypted_text: Option<String>,
    pub json: Option<Value>,
    pub raw_data: Option<Value>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug)]
pub struct NewItem<'a> {
    pub session_id: i32,
    pub turn_id: i32,
    pub response_id: Option<i32>,
    pub provider_id: Option<i32>,
    pub ty: ItemType,
    pub upstream_id: Option<&'a str>,
    pub upstream_type: Option<&'a str>,
    pub upstream_call_id: Option<&'a str>,
    pub text: Option<&'a str>,
}

impl<'a> diesel::insertable::Insertable<item::table> for NewItem<'a> {
    type Values = <(
        Option<diesel::dsl::Eq<item::session_id, i32>>,
        Option<diesel::dsl::Eq<item::turn_id, i32>>,
        Option<diesel::dsl::Eq<item::response_id, i32>>,
        Option<diesel::dsl::Eq<item::provider_id, i32>>,
        Option<diesel::dsl::Eq<item::type_, String>>,
        Option<diesel::dsl::Eq<item::upstream_id, &'a str>>,
        Option<diesel::dsl::Eq<item::upstream_type, &'a str>>,
        Option<diesel::dsl::Eq<item::upstream_call_id, &'a str>>,
        Option<diesel::dsl::Eq<item::text, &'a str>>,
    ) as diesel::insertable::Insertable<item::table>>::Values;

    fn values(self) -> Self::Values {
        diesel::insertable::Insertable::<item::table>::values((
            Some(item::session_id.eq(self.session_id)),
            Some(item::turn_id.eq(self.turn_id)),
            self.response_id.map(|x| item::response_id.eq(x)),
            self.provider_id.map(|x| item::provider_id.eq(x)),
            Some(item::type_.eq(self.ty.to_string())),
            self.upstream_id.map(|x| item::upstream_id.eq(x)),
            self.upstream_type.map(|x| item::upstream_type.eq(x)),
            self.upstream_call_id.map(|x| item::upstream_call_id.eq(x)),
            self.text.map(|x| item::text.eq(x)),
        ))
    }
}

impl Item {
    pub fn create(conn: &mut SqliteConnection, new: NewItem<'_>) -> AnyResult<Item> {
        let item = diesel::insert_into(item::table)
            .values(new)
            .returning(item::all_columns)
            .get_result(conn)?;
        Ok(item)
    }

    pub fn get_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<Option<Item>> {
        let result = item::table
            .filter(item::id.eq(id))
            .first::<Item>(conn)
            .optional()?;
        Ok(result)
    }

    pub fn list_by_session(conn: &mut SqliteConnection, session_id: i32) -> AnyResult<Vec<Item>> {
        let items = item::table
            .filter(item::session_id.eq(session_id))
            .order(item::id.asc())
            .load::<Item>(conn)?;
        Ok(items)
    }

    pub fn list_by_turn(conn: &mut SqliteConnection, turn_id: i32) -> AnyResult<Vec<Item>> {
        let items = item::table
            .filter(item::turn_id.eq(turn_id))
            .order(item::id.asc())
            .load::<Item>(conn)?;
        Ok(items)
    }

    pub fn delete_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<bool> {
        let count = diesel::delete(item::table.filter(item::id.eq(id))).execute(conn)?;
        Ok(count > 0)
    }

    pub fn update_text(conn: &mut SqliteConnection, id: i32, text: &str) -> AnyResult<()> {
        use crate::schema::item::dsl;
        diesel::update(dsl::item.filter(dsl::id.eq(id)))
            .set(dsl::text.eq(text))
            .execute(conn)?;
        Ok(())
    }

    pub fn update_summary(conn: &mut SqliteConnection, id: i32, summary: &str) -> AnyResult<()> {
        use crate::schema::item::dsl;
        diesel::update(dsl::item.filter(dsl::id.eq(id)))
            .set(dsl::summary.eq(summary))
            .execute(conn)?;
        Ok(())
    }

    pub fn set_raw_data(conn: &mut SqliteConnection, id: i32, raw_data: &Value) -> AnyResult<()> {
        use crate::schema::item::dsl;
        diesel::update(dsl::item.filter(dsl::id.eq(id)))
            .set(dsl::raw_data.eq(raw_data.clone()))
            .execute(conn)?;
        Ok(())
    }

    pub fn ty(&self) -> AnyResult<ItemType> {
        Ok(ItemType::from_str(&self.ty)?)
    }
}

/// Exposed fields:
/// - id: number
/// - session_id: number
/// - turn_id: number
/// - response_id: number | null
/// - provider_id: number | null
/// - ty: string
/// - upstream_id: string | null
/// - upstream_type: string | null
/// - upstream_call_id: string | null
/// - text: string | null
/// - summary: string | null
/// - encrypted_text: string | null
/// - json: JSON | null
/// - raw_data: JSON | null
/// - created_at: string (ISO 8601)
/// - updated_at: string (ISO 8601)
impl DataQuery for Item {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "id": self.id,
                "session_id": self.session_id,
                "turn_id": self.turn_id,
                "response_id": self.response_id,
                "provider_id": self.provider_id,
                "ty": self.ty,
                "upstream_id": self.upstream_id,
                "upstream_type": self.upstream_type,
                "upstream_call_id": self.upstream_call_id,
                "text": self.text,
                "summary": self.summary,
                "encrypted_text": self.encrypted_text,
                "json": self.json,
                "raw_data": self.raw_data,
                "created_at": self.created_at.to_json(),
                "updated_at": self.updated_at.to_json(),
            }))),
            "id" => Ok(QueryField::Value(json!(self.id))),
            "session_id" => Ok(QueryField::Value(json!(self.session_id))),
            "turn_id" => Ok(QueryField::Value(json!(self.turn_id))),
            "response_id" => Ok(QueryField::Value(json!(self.response_id))),
            "provider_id" => Ok(QueryField::Value(json!(self.provider_id))),
            "ty" => Ok(QueryField::Value(json!(self.ty))),
            "upstream_id" => Ok(QueryField::Value(json!(self.upstream_id))),
            "upstream_type" => Ok(QueryField::Value(json!(self.upstream_type))),
            "upstream_call_id" => Ok(QueryField::Value(json!(self.upstream_call_id))),
            "text" => Ok(QueryField::Value(json!(self.text))),
            "summary" => Ok(QueryField::Value(json!(self.summary))),
            "encrypted_text" => Ok(QueryField::Value(json!(self.encrypted_text))),
            "json" => Ok(QueryField::Value(json!(self.json))),
            "raw_data" => Ok(QueryField::Value(json!(self.raw_data))),
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
    fn test_session_crud() {
        let mut conn = crate::db::open_new().expect("failed to open in-memory db");

        let s1 = Session::create(&mut conn, "Session 1").expect("create session failed");
        assert_eq!(s1.name, "Session 1");

        let s2 = Session::create(&mut conn, "Session 2").expect("create session failed");
        assert_eq!(s2.name, "Session 2");

        let fetched = Session::get_by_id(&mut conn, s1.id)
            .expect("get failed")
            .expect("session not found");
        assert_eq!(fetched.id, s1.id);
        assert_eq!(fetched.name, "Session 1");

        let all = Session::all(&mut conn).expect("all failed");
        assert_eq!(all.len(), 2);

        let deleted = Session::delete_by_id(&mut conn, s1.id).expect("delete failed");
        assert!(deleted);

        let gone = Session::get_by_id(&mut conn, s1.id).expect("get failed");
        assert!(gone.is_none());

        let already_deleted = Session::delete_by_id(&mut conn, s1.id).expect("delete failed");
        assert!(!already_deleted);
    }

    #[test]
    fn test_turn_response_item_crud_and_cascade() {
        let mut conn = crate::db::open_new().expect("failed to open in-memory db");
        let session = Session::create(&mut conn, "Chat").expect("create session");

        // Note: provider_id 999 does not exist in the provider table, proving
        // provider_id is *not* a foreign key
        let user_turn = Turn::create(&mut conn, session.id, TurnType::User, None, None, None)
            .expect("create user turn");
        assert_eq!(user_turn.session_id, session.id);
        assert_eq!(user_turn.ty, "user");
        assert_eq!(user_turn.provider_id, None);
        assert_eq!(user_turn.provider_name, None);
        assert_eq!(user_turn.model_id, None);

        let assistant_turn = Turn::create(
            &mut conn,
            session.id,
            TurnType::Assistant,
            Some(999),
            Some("openai"),
            Some("gpt-4o"),
        )
        .expect("create assistant turn");
        assert_eq!(assistant_turn.ty, "assistant");
        assert_eq!(assistant_turn.provider_id, Some(999));
        assert_eq!(assistant_turn.provider_name.as_deref(), Some("openai"));
        assert_eq!(assistant_turn.model_id.as_deref(), Some("gpt-4o"));

        let turns = Turn::list_by_session(&mut conn, session.id).expect("list turns");
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].id, user_turn.id);
        assert_eq!(turns[1].id, assistant_turn.id);

        let response = Response::create(
            &mut conn,
            session.id,
            assistant_turn.id,
            Some("resp-1"),
            Some("in_progress"),
        )
        .expect("create response");
        assert_eq!(response.session_id, session.id);
        assert_eq!(response.turn_id, assistant_turn.id);
        assert_eq!(response.upstream_id.as_deref(), Some("resp-1"));
        assert_eq!(response.upstream_status.as_deref(), Some("in_progress"));
        assert_eq!(response.input_tokens, None);
        assert_eq!(response.raw_request, None);
        assert_eq!(response.raw_response, None);

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
        )
        .expect("update completion");
        let fetched = Response::get_by_id(&mut conn, response.id)
            .expect("get response")
            .expect("response not found");
        assert_eq!(fetched.upstream_status.as_deref(), Some("completed"));
        assert_eq!(fetched.input_tokens, Some(12));
        assert_eq!(fetched.cached_input_tokens, Some(4));
        assert_eq!(fetched.output_tokens, Some(18));
        assert_eq!(fetched.reasoning_tokens, Some(7));
        assert_eq!(fetched.total_tokens, Some(30));
        assert_eq!(fetched.raw_response, Some(raw_response));

        let prompt = Item::create(
            &mut conn,
            NewItem {
                session_id: session.id,
                turn_id: user_turn.id,
                response_id: None,
                provider_id: None,
                ty: ItemType::UserText,
                upstream_id: None,
                upstream_type: None,
                upstream_call_id: None,
                text: Some("hello"),
            },
        )
        .expect("create prompt item");
        assert_eq!(prompt.session_id, session.id);
        assert_eq!(prompt.turn_id, user_turn.id);
        assert_eq!(prompt.ty, "user_text");
        assert_eq!(prompt.ty().unwrap(), ItemType::UserText);
        assert_eq!(prompt.text.as_deref(), Some("hello"));
        assert_eq!(prompt.response_id, None);

        let reasoning = Item::create(
            &mut conn,
            NewItem {
                session_id: session.id,
                turn_id: assistant_turn.id,
                response_id: Some(response.id),
                provider_id: Some(999),
                ty: ItemType::Reasoning,
                upstream_id: Some("rs_1"),
                upstream_type: Some("reasoning"),
                upstream_call_id: None,
                text: None,
            },
        )
        .expect("create reasoning item");
        assert_eq!(reasoning.ty().unwrap(), ItemType::Reasoning);
        assert_eq!(reasoning.upstream_id.as_deref(), Some("rs_1"));
        assert_eq!(reasoning.upstream_type.as_deref(), Some("reasoning"));

        Item::update_text(&mut conn, reasoning.id, "thinking").expect("update text");
        Item::update_summary(&mut conn, reasoning.id, "summarizing").expect("update summary");
        let raw_data = json!({"id": "rs_1", "type": "reasoning"});
        Item::set_raw_data(&mut conn, reasoning.id, &raw_data).expect("set raw data");
        let fetched = Item::get_by_id(&mut conn, reasoning.id)
            .expect("get item")
            .expect("item not found");
        assert_eq!(fetched.text.as_deref(), Some("thinking"));
        assert_eq!(fetched.summary.as_deref(), Some("summarizing"));
        assert_eq!(fetched.encrypted_text, None);
        assert_eq!(fetched.json, None);
        assert_eq!(fetched.raw_data, Some(raw_data));

        let answer = Item::create(
            &mut conn,
            NewItem {
                session_id: session.id,
                turn_id: assistant_turn.id,
                response_id: Some(response.id),
                provider_id: Some(999),
                ty: ItemType::ResponseText,
                upstream_id: Some("msg_1"),
                upstream_type: Some("message"),
                upstream_call_id: None,
                text: None,
            },
        )
        .expect("create answer item");

        let session_items =
            Item::list_by_session(&mut conn, session.id).expect("list session items");
        assert_eq!(session_items.len(), 3);
        assert_eq!(session_items[0].id, prompt.id);
        assert_eq!(session_items[1].id, reasoning.id);
        assert_eq!(session_items[2].id, answer.id);

        let turn_items = Item::list_by_turn(&mut conn, assistant_turn.id).expect("list turn items");
        assert_eq!(turn_items.len(), 2);
        assert_eq!(turn_items[0].id, reasoning.id);
        assert_eq!(turn_items[1].id, answer.id);

        let turn_responses =
            Response::list_by_turn(&mut conn, assistant_turn.id).expect("list turn responses");
        assert_eq!(turn_responses.len(), 1);
        assert_eq!(turn_responses[0].id, response.id);

        // Deleting a response cascades to its items but keeps the turn
        assert!(Response::delete_by_id(&mut conn, response.id).expect("delete response"));
        assert!(Item::get_by_id(&mut conn, reasoning.id).unwrap().is_none());
        assert!(Item::get_by_id(&mut conn, answer.id).unwrap().is_none());
        assert!(Item::get_by_id(&mut conn, prompt.id).unwrap().is_some());
        assert!(
            Turn::get_by_id(&mut conn, assistant_turn.id)
                .unwrap()
                .is_some()
        );

        // Deleting a turn cascades to its responses and items
        let response2 = Response::create(
            &mut conn,
            session.id,
            assistant_turn.id,
            Some("resp-2"),
            Some("in_progress"),
        )
        .expect("create response2");
        let orphan = Item::create(
            &mut conn,
            NewItem {
                session_id: session.id,
                turn_id: assistant_turn.id,
                response_id: Some(response2.id),
                provider_id: None,
                ty: ItemType::ResponseText,
                upstream_id: None,
                upstream_type: None,
                upstream_call_id: None,
                text: Some("orphan"),
            },
        )
        .expect("create orphan item");
        assert!(Turn::delete_by_id(&mut conn, assistant_turn.id).expect("delete turn"));
        assert!(
            Response::get_by_id(&mut conn, response2.id)
                .unwrap()
                .is_none()
        );
        assert!(Item::get_by_id(&mut conn, orphan.id).unwrap().is_none());

        // Deleting the session cascades to remaining turns and items
        assert!(Session::delete_by_id(&mut conn, session.id).expect("delete session"));
        assert!(Item::get_by_id(&mut conn, prompt.id).unwrap().is_none());
        assert!(Turn::get_by_id(&mut conn, user_turn.id).unwrap().is_none());

        assert_eq!(
            ItemType::from_upstream("message"),
            Some(ItemType::ResponseText)
        );
        assert_eq!(
            ItemType::from_upstream("reasoning"),
            Some(ItemType::Reasoning)
        );
        assert_eq!(ItemType::from_upstream("function_call"), None);
        assert!("bogus".parse::<ItemType>().is_err());
        assert!("bogus".parse::<TurnType>().is_err());
    }

    #[test]
    fn test_data_query() {
        use serde_json::json;

        let mut conn = crate::db::open_new().expect("failed to open in-memory db");
        let session = Session::create(&mut conn, "Session 1").expect("create session failed");
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
        )
        .expect("create turn failed");
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
        )
        .expect("create response failed");
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
        )
        .expect("update completion failed");
        let response = Response::get_by_id(&mut conn, response.id)
            .expect("get response")
            .expect("response not found");
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

        let raw_data = json!({"id": "rs_1", "type": "reasoning"});
        let item = Item::create(
            &mut conn,
            NewItem {
                session_id: session.id,
                turn_id: turn.id,
                response_id: Some(response.id),
                provider_id: Some(1),
                ty: ItemType::Reasoning,
                upstream_id: Some("rs_1"),
                upstream_type: Some("reasoning"),
                upstream_call_id: None,
                text: Some("thinking"),
            },
        )
        .expect("create item failed");
        Item::set_raw_data(&mut conn, item.id, &raw_data).expect("set raw data");
        let item = Item::get_by_id(&mut conn, item.id)
            .expect("get item")
            .expect("item not found");
        assert_eq!(
            item.query("/").unwrap(),
            json!({
                "id": item.id,
                "session_id": item.session_id,
                "turn_id": item.turn_id,
                "response_id": item.response_id,
                "provider_id": item.provider_id,
                "ty": item.ty,
                "upstream_id": item.upstream_id,
                "upstream_type": item.upstream_type,
                "upstream_call_id": item.upstream_call_id,
                "text": item.text,
                "summary": item.summary,
                "encrypted_text": item.encrypted_text,
                "json": item.json,
                "raw_data": item.raw_data,
                "created_at": item.created_at.to_json(),
                "updated_at": item.updated_at.to_json(),
            })
        );
        assert_eq!(item.query("/ty").unwrap(), json!(item.ty));
        assert_eq!(
            item.query("/upstream_type").unwrap(),
            json!(item.upstream_type)
        );
        assert_eq!(item.query("/text").unwrap(), json!(item.text));
        assert_eq!(item.query("/raw_data").unwrap(), json!(item.raw_data));
    }
}
