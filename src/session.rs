use anyhow::{anyhow, bail};
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
use crate::schema::{item, response, session, turn};
use crate::tools::ToolResult;

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

#[derive(Debug, Clone, Copy, Eq, PartialEq, diesel::AsExpression, diesel::FromSqlRow)]
#[diesel(sql_type = Text)]
pub enum ItemType {
    UserText,
    ResponseText,
    Reasoning,
    ToolCall,
}

impl fmt::Display for ItemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ItemType::UserText => write!(f, "user_text"),
            ItemType::ResponseText => write!(f, "response_text"),
            ItemType::Reasoning => write!(f, "reasoning"),
            ItemType::ToolCall => write!(f, "tool_call"),
        }
    }
}

impl FromStr for ItemType {
    type Err = AnyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user_text" => Ok(ItemType::UserText),
            "response_text" => Ok(ItemType::ResponseText),
            "reasoning" => Ok(ItemType::Reasoning),
            "tool_call" => Ok(ItemType::ToolCall),
            other => Err(anyhow!("unknown item type: {other}")),
        }
    }
}

impl ToSql<Text, Sqlite> for ItemType {
    fn to_sql<'b>(&'b self, out: &mut serialize::Output<'b, '_, Sqlite>) -> serialize::Result {
        out.set_value(self.to_string());
        Ok(IsNull::No)
    }
}

impl FromSql<Text, Sqlite> for ItemType {
    fn from_sql(bytes: <Sqlite as Backend>::RawValue<'_>) -> deserialize::Result<Self> {
        let value = <String as FromSql<Text, Sqlite>>::from_sql(bytes)?;
        value.parse().map_err(Into::into)
    }
}

impl ItemType {
    pub fn from_upstream(ty: &str) -> Option<Self> {
        match ty {
            "message" => Some(ItemType::ResponseText),
            "reasoning" => Some(ItemType::Reasoning),
            "function_call" => Some(ItemType::ToolCall),
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

#[derive(Clone, Debug)]
pub struct ToolCallArgs {
    pub name: String,
    pub args: Value,
}

impl DataQuery for ToolCallArgs {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "name": self.query("/name")?,
                "args": self.query("/args")?,
            }))),
            "name" => Ok(QueryField::Value(self.name.clone().into())),
            "args" => Ok(QueryField::Value(self.args.clone())),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

/// Basic datum in human/agent/tool loop history.
#[derive(Debug, Clone, Eq, PartialEq, Queryable, Selectable)]
#[diesel(table_name = item)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct DbItem {
    pub id: i32,
    pub session_id: i32,
    pub turn_id: i32,
    pub response_id: Option<i32>,
    pub provider_id: Option<i32>,
    /// Maps to ItemType, may be converted from upstream_type
    pub ty: String,
    pub upstream_id: Option<String>,
    /// Raw type from API
    pub upstream_type: Option<String>,
    /// Correlates tool call and tool output
    pub upstream_call_id: Option<String>,
    /// Used by user_text, response_text, and reasoning. Also stores the tool
    /// name on tool_call
    pub text: Option<String>,
    /// Reasoning summary
    pub summary: Option<String>,
    /// Encrypted reasoning
    pub encrypted_text: Option<String>,
    /// Tool call args
    pub tool_args: Option<String>,
    // TODO: config setting to disable
    pub raw_data: Option<String>,
    /// Position in the session's item ordering; unique per session.
    pub seqno: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
    /// For tool calls, the JSON output of the call, if finished
    pub tool_output: Option<String>,
}

#[derive(Debug, Default, Insertable)]
#[diesel(table_name = item)]
pub struct NewItem<'a> {
    pub session_id: Option<i32>,
    pub turn_id: Option<i32>,
    pub response_id: Option<i32>,
    pub provider_id: Option<i32>,
    pub ty: Option<ItemType>,
    pub upstream_id: Option<&'a str>,
    pub upstream_type: Option<&'a str>,
    pub upstream_call_id: Option<&'a str>,
    pub text: Option<&'a str>,
    /// Explicit seqno, or `None` to autoincrement
    pub seqno: Option<i64>,
}

impl DbItem {
    pub fn create(conn: &mut SqliteConnection, new: NewItem<'_>) -> AnyResult<DbItem> {
        if new.seqno.is_some() {
            return Self::insert(conn, new);
        }
        let Some(session_id) = new.session_id else {
            return Self::insert(conn, new);
        };
        conn.transaction(|conn| {
            let seqno = Self::max_seqno(conn, session_id)?.unwrap_or(0) + 1;
            Self::insert(conn, NewItem {
                seqno: Some(seqno),
                ..new
            })
        })
    }

