use anyhow::{anyhow, bail};
use chrono::NaiveDateTime;
use diesel::backend::Backend;
use diesel::deserialize::{self, FromSql};
use diesel::prelude::*;
use diesel::serialize::{self, IsNull, ToSql};
use diesel::sql_types::Text;
use diesel::sqlite::{Sqlite, SqliteConnection};
use log::warn;
use serde_json::{Value, json};
use std::fmt;
use std::str::FromStr;

use crate::error::{AnyError, AnyResult};
use crate::query::{DataQuery, QueryError, QueryField};
use crate::schema::item;
use crate::try_nested;

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

/// Raw row from the item table.
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
pub struct NewDbItem<'a> {
    pub session_id: Option<i32>,
    pub turn_id: Option<i32>,
    pub response_id: Option<i32>,
    pub provider_id: Option<i32>,
    pub ty: Option<ItemType>,
    pub upstream_id: Option<&'a str>,
    pub upstream_type: Option<&'a str>,
    pub upstream_call_id: Option<&'a str>,
    pub text: Option<&'a str>,
    /// Reasoning summary
    pub summary: Option<&'a str>,
    /// Encrypted reasoning
    pub encrypted_text: Option<&'a str>,
    pub tool_args: Option<String>,
    pub tool_output: Option<String>,
    /// Explicit seqno, or `None` to autoincrement
    pub seqno: Option<i64>,
    pub raw_data: Option<&'a str>,
}

