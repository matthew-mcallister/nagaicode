use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::error::AnyResult;
use crate::schema::{chain, content, item, session};

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

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = item)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Item {
    pub id: i32,
    pub session_id: i32,
    pub chain_id: Option<i32>,
    pub r#type: String,
    pub response_id: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = item)]
pub struct NewItem<'a> {
    pub session_id: i32,
    pub chain_id: Option<i32>,
    pub r#type: &'a str,
    pub response_id: Option<&'a str>,
}

impl Item {
    pub fn create(
        conn: &mut SqliteConnection,
        session_id: i32,
        chain_id: Option<i32>,
        r#type: &str,
        response_id: Option<&str>,
    ) -> AnyResult<Item> {
        let new = NewItem {
            session_id,
            chain_id,
            r#type,
            response_id,
        };
        let item = diesel::insert_into(item::table)
            .values(&new)
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
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = content)]
#[diesel(check_for_backend(diesel::sqlite::Sqlite))]
pub struct Content {
    pub id: i32,
    pub item_id: i32,
    pub r#type: String,
    pub value: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = content)]
pub struct NewContent<'a> {
    pub item_id: i32,
    pub r#type: &'a str,
    pub value: &'a str,
}

impl Content {
    pub fn create(
        conn: &mut SqliteConnection,
        item_id: i32,
        r#type: &str,
        value: &str,
    ) -> AnyResult<Content> {
        let new = NewContent {
            item_id,
            r#type,
            value,
        };
        let content = diesel::insert_into(content::table)
            .values(&new)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_crud() {
        let mut conn = crate::db::open_in_memory().expect("failed to open in-memory db");

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
        let mut conn = crate::db::open_in_memory().expect("failed to open in-memory db");
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
        let mut conn = crate::db::open_in_memory().expect("failed to open in-memory db");
        let session = Session::create(&mut conn, "Chat").expect("create session");
        let chain = Chain::create(&mut conn, session.id, 1, "openai", "gpt-4o").expect("create chain");

        // Create item with chain_id and response_id
        let item1 = Item::create(&mut conn, session.id, Some(chain.id), "user_message", None)
            .expect("create item1");
        assert_eq!(item1.session_id, session.id);
        assert_eq!(item1.chain_id, Some(chain.id));
        assert_eq!(item1.r#type, "user_message");
        assert_eq!(item1.response_id, None);

        // Create item without chain_id but with response_id
        let item2 = Item::create(&mut conn, session.id, None, "assistant_message", Some("resp-123"))
            .expect("create item2");
        assert_eq!(item2.chain_id, None);
        assert_eq!(item2.response_id, Some("resp-123".to_string()));

        let session_items = Item::list_by_session(&mut conn, session.id).expect("list session items");
        assert_eq!(session_items.len(), 2);

        let chain_items = Item::list_by_chain(&mut conn, chain.id).expect("list chain items");
        assert_eq!(chain_items.len(), 1);
        assert_eq!(chain_items[0].id, item1.id);

        // Create content for item1
        let c1 = Content::create(&mut conn, item1.id, "text", "Hello world").expect("create content 1");
        let c2 = Content::create(&mut conn, item1.id, "text", "Another part").expect("create content 2");
        assert_eq!(c1.item_id, item1.id);
        assert_eq!(c1.r#type, "text");
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
        let item3 = Item::create(&mut conn, session.id, Some(chain_for_item3.id), "msg", None).expect("create item3");
        Chain::delete_by_id(&mut conn, chain_for_item3.id).expect("delete chain");
        let item3_refetched = Item::get_by_id(&mut conn, item3.id).unwrap().expect("item3 exists");
        assert_eq!(item3_refetched.chain_id, None, "chain_id should be SET NULL when chain is deleted");

        // Deleting session cascades to items
        Session::delete_by_id(&mut conn, session.id).expect("delete session");
        assert!(Item::get_by_id(&mut conn, item2.id).unwrap().is_none());
        assert!(Item::get_by_id(&mut conn, item3.id).unwrap().is_none());
    }
}