    fn insert(conn: &mut SqliteConnection, new: NewItem<'_>) -> AnyResult<DbItem> {
        let item = diesel::insert_into(item::table)
            .values(new)
            .returning(item::all_columns)
            .get_result(conn)?;
        Ok(item)
    }

    /// Returns the highest seqno for the session, if any.
    // XXX: Why not just next_seqno() -> max + 1 with default, done in query...
    pub fn max_seqno(conn: &mut SqliteConnection, session_id: i32) -> AnyResult<Option<i64>> {
        let result = item::table
            .filter(item::session_id.eq(session_id))
            .select(diesel::dsl::max(item::seqno))
            .first::<Option<i64>>(conn)?;
        Ok(result)
    }

    pub fn get_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<Option<DbItem>> {
        let result = item::table
            .filter(item::id.eq(id))
            .first::<DbItem>(conn)
            .optional()?;
        Ok(result)
    }

    pub fn list_by_session(conn: &mut SqliteConnection, session_id: i32) -> AnyResult<Vec<DbItem>> {
        let items = item::table
            .filter(item::session_id.eq(session_id))
            .order(item::seqno.asc())
            .load::<DbItem>(conn)?;
        Ok(items)
    }

    pub fn list_by_turn(conn: &mut SqliteConnection, turn_id: i32) -> AnyResult<Vec<DbItem>> {
        let items = item::table
            .filter(item::turn_id.eq(turn_id))
            .order(item::seqno.asc())
            .load::<DbItem>(conn)?;
        Ok(items)
    }

    pub fn tool_calls_by_response(
        conn: &mut SqliteConnection,
        response_id: i32,
    ) -> AnyResult<Vec<DbItem>> {
        use crate::schema::item::dsl;
        let items = dsl::item
            .filter(dsl::response_id.eq(response_id))
            .filter(dsl::ty.eq(ItemType::ToolCall.to_string()))
            .order(dsl::seqno.asc())
            .load::<DbItem>(conn)?;
        Ok(items)
    }

    pub fn delete_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<bool> {
        let count = diesel::delete(item::table.filter(item::id.eq(id))).execute(conn)?;
        Ok(count > 0)
    }

    pub fn update_text(&mut self, conn: &mut SqliteConnection, text: impl Into<String>) -> AnyResult<()> {
        use crate::schema::item::dsl;
        self.text = Some(text.into());
        diesel::update(dsl::item.filter(dsl::id.eq(self.id)))
            .set(dsl::text.eq(&self.text))
            .execute(conn)?;
        Ok(())
    }

    pub fn update_summary(&mut self, conn: &mut SqliteConnection, summary: impl Into<String>) -> AnyResult<()> {
        use crate::schema::item::dsl;
        self.summary = Some(summary.into());
        diesel::update(dsl::item.filter(dsl::id.eq(self.id)))
            .set(dsl::summary.eq(&self.summary))
            .execute(conn)?;
        Ok(())
    }

    pub fn update_tool_args(&mut self, conn: &mut SqliteConnection, tool_args: impl Into<String>) -> AnyResult<()> {
        use crate::schema::item::dsl;
        self.tool_args = Some(tool_args.into());
        diesel::update(dsl::item.filter(dsl::id.eq(self.id)))
            .set(dsl::tool_args.eq(&self.tool_args))
            .execute(conn)?;
        Ok(())
    }

    /// Writes a tool call's result. The result's name replaces the item's
    /// name, which is how failed calls are re-routed to the failure tool.
    pub fn set_tool_output(
        &mut self,
        conn: &mut SqliteConnection,
        result: &ToolResult,
    ) -> AnyResult<()> {
        use crate::schema::item::dsl;
        let tool_output = result.output.to_string();
        diesel::update(dsl::item.filter(dsl::id.eq(self.id)))
            .set((dsl::text.eq(&result.name), dsl::tool_output.eq(&tool_output)))
            .execute(conn)?;
        self.text = Some(result.name.clone());
        self.tool_output = Some(tool_output);
        Ok(())
    }

    pub fn set_raw_data(conn: &mut SqliteConnection, id: i32, raw_data: &Value) -> AnyResult<()> {
        use crate::schema::item::dsl;
        diesel::update(dsl::item.filter(dsl::id.eq(id)))
            .set(dsl::raw_data.eq(raw_data.to_string()))
            .execute(conn)?;
        Ok(())
    }

