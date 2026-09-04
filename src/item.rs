use anyhow::{anyhow, bail};
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use log::warn;
use serde_json::{Value, json};

use crate::error::AnyResult;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::schema::item;
use crate::session::{DbItem, ItemType};
use crate::try_nested;

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

/// Item content spread across the columns which store it.
#[derive(Debug, Insertable)]
#[diesel(table_name = item)]
struct NewRow<'a> {
    session_id: i32,
    turn_id: i32,
    response_id: Option<i32>,
    provider_id: Option<i32>,
    ty: ItemType,
    upstream_id: Option<&'a str>,
    upstream_call_id: Option<&'a str>,
    text: Option<&'a str>,
    summary: Option<&'a str>,
    encrypted_text: Option<&'a str>,
    tool_args: Option<String>,
    tool_output: Option<String>,
    seqno: i64,
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
        let mut row = NewRow {
            session_id: new.session_id,
            turn_id: new.turn_id,
            response_id: new.response_id,
            provider_id: new.provider_id,
            ty: ItemType::UserText,
            upstream_id: new.upstream_id.as_deref(),
            upstream_call_id: None,
            text: None,
            summary: None,
            encrypted_text: None,
            tool_args: None,
            tool_output: None,
            seqno: 0,
        };
        match &new.content {
            ItemContent::UserText(text) => {
                row.ty = ItemType::UserText;
                row.text = Some(text);
            }
            ItemContent::ResponseText(text) => {
                row.ty = ItemType::ResponseText;
                row.text = Some(text);
            }
            ItemContent::Reasoning(ReasoningContent {
                text,
                summary,
                encrypted,
            }) => {
                row.ty = ItemType::Reasoning;
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
                row.ty = ItemType::ToolCall;
                row.text = Some(tool_name);
                row.upstream_call_id = Some(call_id);
                row.tool_args = Some(args.to_string());
                row.tool_output = output.as_ref().map(encode_output);
            }
        }
        conn.transaction(|conn| {
            row.seqno = match new.seqno {
                Some(seqno) => seqno,
                None => DbItem::max_seqno(conn, new.session_id)?.unwrap_or(0) + 1,
            };
            let row = diesel::insert_into(item::table)
                .values(row)
                .returning(item::all_columns)
                .get_result::<DbItem>(conn)?;
            Ok(Self::from_row(&row).unwrap())
        })
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
    use crate::session::{ItemType, NewItem as NewDbItem};
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
