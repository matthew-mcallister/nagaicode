use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::error::AnyResult;
use crate::query::{DataQuery, QueryError, QueryField, datetime_to_json};
use crate::schema::{chain, content, item, session};
use serde_json::json;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ContentType {
    Thought,
    Text,
}

impl fmt::Display for ContentType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContentType::Thought => write!(f, "thought"),
            ContentType::Text => write!(f, "text"),
        }
    }
}

impl FromStr for ContentType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "thought" => Ok(ContentType::Thought),
            "text" => Ok(ContentType::Text),
            other => Err(format!("unknown content type: '{other}'")),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ItemType {
    User,
    Model,
}

impl fmt::Display for ItemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ItemType::User => write!(f, "user"),
            ItemType::Model => write!(f, "model"),
        }
    }
}

impl FromStr for ItemType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(ItemType::User),
            "model" => Ok(ItemType::Model),
            other => Err(format!("unknown item type: {other}")),
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
                "created_at": datetime_to_json(self.created_at),
                "updated_at": datetime_to_json(self.updated_at),
            }))),
            "id" => Ok(QueryField::Value(json!(self.id))),
            "name" => Ok(QueryField::Value(json!(self.name))),
            "created_at" => Ok(QueryField::Value(datetime_to_json(self.created_at))),
            "updated_at" => Ok(QueryField::Value(datetime_to_json(self.updated_at))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = chain)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Chain {
    pub id: i32,
    pub session_id: i32,
    pub provider_id: i32,
    pub provider_name: String,
    pub model_id: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = chain)]
pub struct NewChain<'a> {
    pub session_id: i32,
    pub provider_id: i32,
    pub provider_name: &'a str,
    pub model_id: &'a str,
}

impl Chain {
    pub fn create(
        conn: &mut SqliteConnection,
        session_id: i32,
        provider_id: i32,
        provider_name: &str,
        model_id: &str,
    ) -> AnyResult<Chain> {
        let new = NewChain {
            session_id,
            provider_id,
            provider_name,
            model_id,
        };
        let chain = diesel::insert_into(chain::table)
            .values(&new)
            .returning(chain::all_columns)
            .get_result(conn)?;
        Ok(chain)
    }

    pub fn get_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<Option<Chain>> {
        let result = chain::table
            .filter(chain::id.eq(id))
            .first::<Chain>(conn)
            .optional()?;
        Ok(result)
    }

    pub fn list_by_session(conn: &mut SqliteConnection, session_id: i32) -> AnyResult<Vec<Chain>> {
        let chains = chain::table
            .filter(chain::session_id.eq(session_id))
            .order(chain::id.asc())
            .load::<Chain>(conn)?;
        Ok(chains)
    }

    pub fn delete_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<bool> {
        let count = diesel::delete(chain::table.filter(chain::id.eq(id))).execute(conn)?;
        Ok(count > 0)
    }
}

/// Exposed fields:
/// - id: number
/// - session_id: number
/// - provider_id: number
/// - provider_name: string
/// - model_id: string
/// - created_at: string (ISO 8601)
/// - updated_at: string (ISO 8601)
impl DataQuery for Chain {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "id": self.id,
                "session_id": self.session_id,
                "provider_id": self.provider_id,
                "provider_name": self.provider_name,
                "model_id": self.model_id,
                "created_at": datetime_to_json(self.created_at),
                "updated_at": datetime_to_json(self.updated_at),
            }))),
            "id" => Ok(QueryField::Value(json!(self.id))),
            "session_id" => Ok(QueryField::Value(json!(self.session_id))),
            "provider_id" => Ok(QueryField::Value(json!(self.provider_id))),
            "provider_name" => Ok(QueryField::Value(json!(self.provider_name))),
            "model_id" => Ok(QueryField::Value(json!(self.model_id))),
            "created_at" => Ok(QueryField::Value(datetime_to_json(self.created_at))),
            "updated_at" => Ok(QueryField::Value(datetime_to_json(self.updated_at))),
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
    pub chain_id: Option<i32>,
    #[diesel(column_name = "type")]
    pub ty: String,
    pub response_id: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug)]
pub struct NewItem<'a> {
    pub session_id: i32,
    pub chain_id: Option<i32>,
    pub ty: ItemType,
    pub response_id: Option<&'a str>,
}