    pub fn ty(&self) -> AnyResult<ItemType> {
        ItemType::from_str(&self.ty)
    }

    /// Parse the stored tool args JSON blob, if present.
    pub fn tool_args_json(&self) -> AnyResult<Option<Value>> {
        self.tool_args
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(Into::into)
    }

    pub fn tool_args(&self) -> AnyResult<Option<ToolCallArgs>> {
        if self.ty()? != ItemType::ToolCall {
            Ok(None)
        } else if let Some(name) = self.text.clone()
            && let Some(args) = self.tool_args_json()?.clone()
        {
            Ok(Some(ToolCallArgs { name, args }))
        } else {
            bail!("tool call item {} has no args", self.id);
        }
    }

    /// For tool call items, parses the stored output, if present.
    pub fn tool_output(&self) -> AnyResult<Option<Value>> {
        self.tool_output
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(Into::into)
    }
}

impl DataQuery for DbItem {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "id": self.query("/id")?,
                "session_id": self.query("/session_id")?,
                "turn_id": self.query("/turn_id")?,
                "response_id": self.query("/response_id")?,
                "provider_id": self.query("/provider_id")?,
                "ty": self.query("/ty")?,
                "upstream_id": self.query("/upstream_id")?,
                "upstream_type": self.query("/upstream_type")?,
                "upstream_call_id": self.query("/upstream_call_id")?,
                "text": self.query("/text")?,
                "summary": self.query("/summary")?,
                "encrypted_text": self.query("/encrypted_text")?,
                "tool_args": self.query("/tool_args")?,
                "raw_data": self.query("/raw_data")?,
                "seqno": self.query("/seqno")?,
                "created_at": self.query("/created_at")?,
                "updated_at": self.query("/updated_at")?,
                "tool_output": self.query("/tool_output")?,
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
            "tool_args" => Ok(QueryField::Value(
                self.tool_args_json()
                    .map_err(|e| QueryError::DataError(e.to_string()))?
                    .unwrap_or(Value::Null),
            )),
            "raw_data" => Ok(QueryField::Value(json!(self.raw_data))),
            "seqno" => Ok(QueryField::Value(json!(self.seqno))),
            "tool_output" => Ok(QueryField::Value(
                self.tool_output()
                    .map_err(|e| QueryError::DataError(e.to_string()))?
                    .unwrap_or(Value::Null),
            )),
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
        let turn_ty = turn::table
            .filter(turn::id.eq(user_turn.id))
            .select(turn::ty)
            .first::<TurnType>(&mut conn)
            .expect("read turn type");
        assert_eq!(turn_ty, TurnType::User);

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
        assert_eq!(fetched.raw_response, Some(raw_response.to_string()));