impl DbItem {
    pub fn create(conn: &mut SqliteConnection, mut new: NewDbItem<'_>) -> AnyResult<DbItem> {
        conn.transaction(|conn| {
            if new.seqno.is_none() && let Some(session_id) = new.session_id {
                // Fill in default seqno
                new.seqno = Some(Self::max_seqno(conn, session_id)?.unwrap_or(0) + 1);
            };
            let item = diesel::insert_into(item::table)
                .values(new)
                .returning(item::all_columns)
                .get_result(conn)?;
            Ok(item)
        })
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

    pub fn set_raw_data(&mut self, conn: &mut SqliteConnection, raw_data: String) -> AnyResult<()> {
        use crate::schema::item::dsl;
        self.raw_data = Some(raw_data);
        diesel::update(dsl::item.filter(dsl::id.eq(self.id)))
            .set(dsl::raw_data.eq(self.raw_data.as_deref()))
            .execute(conn)?;
        Ok(())
    }

    pub fn ty(&self) -> AnyResult<ItemType> {
        ItemType::from_str(&self.ty)
    }
}

/// Basic datum in the human/agent/tool loop history.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Item {
    pub id: i32,
    pub session_id: i32,
    pub turn_id: i32,
    pub response_id: Option<i32>,
    pub provider_id: Option<i32>,
    /// Position in the session's item ordering; unique per session.
    pub seqno: i64,
    pub upstream_id: Option<String>,
    pub content: ItemContent,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReasoningContent {
    pub text: Option<String>,
    pub summary: Option<String>,
    pub encrypted: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ToolCallContent {
    // TODO: Some day should add a data format version tag before every tool
    // ends up suffixed with _v2
    pub tool_name: String,
    pub call_id: String,
    pub args: Value,
    pub output: Option<ToolOutput>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ToolOutput {
    Completed { value: Value },
    Failed { error: String },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ItemContent {
    UserText(String),
    ResponseText(String),
    Reasoning(ReasoningContent),
    ToolCall(ToolCallContent),
}

impl ItemContent {
    pub fn as_tool_call(&self) -> Option<&ToolCallContent> {
        match self {
            Self::ToolCall(content) => Some(&content),
            _ => None,
        }
    }
}

/// Item creation parameters
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct NewItem {
    pub session_id: i32,
    pub turn_id: i32,
    pub response_id: Option<i32>,
    pub provider_id: Option<i32>,
    pub upstream_id: Option<String>,
    /// Explicit seqno, or `None` to append to the session.
    pub seqno: Option<i64>,
    pub content: ItemContent,
}

impl Item {
    /// Decodes a database row into an item. Malformed rows are logged and
    /// decode to `None`; they should be ignored.
    pub fn from_row(row: &DbItem) -> Option<Item> {
        Self::from_row_inner(row)
            .inspect_err(|e| warn!("malformed item {}: {e}", row.id))
            .ok()
    }

    fn from_row_inner(row: &DbItem) -> AnyResult<Item> {
        let content = match row.ty()? {
            ItemType::UserText => {
                ItemContent::UserText(require(row.id, "text", row.text.clone())?)
            }
            ItemType::ResponseText => {
                ItemContent::ResponseText(require(row.id, "text", row.text.clone())?)
            }
            ItemType::Reasoning => ItemContent::Reasoning(ReasoningContent {
                text: row.text.clone(),
                summary: row.summary.clone(),
                encrypted: row.encrypted_text.clone(),
            }),
            ItemType::ToolCall => ItemContent::ToolCall(ToolCallContent {
                tool_name: require(row.id, "text", row.text.clone())?,
                call_id: require(row.id, "upstream_call_id", row.upstream_call_id.clone())?,
                args: parse_json(row.id, "tool_args", row.tool_args.as_deref())?,
                output: match row.tool_output.as_deref() {
                    Some(raw) => {
                        Some(decode_output(raw).map_err(|e| anyhow!("item {}: {e}", row.id))?)
                    }
                    None => None,
                },
            }),
        };
        Ok(Item {
            id: row.id,
            session_id: row.session_id,
            turn_id: row.turn_id,
            response_id: row.response_id,
            provider_id: row.provider_id,
            seqno: row.seqno,
            upstream_id: row.upstream_id.clone(),
            content,
        })
    }

    /// Loads an item by id, or `None` if it is missing or undecodable.
    pub fn get(conn: &mut SqliteConnection, id: i32) -> AnyResult<Option<Item>> {
        let row = try_nested!(
            item::table
                .filter(item::id.eq(id))
                .first::<DbItem>(conn)
                .optional()
        );
        Ok(Self::from_row(&row))
    }

    /// Lists a session's items in seqno order, skipping undecodable rows.
    pub fn list_by_session(conn: &mut SqliteConnection, session_id: i32) -> AnyResult<Vec<Item>> {
        let rows = item::table
            .filter(item::session_id.eq(session_id))
            .order(item::seqno.asc())
            .load::<DbItem>(conn)?;
        Ok(Self::decode_rows(rows))
    }

    /// Lists the tool calls belonging to a response, in seqno order.
    pub fn tool_calls_by_response(
        conn: &mut SqliteConnection,
        response_id: i32,
    ) -> AnyResult<Vec<Item>> {
        use crate::schema::item::dsl;
        let rows = dsl::item
            .filter(dsl::response_id.eq(response_id))
            .filter(dsl::ty.eq(ItemType::ToolCall.to_string()))
            .order(dsl::seqno.asc())
            .load::<DbItem>(conn)?;
        Ok(Self::decode_rows(rows))
    }

    /// Test function, lists a turn's items in seqno order
    pub fn list_by_turn(conn: &mut SqliteConnection, turn_id: i32) -> Vec<Item> {
        let rows = item::table
            .filter(item::turn_id.eq(turn_id))
            .order(item::seqno.asc())
            .load::<DbItem>(conn)
            .unwrap();
        Self::decode_rows(rows)
    }

    /// Inserts an item
    pub fn create(conn: &mut SqliteConnection, new: NewItem) -> AnyResult<Item> {
        let mut row = NewDbItem {
            session_id: Some(new.session_id),
            turn_id: Some(new.turn_id),
            response_id: new.response_id,
            provider_id: new.provider_id,
            ty: None,
            upstream_id: new.upstream_id.as_deref(),
            upstream_type: None,
            upstream_call_id: None,
            text: None,
            summary: None,
            encrypted_text: None,
            tool_args: None,
            tool_output: None,
            seqno: new.seqno,
            raw_data: None,
        };
        match &new.content {
            ItemContent::UserText(text) => {
                row.ty = Some(ItemType::UserText);
                row.text = Some(text);
            }
            ItemContent::ResponseText(text) => {
                row.ty = Some(ItemType::ResponseText);
                row.text = Some(text);
            }
            ItemContent::Reasoning(ReasoningContent {
                text,
                summary,
                encrypted,
            }) => {
                row.ty = Some(ItemType::Reasoning);
                row.text = text.as_deref();
                row.summary = summary.as_deref();
                row.encrypted_text = encrypted.as_deref();
            }
            ItemContent::ToolCall(ToolCallContent {
                tool_name,
                call_id,
                args,
                output,
            }) => {
                row.ty = Some(ItemType::ToolCall);
                row.text = Some(tool_name);
                row.upstream_call_id = Some(call_id);
                row.tool_args = Some(args.to_string());
                row.tool_output = output.as_ref().map(encode_output);
            }
        }
        let row = DbItem::create(conn, row)?;
        Ok(Self::from_row(&row).unwrap())
    }

    /// Updates and commits output of a tool call.
    ///
    /// Panics if the item is not a tool call.
    pub fn set_output(&mut self, conn: &mut SqliteConnection, output: ToolOutput) -> AnyResult<()> {
        let ItemContent::ToolCall(ToolCallContent {
            output: slot, ..
        }) = &mut self.content
        else {
            panic!("item {} is not a tool call", self.id);
        };
        let encoded = encode_output(&output);
        diesel::update(item::table.filter(item::id.eq(self.id)))
            .set(item::tool_output.eq(&encoded))
            .execute(conn)?;
        *slot = Some(output);
        Ok(())
    }

    // Decodes rows. Ignores and warns about malformed item data.
    fn decode_rows(rows: Vec<DbItem>) -> Vec<Item> {
        rows.into_iter()
            .filter_map(|row| Self::from_row(&row))
            .collect()
    }
}

impl DataQuery for Item {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "id": self.query("/id")?,
                "session_id": self.query("/session_id")?,
                "turn_id": self.query("/turn_id")?,
                "response_id": self.query("/response_id")?,
                "provider_id": self.query("/provider_id")?,
                "seqno": self.query("/seqno")?,
                "upstream_id": self.query("/upstream_id")?,
                "content": self.query("/content")?,
            }))),
            "id" => Ok(QueryField::Value(json!(self.id))),
            "session_id" => Ok(QueryField::Value(json!(self.session_id))),
            "turn_id" => Ok(QueryField::Value(json!(self.turn_id))),
            "response_id" => Ok(QueryField::Value(json!(self.response_id))),
            "provider_id" => Ok(QueryField::Value(json!(self.provider_id))),
            "seqno" => Ok(QueryField::Value(json!(self.seqno))),
            "upstream_id" => Ok(QueryField::Value(json!(self.upstream_id))),
            "content" => Ok(QueryField::DataQuery(&self.content)),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

/// Queries the content as a variant-tagged object. Fields which are unset or
/// belong to another variant read as `null`.
impl DataQuery for ItemContent {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match self {
            ItemContent::UserText(text) => match field {
                "" => Ok(QueryField::Value(json!({
                    "type": self.query("/type")?,
                    "text": self.query("/text")?,
                }))),
                "type" => Ok(QueryField::Value(json!("user_text"))),
                "text" => Ok(QueryField::Value(json!(text))),
                _ => Err(QueryError::InvalidField(field.to_string())),
            },
            ItemContent::ResponseText(text) => match field {
                "" => Ok(QueryField::Value(json!({
                    "type": self.query("/type")?,
                    "text": self.query("/text")?,
                }))),
                "type" => Ok(QueryField::Value(json!("response_text"))),
                "text" => Ok(QueryField::Value(json!(text))),
                _ => Err(QueryError::InvalidField(field.to_string())),
            },
            ItemContent::Reasoning(ReasoningContent {
                text,
                summary,
                encrypted,
            }) => match field {
                "" => Ok(QueryField::Value(json!({
                    "type": self.query("/type")?,
                    "text": self.query("/text")?,
                    "summary": self.query("/summary")?,
                    "encrypted": self.query("/encrypted")?,
                }))),
                "type" => Ok(QueryField::Value(json!("reasoning"))),
                "text" => Ok(QueryField::Value(json!(text))),
                "summary" => Ok(QueryField::Value(json!(summary))),
                "encrypted" => Ok(QueryField::Value(json!(encrypted))),
                _ => Err(QueryError::InvalidField(field.to_string())),
            },
            ItemContent::ToolCall(ToolCallContent {
                tool_name,
                call_id,
                args,
                output,
            }) => match field {
                "" => Ok(QueryField::Value(json!({
                    "type": self.query("/type")?,
                    "tool_name": self.query("/tool_name")?,
                    "call_id": self.query("/call_id")?,
                    "args": self.query("/args")?,
                    "output": self.query("/output")?,
                }))),
                "type" => Ok(QueryField::Value(json!("tool_call"))),
                "tool_name" => Ok(QueryField::Value(json!(tool_name))),
                "call_id" => Ok(QueryField::Value(json!(call_id))),
                "args" => Ok(QueryField::DataQuery(args)),
                "output" => match output {
                    Some(output) => Ok(QueryField::DataQuery(output)),
                    None => Ok(QueryField::Value(json!(null))),
                },
                _ => Err(QueryError::InvalidField(field.to_string())),
            },
        }
    }
}