impl<'a> diesel::insertable::Insertable<item::table> for NewItem<'a> {
    type Values = <(
        Option<diesel::dsl::Eq<item::session_id, i32>>,
        Option<diesel::dsl::Eq<item::chain_id, i32>>,
        Option<diesel::dsl::Eq<item::r#type, String>>,
        Option<diesel::dsl::Eq<item::response_id, &'a str>>,
    ) as diesel::insertable::Insertable<item::table>>::Values;

    fn values(self) -> Self::Values {
        diesel::insertable::Insertable::<item::table>::values((
            Some(item::session_id.eq(self.session_id)),
            self.chain_id.map(|x| item::chain_id.eq(x)),
            Some(item::r#type.eq(self.ty.to_string())),
            self.response_id.map(|x| item::response_id.eq(x)),
        ))
    }
}

impl Item {
    pub fn create(
        conn: &mut SqliteConnection,
        session_id: i32,
        chain_id: Option<i32>,
        ty: ItemType,
        response_id: Option<&str>,
    ) -> AnyResult<Item> {
        let new = NewItem {
            session_id,
            chain_id,
            ty,
            response_id,
        };
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

    pub fn list_by_chain(conn: &mut SqliteConnection, chain_id: i32) -> AnyResult<Vec<Item>> {
        let items = item::table
            .filter(item::chain_id.eq(chain_id))
            .order(item::id.asc())
            .load::<Item>(conn)?;
        Ok(items)
    }

    pub fn delete_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<bool> {
        let count = diesel::delete(item::table.filter(item::id.eq(id))).execute(conn)?;
        Ok(count > 0)
    }

    pub fn ty(&self) -> AnyResult<ItemType> {
        Ok(ItemType::from_str(&self.ty)?)
    }
}

/// Exposed fields:
/// - id: number
/// - session_id: number
/// - chain_id: number | null
/// - ty: string
/// - response_id: string | null
/// - created_at: string (ISO 8601)
/// - updated_at: string (ISO 8601)
impl DataQuery for Item {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "id": self.id,
                "session_id": self.session_id,
                "chain_id": self.chain_id,
                "ty": self.ty,
                "response_id": self.response_id,
                "created_at": datetime_to_json(self.created_at),
                "updated_at": datetime_to_json(self.updated_at),
            }))),
            "id" => Ok(QueryField::Value(json!(self.id))),
            "session_id" => Ok(QueryField::Value(json!(self.session_id))),
            "chain_id" => Ok(QueryField::Value(json!(self.chain_id))),
            "ty" => Ok(QueryField::Value(json!(self.ty))),
            "response_id" => Ok(QueryField::Value(json!(self.response_id))),
            "created_at" => Ok(QueryField::Value(datetime_to_json(self.created_at))),
            "updated_at" => Ok(QueryField::Value(datetime_to_json(self.updated_at))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Queryable, Selectable)]
#[diesel(table_name = content)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Content {
    pub id: i32,
    pub item_id: i32,
    #[diesel(column_name = "type")]
    pub ty: String,
    pub value: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug)]
pub struct NewContent<'a> {
    pub item_id: i32,
    pub ty: ContentType,
    pub value: &'a str,
}

impl<'a> diesel::insertable::Insertable<content::table> for NewContent<'a> {
    type Values = <(
        Option<diesel::dsl::Eq<content::item_id, i32>>,
        Option<diesel::dsl::Eq<content::r#type, String>>,
        Option<diesel::dsl::Eq<content::value, &'a str>>,
    ) as diesel::insertable::Insertable<content::table>>::Values;

    fn values(self) -> Self::Values {
        diesel::insertable::Insertable::<content::table>::values((
            Some(content::item_id.eq(self.item_id)),
            Some(content::r#type.eq(self.ty.to_string())),
            Some(content::value.eq(self.value)),
        ))
    }
}

impl Content {
    pub fn create(
        conn: &mut SqliteConnection,
        item_id: i32,
        ty: ContentType,
        value: &str,
    ) -> AnyResult<Content> {
        let new = NewContent {
            item_id,
            ty,
            value,
        };
        let content = diesel::insert_into(content::table)
            .values(new)
            .returning(content::all_columns)
            .get_result(conn)?;
        Ok(content)
    }

    pub fn get_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<Option<Content>> {
        let result = content::table
            .filter(content::id.eq(id))
            .first::<Content>(conn)
            .optional()?;
        Ok(result)
    }

    pub fn list_by_item(conn: &mut SqliteConnection, item_id: i32) -> AnyResult<Vec<Content>> {
        let contents = content::table
            .filter(content::item_id.eq(item_id))
            .order(content::id.asc())
            .load::<Content>(conn)?;
        Ok(contents)
    }