        let prompt = DbItem::create(
            &mut conn,
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(user_turn.id),
                ty: Some(ItemType::UserText),
                text: Some("hello"),
                ..Default::default()
            },
        )
        .expect("create prompt item");
        assert_eq!(prompt.session_id, session.id);
        assert_eq!(prompt.turn_id, user_turn.id);
        assert_eq!(prompt.ty, "user_text");
        assert_eq!(prompt.ty().unwrap(), ItemType::UserText);
        assert_eq!(prompt.text.as_deref(), Some("hello"));
        assert_eq!(prompt.response_id, None);
        let item_ty = item::table
            .filter(item::id.eq(prompt.id))
            .select(item::ty)
            .first::<ItemType>(&mut conn)
            .expect("read item type");
        assert_eq!(item_ty, ItemType::UserText);

        let mut reasoning = DbItem::create(
            &mut conn,
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(assistant_turn.id),
                response_id: Some(response.id),
                provider_id: Some(999),
                ty: Some(ItemType::Reasoning),
                upstream_id: Some("rs_1"),
                upstream_type: Some("reasoning"),
                ..Default::default()
            },
        )
        .expect("create reasoning item");
        assert_eq!(reasoning.ty().unwrap(), ItemType::Reasoning);
        assert_eq!(reasoning.upstream_id.as_deref(), Some("rs_1"));
        assert_eq!(reasoning.upstream_type.as_deref(), Some("reasoning"));

        reasoning.update_text(&mut conn, "thinking").expect("update text");
        reasoning.update_summary(&mut conn, "summarizing").expect("update summary");
        let raw_data = json!({"id": "rs_1", "type": "reasoning"});
        DbItem::set_raw_data(&mut conn, reasoning.id, &raw_data).expect("set raw data");
        let fetched = DbItem::get_by_id(&mut conn, reasoning.id)
            .expect("get item")
            .expect("item not found");
        assert_eq!(fetched.text.as_deref(), Some("thinking"));
        assert_eq!(fetched.summary.as_deref(), Some("summarizing"));
        assert_eq!(fetched.encrypted_text, None);
        assert_eq!(fetched.tool_args, None);
        assert_eq!(fetched.raw_data, Some(raw_data.to_string()));

        let answer = DbItem::create(
            &mut conn,
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(assistant_turn.id),
                response_id: Some(response.id),
                provider_id: Some(999),
                ty: Some(ItemType::ResponseText),
                upstream_id: Some("msg_1"),
                upstream_type: Some("message"),
                ..Default::default()
            },
        )
        .expect("create answer item");

        let mut tool_call = DbItem::create(
            &mut conn,
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(assistant_turn.id),
                response_id: Some(response.id),
                provider_id: Some(999),
                ty: Some(ItemType::ToolCall),
                upstream_id: Some("fc_1"),
                upstream_type: Some("function_call"),
                upstream_call_id: Some("call_1"),
                text: Some("read_file"),
                ..Default::default()
            },
        )
        .expect("create tool call item");
        assert_eq!(tool_call.ty, "tool_call");
        assert_eq!(tool_call.ty().unwrap(), ItemType::ToolCall);
        assert_eq!(tool_call.upstream_call_id.as_deref(), Some("call_1"));
        assert_eq!(tool_call.tool_output, None);

        tool_call.update_tool_args(&mut conn, r#"{"path": "a.txt"}"#)
            .expect("update tool args");
        let output = json!({"error": "file not found"});
        let mut fetched_call = DbItem::get_by_id(&mut conn, tool_call.id)
            .expect("get tool call")
            .expect("tool call not found");
        let result = ToolResult {
            name: "read_file".to_owned(),
            output: output.clone(),
        };
        fetched_call.set_tool_output(&mut conn, &result).expect("set tool output");
        assert_eq!(fetched_call.text.as_deref(), Some("read_file"));
        assert_eq!(fetched_call.tool_output().unwrap(), Some(output));
        let args = fetched_call.tool_args().expect("tool args").expect("no args");
        assert_eq!(args.name, "read_file");
        assert_eq!(args.args, json!({"path": "a.txt"}));

        let session_items =
            DbItem::list_by_session(&mut conn, session.id).expect("list session items");
        assert_eq!(session_items.len(), 4);
        assert_eq!(session_items[0].id, prompt.id);
        assert_eq!(session_items[1].id, reasoning.id);
        assert_eq!(session_items[2].id, answer.id);
        assert_eq!(session_items[3].id, tool_call.id);

        let turn_items = DbItem::list_by_turn(&mut conn, assistant_turn.id).expect("list turn items");
        assert_eq!(turn_items.len(), 3);
        assert_eq!(turn_items[0].id, reasoning.id);
        assert_eq!(turn_items[1].id, answer.id);
        assert_eq!(turn_items[2].id, tool_call.id);

        let turn_responses =
            Response::list_by_turn(&mut conn, assistant_turn.id).expect("list turn responses");
        assert_eq!(turn_responses.len(), 1);
        assert_eq!(turn_responses[0].id, response.id);

        // Deleting a response cascades to its items but keeps the turn
        assert!(Response::delete_by_id(&mut conn, response.id).expect("delete response"));
        assert!(DbItem::get_by_id(&mut conn, reasoning.id).unwrap().is_none());
        assert!(DbItem::get_by_id(&mut conn, answer.id).unwrap().is_none());
        assert!(DbItem::get_by_id(&mut conn, tool_call.id).unwrap().is_none());
        assert!(DbItem::get_by_id(&mut conn, prompt.id).unwrap().is_some());
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
        let orphan = DbItem::create(
            &mut conn,
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(assistant_turn.id),
                response_id: Some(response2.id),
                ty: Some(ItemType::ResponseText),
                text: Some("orphan"),
                ..Default::default()
            },
        )
        .expect("create orphan item");
        assert!(Turn::delete_by_id(&mut conn, assistant_turn.id).expect("delete turn"));
        assert!(
            Response::get_by_id(&mut conn, response2.id)
                .unwrap()
                .is_none()
        );
        assert!(DbItem::get_by_id(&mut conn, orphan.id).unwrap().is_none());

        // Deleting the session cascades to remaining turns and items
        assert!(Session::delete_by_id(&mut conn, session.id).expect("delete session"));
        assert!(DbItem::get_by_id(&mut conn, prompt.id).unwrap().is_none());
        assert!(Turn::get_by_id(&mut conn, user_turn.id).unwrap().is_none());

        assert_eq!(
            ItemType::from_upstream("message"),
            Some(ItemType::ResponseText)
        );
        assert_eq!(
            ItemType::from_upstream("reasoning"),
            Some(ItemType::Reasoning)
        );
        assert_eq!(
            ItemType::from_upstream("function_call"),
            Some(ItemType::ToolCall)
        );
        assert_eq!(ItemType::from_upstream("function_call_output"), None);
        assert_eq!(
            "tool_call".parse::<ItemType>().unwrap(),
            ItemType::ToolCall
        );
        assert!("tool_output".parse::<ItemType>().is_err());
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
        let item = DbItem::create(
            &mut conn,
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(turn.id),
                response_id: Some(response.id),
                provider_id: Some(1),
                ty: Some(ItemType::Reasoning),
                upstream_id: Some("rs_1"),
                upstream_type: Some("reasoning"),
                text: Some("thinking"),
                ..Default::default()
            },
        )
        .expect("create item failed");
        DbItem::set_raw_data(&mut conn, item.id, &raw_data).expect("set raw data");
        let item = DbItem::get_by_id(&mut conn, item.id)
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
                "tool_args": item.tool_args_json().unwrap(),
                "raw_data": item.raw_data,
                "seqno": item.seqno,
                "created_at": item.created_at.to_json(),
                "updated_at": item.updated_at.to_json(),
                "tool_output": item.tool_output().unwrap(),
            })
        );
        assert_eq!(item.query("/ty").unwrap(), json!(item.ty));
        assert_eq!(
            item.query("/upstream_type").unwrap(),
            json!(item.upstream_type)
        );
        assert_eq!(item.query("/text").unwrap(), json!(item.text));
        assert_eq!(item.query("/raw_data").unwrap(), json!(item.raw_data));
        assert_eq!(item.query("/seqno").unwrap(), json!(item.seqno));
        assert_eq!(
            item.query("/tool_output").unwrap(),
            json!(item.tool_output().unwrap())
        );
    }

    fn make_item(
        conn: &mut SqliteConnection,
        session_id: i32,
        turn_id: i32,
        seqno: Option<i64>,
    ) -> DbItem {
        DbItem::create(
            conn,
            NewItem {
                session_id: Some(session_id),
                turn_id: Some(turn_id),
                ty: Some(ItemType::UserText),
                text: Some("hi"),
                seqno,
                ..Default::default()
            },
        )
        .expect("create item")
    }

    #[test]
    fn test_seqno() {
        let mut conn = crate::db::open_new().expect("failed to open in-memory db");
        let session = Session::create(&mut conn, "Session").expect("create session");
        let turn = Turn::create(&mut conn, session.id, TurnType::User, None, None, None)
            .expect("create turn");

        // None appends after the session's highest seqno.
        let a = make_item(&mut conn, session.id, turn.id, None);
        let b = make_item(&mut conn, session.id, turn.id, None);
        let c = make_item(&mut conn, session.id, turn.id, None);
        assert_eq!([a.seqno, b.seqno, c.seqno], [1, 2, 3]);
        assert_eq!(DbItem::max_seqno(&mut conn, session.id).unwrap(), Some(3));

        // Explicit seqnos win over insertion order.
        let late = make_item(&mut conn, session.id, turn.id, Some(10));
        let early = make_item(&mut conn, session.id, turn.id, Some(5));
        let items = DbItem::list_by_session(&mut conn, session.id).unwrap();
        let ids: Vec<i32> = items.iter().map(|i| i.id).collect();
        assert_eq!(
            ids,
            [a.id, b.id, c.id, early.id, late.id],
            "expected seqno ordering, not insertion order"
        );
        assert_eq!(DbItem::max_seqno(&mut conn, session.id).unwrap(), Some(10));

        // Duplicate (session_id, seqno) is rejected by the unique index.
        let duplicate = DbItem::create(
            &mut conn,
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(turn.id),
                ty: Some(ItemType::UserText),
                text: Some("dup"),
                seqno: Some(5),
                ..Default::default()
            },
        );
        assert!(duplicate.is_err());

        // list_by_turn orders by seqno too.
        let turn_items = DbItem::list_by_turn(&mut conn, turn.id).unwrap();
        assert_eq!(turn_items.len(), 5);
    }
}