impl DataQuery for ToolOutput {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        // TODO: Maybe should change storage format to match?
        match field {
            "" => Ok(QueryField::Value(json!({
                "status": self.query("/status")?,
                "content": self.query("/content")?,
            }))),
            "status" => match self {
                Self::Completed { .. } => Ok(QueryField::Value(json!("completed"))),
                Self::Failed { .. } => Ok(QueryField::Value(json!("failed"))),
            },
            "content" => match self {
                ToolOutput::Completed { value } => Ok(QueryField::DataQuery(value)),
                ToolOutput::Failed { error } => Ok(QueryField::Value(json!(error))),
            },
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

fn require(id: i32, field: &str, value: Option<String>) -> AnyResult<String> {
    value.ok_or_else(|| anyhow!("item {id}: {field} is missing"))
}

fn parse_json(id: i32, field: &str, raw: Option<&str>) -> AnyResult<Value> {
    let raw = raw.ok_or_else(|| anyhow!("item {id}: {field} is missing"))?;
    let value = serde_json::from_str(raw)
        .map_err(|e| anyhow!("item {id}: {field} is not valid JSON: {e}"))?;
    Ok(value)
}

fn encode_output(output: &ToolOutput) -> String {
    match output {
        ToolOutput::Completed { value } => json!({ "completed": value }),
        ToolOutput::Failed { error } => json!({ "failed": error }),
    }.to_string()
}

fn decode_output(raw: &str) -> AnyResult<ToolOutput> {
    let value: Value = serde_json::from_str(raw)?;
    if let Some(value) = value.get("completed") {
        Ok(ToolOutput::Completed {
            value: value.clone(),
        })
    } else if let Some(error) = value.get("failed") {
        let error = error
            .as_str()
            .ok_or_else(|| anyhow!("tool output 'error' is not a string"))?;
        Ok(ToolOutput::Failed {
            error: error.to_owned(),
        })
    } else {
        bail!("tool output is missing a 'completed' or 'failed' key")
    }
}

#[cfg(test)]
mod tests {
    use diesel::sqlite::SqliteConnection;
    use serde_json::json;

    use super::*;
    use crate::testing::session_turn;

    /// Creates an item in `session`/`turn` with all optional fields unset.
    fn create_item(
        conn: &mut SqliteConnection,
        session_id: i32,
        turn_id: i32,
        seqno: Option<i64>,
        content: ItemContent,
    ) -> AnyResult<Item> {
        Item::create(
            conn,
            NewItem {
                session_id,
                turn_id,
                response_id: None,
                provider_id: None,
                upstream_id: None,
                seqno,
                content,
            },
        )
    }

    #[test]
    fn test_item_type() {
        assert_eq!(ItemType::from_upstream("message"), Some(ItemType::ResponseText));
        assert_eq!(ItemType::from_upstream("reasoning"), Some(ItemType::Reasoning));
        assert_eq!(ItemType::from_upstream("function_call"), Some(ItemType::ToolCall));
        assert_eq!(ItemType::from_upstream("function_call_output"), None);
        assert_eq!("tool_call".parse::<ItemType>().unwrap(), ItemType::ToolCall);
        assert!("tool_output".parse::<ItemType>().is_err());
        assert!("bogus".parse::<ItemType>().is_err());

        let mut conn = crate::db::open_new().unwrap();
        let (session, turn) = session_turn(&mut conn);
        let item = DbItem::create(
            &mut conn,
            NewDbItem {
                session_id: Some(session.id),
                turn_id: Some(turn.id),
                ty: Some(ItemType::UserText),
                text: Some("hi"),
                ..Default::default()
            },
        ).unwrap();
        let ty = item::table
            .filter(item::id.eq(item.id))
            .select(item::ty)
            .first::<ItemType>(&mut conn)
            .unwrap();
        assert_eq!(ty, ItemType::UserText);
    }

    #[test]
    fn test_item_query() {
        let mut conn = crate::db::open_new().unwrap();
        let (session, turn) = session_turn(&mut conn);
        let item = create_item(
            &mut conn,
            session.id,
            turn.id,
            None,
            ItemContent::UserText("hello".to_owned()),
        ).unwrap();

        assert_eq!(item.query("/id").unwrap(), json!(item.id));
        assert_eq!(item.query("/session_id").unwrap(), json!(session.id));
        assert_eq!(item.query("/turn_id").unwrap(), json!(turn.id));
        assert_eq!(item.query("/seqno").unwrap(), json!(1));
        assert_eq!(item.query("/response_id").unwrap(), json!(null));
        assert_eq!(item.query("/provider_id").unwrap(), json!(null));
        assert_eq!(item.query("/upstream_id").unwrap(), json!(null));
        assert_eq!(
            item.query("/content").unwrap(),
            json!({"type": "user_text", "text": "hello"})
        );

        let whole = json!({
            "id": item.id,
            "session_id": session.id,
            "turn_id": turn.id,
            "response_id": null,
            "provider_id": null,
            "seqno": 1,
            "upstream_id": null,
            "content": {"type": "user_text", "text": "hello"},
        });
        assert_eq!(item.query("/").unwrap(), whole);
        assert_eq!(item.query("query://content/text").unwrap(), json!("hello"));

        assert!(matches!(item.query("/missing"), Err(QueryError::InvalidField(_))));
        assert!(matches!(item.query("/content/missing"), Err(QueryError::InvalidField(_))));
    }

    #[test]
    fn test_tool_call_content_query() {
        let mut conn = crate::db::open_new().unwrap();
        let (session, turn) = session_turn(&mut conn);
        let mut create = |output| {
            create_item(
                &mut conn,
                session.id,
                turn.id,
                None,
                ItemContent::ToolCall(ToolCallContent {
                    tool_name: "read".to_owned(),
                    call_id: "call_1".to_owned(),
                    args: json!({ "path": "a.txt" }),
                    output,
                }),
            ).unwrap()
        };

        let pending = create(None);
        assert_eq!(
            pending.query("/content").unwrap(),
            json!({
                "type": "tool_call",
                "tool_name": "read",
                "call_id": "call_1",
                "args": {"path": "a.txt"},
                "output": null,
            })
        );
        assert_eq!(pending.query("/content/args/path").unwrap(), json!("a.txt"));
        assert_eq!(pending.query("/content/call_id").unwrap(), json!("call_1"));
        assert_eq!(pending.query("/content/output").unwrap(), json!(null));
    }

    #[test]
    fn test_item_round_trip() {
        let mut conn = crate::db::open_new().unwrap();
        let (session, turn) = session_turn(&mut conn);
        let mut create =
            |content| create_item(&mut conn, session.id, turn.id, None, content).unwrap();

        let user_text = create(ItemContent::UserText("hello".to_owned()));
        let response_text = create(ItemContent::ResponseText("hi there".to_owned()));
        let reasoning = create(ItemContent::Reasoning(ReasoningContent {
            text: Some("thinking".to_owned()),
            summary: Some("summarizing".to_owned()),
            encrypted: None,
        }));

        assert_eq!(Item::get(&mut conn, user_text.id).unwrap().as_ref(), Some(&user_text));
        assert_eq!(
            Item::get(&mut conn, response_text.id).unwrap(),
            Some(Item {
                id: response_text.id,
                seqno: 2,
                content: ItemContent::ResponseText("hi there".to_owned()),
                ..user_text
            })
        );
        assert_eq!(
            Item::get(&mut conn, reasoning.id).unwrap(),
            Some(Item {
                id: reasoning.id,
                seqno: 3,
                content: ItemContent::Reasoning(ReasoningContent {
                    text: Some("thinking".to_owned()),
                    summary: Some("summarizing".to_owned()),
                    encrypted: None,
                }),
                ..response_text
            })
        );
    }

    #[test]
    fn test_tool_output_round_trip() {
        /// The tool call under test, with the given output.
        fn call_with(output: Option<ToolOutput>) -> ItemContent {
            ItemContent::ToolCall(ToolCallContent {
                tool_name: "read".to_owned(),
                call_id: "call_1".to_owned(),
                args: json!({ "path": "a.txt" }),
                output,
            })
        }

        let mut conn = crate::db::open_new().unwrap();
        let (session, turn) = session_turn(&mut conn);
        let mut create =
            |content| create_item(&mut conn, session.id, turn.id, None, content).unwrap();

        let mut call = create(call_with(None));
        assert_eq!(Item::get(&mut conn, call.id).unwrap().unwrap(), call);

        let completed = ToolOutput::Completed {
            value: json!({ "contents": "file contents" }),
        };
        call.set_output(&mut conn, completed.clone())
            .unwrap();
        let reloaded = Item::get(&mut conn, call.id).unwrap().unwrap();
        assert_eq!(reloaded.content, call_with(Some(completed)));

        let failed = ToolOutput::Failed {
            error: "file not found".to_owned(),
        };
        call.set_output(&mut conn, failed.clone()).unwrap();
        let reloaded = Item::get(&mut conn, call.id).unwrap().unwrap();
        assert_eq!(reloaded.content, call_with(Some(failed)));
    }

    #[test]
    fn test_invalid_data() {
        let mut conn = crate::db::open_new().unwrap();
        let (session, turn) = session_turn(&mut conn);

        // A tool call with no args cannot be decoded
        let invalid = DbItem::create(
            &mut conn,
            NewDbItem {
                session_id: Some(session.id),
                turn_id: Some(turn.id),
                ty: Some(ItemType::ToolCall),
                text: Some("read"),
                upstream_call_id: Some("call_1"),
                ..Default::default()
            },
        ).unwrap();

        let healthy = create_item(
            &mut conn,
            session.id,
            turn.id,
            None,
            ItemContent::UserText("hello".to_owned()),
        ).unwrap();

        let items = Item::list_by_session(&mut conn, session.id).unwrap();
        let ids: Vec<i32> = items.iter().map(|item| item.id).collect();
        assert_eq!(ids, [healthy.id]);

        assert!(Item::get(&mut conn, invalid.id).unwrap().is_none());
    }

    #[test]
    fn test_seqno() {
        let mut conn = crate::db::open_new().unwrap();
        let (session, turn) = session_turn(&mut conn);
        let mut create = |seqno: i64, text: &str| create_item(
            &mut conn,
            session.id,
            turn.id,
            Some(seqno),
            ItemContent::UserText(text.to_owned()),
        ).unwrap();

        let third = create(30, "third");
        let first = create(10, "first");
        let second = create(20, "second");

        let items = Item::list_by_session(&mut conn, session.id).unwrap();
        let ids: Vec<i32> = items.iter().map(|item| item.id).collect();
        assert_eq!(ids, [first.id, second.id, third.id]);

        let duplicate = create_item(
            &mut conn,
            session.id,
            turn.id,
            Some(20),
            ItemContent::UserText("duplicate".to_owned()),
        );
        assert!(duplicate.is_err());
    }
}