    pub fn delete_by_id(conn: &mut SqliteConnection, id: i32) -> AnyResult<bool> {
        let count = diesel::delete(content::table.filter(content::id.eq(id))).execute(conn)?;
        Ok(count > 0)
    }

    pub fn ty(&self) -> AnyResult<ContentType> {
        Ok(ContentType::from_str(&self.ty)?)
    }
}

/// Exposed fields:
/// - id: number
/// - item_id: number
/// - ty: string
/// - value: string
/// - created_at: string (ISO 8601)
/// - updated_at: string (ISO 8601)
impl DataQuery for Content {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "id": self.id,
                "item_id": self.item_id,
                "ty": self.ty,
                "value": self.value,
                "created_at": datetime_to_json(self.created_at),
                "updated_at": datetime_to_json(self.updated_at),
            }))),
            "id" => Ok(QueryField::Value(json!(self.id))),
            "item_id" => Ok(QueryField::Value(json!(self.item_id))),
            "ty" => Ok(QueryField::Value(json!(self.ty))),
            "value" => Ok(QueryField::Value(json!(self.value))),
            "created_at" => Ok(QueryField::Value(datetime_to_json(self.created_at))),
            "updated_at" => Ok(QueryField::Value(datetime_to_json(self.updated_at))),
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
    fn test_chain_crud_and_cascade() {
        let mut conn = crate::db::open_new().expect("failed to open in-memory db");
        let session = Session::create(&mut conn, "Test Session").expect("create session");

        // Note: provider_id 999 does not exist in provider table, proving provider_id is *not* a foreign key
        let chain = Chain::create(&mut conn, session.id, 999, "openai", "gpt-4o")
            .expect("create chain");
        assert_eq!(chain.session_id, session.id);
        assert_eq!(chain.provider_id, 999);
        assert_eq!(chain.provider_name, "openai");
        assert_eq!(chain.model_id, "gpt-4o");

        let fetched = Chain::get_by_id(&mut conn, chain.id)
            .expect("get chain")
            .expect("chain not found");
        assert_eq!(fetched.id, chain.id);

        let chains = Chain::list_by_session(&mut conn, session.id).expect("list chains");
        assert_eq!(chains.len(), 1);
        assert_eq!(chains[0].id, chain.id);

        // Delete chain directly
        let deleted = Chain::delete_by_id(&mut conn, chain.id).expect("delete chain");
        assert!(deleted);
        assert!(Chain::get_by_id(&mut conn, chain.id).unwrap().is_none());

        // Re-create chain and test cascade deletion with session
        let chain2 = Chain::create(&mut conn, session.id, 999, "openai", "gpt-4o")
            .expect("create chain2");
        Session::delete_by_id(&mut conn, session.id).expect("delete session");
        assert!(
            Chain::get_by_id(&mut conn, chain2.id).unwrap().is_none(),
            "chain should cascade delete when session is deleted"
        );
    }

    #[test]
    fn test_item_and_content_crud_and_cascade() {
        let mut conn = crate::db::open_new().expect("failed to open in-memory db");
        let session = Session::create(&mut conn, "Chat").expect("create session");
        let chain = Chain::create(&mut conn, session.id, 1, "openai", "gpt-4o").expect("create chain");

        // Create item with chain_id and response_id
        let item1 = Item::create(&mut conn, session.id, Some(chain.id), ItemType::User, None)
            .expect("create item1");
        assert_eq!(item1.session_id, session.id);
        assert_eq!(item1.chain_id, Some(chain.id));
        assert_eq!(item1.ty, "user");
        assert_eq!(item1.response_id, None);

        // Create item without chain_id but with response_id
        let item2 = Item::create(&mut conn, session.id, None, ItemType::Model, Some("resp-123"))
            .expect("create item2");
        assert_eq!(item2.chain_id, None);
        assert_eq!(item2.response_id, Some("resp-123".to_string()));

        let session_items = Item::list_by_session(&mut conn, session.id).expect("list session items");
        assert_eq!(session_items.len(), 2);

        let chain_items = Item::list_by_chain(&mut conn, chain.id).expect("list chain items");
        assert_eq!(chain_items.len(), 1);
        assert_eq!(chain_items[0].id, item1.id);

        // Create content for item1
        let c1 = Content::create(&mut conn, item1.id, ContentType::Text, "Hello world").expect("create content 1");
        let c2 = Content::create(&mut conn, item1.id, ContentType::Text, "Another part").expect("create content 2");
        assert_eq!(c1.item_id, item1.id);
        assert_eq!(c1.ty, "text");
        assert_eq!(c1.value, "Hello world");

        let contents = Content::list_by_item(&mut conn, item1.id).expect("list contents");
        assert_eq!(contents.len(), 2);

        // Deleting content directly
        let deleted_c = Content::delete_by_id(&mut conn, c1.id).expect("delete content");
        assert!(deleted_c);
        assert!(Content::get_by_id(&mut conn, c1.id).unwrap().is_none());

        // Deleting item cascades to remaining content
        let deleted_item = Item::delete_by_id(&mut conn, item1.id).expect("delete item");
        assert!(deleted_item);
        assert!(Content::get_by_id(&mut conn, c2.id).unwrap().is_none());

        // Test ON DELETE SET NULL on chain_id when chain is deleted
        let chain_for_item3 = Chain::create(&mut conn, session.id, 1, "openai", "gpt-4o").expect("create chain");
        let item3 = Item::create(&mut conn, session.id, Some(chain_for_item3.id), ItemType::User, None).expect("create item3");
        Chain::delete_by_id(&mut conn, chain_for_item3.id).expect("delete chain");
        let item3_refetched = Item::get_by_id(&mut conn, item3.id).unwrap().expect("item3 exists");
        assert_eq!(item3_refetched.chain_id, None, "chain_id should be SET NULL when chain is deleted");

        // Deleting session cascades to items
        Session::delete_by_id(&mut conn, session.id).expect("delete session");
        assert!(Item::get_by_id(&mut conn, item2.id).unwrap().is_none());
        assert!(Item::get_by_id(&mut conn, item3.id).unwrap().is_none());
    }

    #[test]
    fn test_data_query() {
        use serde_json::json;

        let mut conn = crate::db::open_new().expect("failed to open in-memory db");

        let session = Session::create(&mut conn, "Session 1").expect("create session failed");
        assert_eq!(session.query("/").unwrap(), json!({
            "id": session.id,
            "name": session.name,
            "created_at": datetime_to_json(session.created_at),
            "updated_at": datetime_to_json(session.updated_at),
        }));
        assert_eq!(session.query("/id").unwrap(), json!(session.id));
        assert_eq!(session.query("/name").unwrap(), json!(session.name));
        assert_eq!(session.query("/created_at").unwrap(), datetime_to_json(session.created_at));
        assert_eq!(session.query("/updated_at").unwrap(), datetime_to_json(session.updated_at));

        let chain = Chain::create(&mut conn, session.id, 1, "openai", "gpt-4o").expect("create chain failed");
        assert_eq!(chain.query("/").unwrap(), json!({
            "id": chain.id,
            "session_id": chain.session_id,
            "provider_id": chain.provider_id,
            "provider_name": chain.provider_name,
            "model_id": chain.model_id,
            "created_at": datetime_to_json(chain.created_at),
            "updated_at": datetime_to_json(chain.updated_at),
        }));
        assert_eq!(chain.query("/provider_name").unwrap(), json!(chain.provider_name));
        assert_eq!(chain.query("/model_id").unwrap(), json!(chain.model_id));

        let item = Item::create(&mut conn, session.id, Some(chain.id), ItemType::User, Some("resp-1"))
            .expect("create item failed");
        assert_eq!(item.query("/").unwrap(), json!({
            "id": item.id,
            "session_id": item.session_id,
            "chain_id": item.chain_id,
            "ty": item.ty,
            "response_id": item.response_id,
            "created_at": datetime_to_json(item.created_at),
            "updated_at": datetime_to_json(item.updated_at),
        }));
        assert_eq!(item.query("/chain_id").unwrap(), json!(item.chain_id));
        assert_eq!(item.query("/ty").unwrap(), json!(item.ty));
        assert_eq!(item.query("/response_id").unwrap(), json!(item.response_id));

        let content = Content::create(&mut conn, item.id, ContentType::Text, "Hello world")
            .expect("create content failed");
        assert_eq!(content.query("/").unwrap(), json!({
            "id": content.id,
            "item_id": content.item_id,
            "ty": content.ty,
            "value": content.value,
            "created_at": datetime_to_json(content.created_at),
            "updated_at": datetime_to_json(content.updated_at),
        }));
        assert_eq!(content.query("/item_id").unwrap(), json!(content.item_id));
        assert_eq!(content.query("/ty").unwrap(), json!(content.ty));
        assert_eq!(content.query("/value").unwrap(), json!(content.value));
    }
}
