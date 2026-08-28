use anyhow::anyhow;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::session::{Item, ItemType, NewItem};
use crate::tasks::{Task, TaskContext};
use crate::tools::{ToolResult, ToolServer};

/// Executes a tool call item and commits a matching tool output item.
pub struct ExecuteToolCall {
    item_id: i32,
}

impl ExecuteToolCall {
    /// Creates a task that executes the tool call item identified by `item_id`.
    pub fn new(item_id: i32) -> Self {
        Self { item_id }
    }

    async fn process(self, context: &mut TaskContext) -> AnyResult<()> {
        let conn = context.connection()?;
        let item = Item::get_by_id(conn, self.item_id)?
            .ok_or_else(|| anyhow!("tool call item {} not found", self.item_id))?;
        if item.ty()? != ItemType::ToolCall {
            anyhow::bail!("item {} is not a tool call", self.item_id);
        }
        let name = item
            .text
            .as_deref()
            .ok_or_else(|| anyhow!("tool call item {} has no tool name", self.item_id))?;
        let args = item.json()?.unwrap_or_default();
        let call_id = item.upstream_call_id.as_deref();

        let result = context.tools_mut().call(name, args).await;

        let (text, json) = match &result {
            ToolResult::Text(text) => (Some(text.as_str()), None),
            ToolResult::Json(json) => (None, Some(json.to_string())),
        };
        let conn = context.connection()?;
        let output = Item::create(
            conn,
            NewItem {
                session_id: Some(item.session_id),
                turn_id: Some(item.turn_id),
                response_id: None,
                provider_id: item.provider_id,
                ty: Some(ItemType::ToolOutput),
                upstream_id: None,
                upstream_type: Some("function_call_output"),
                upstream_call_id: call_id,
                text,
                seqno: None,
                completed: Some(true),
            },
        )?;
        if let Some(json) = json {
            Item::update_json(conn, output.id, &json)?;
        }
        context.send(AppEvent::ItemCreated { item: output });
        Ok(())
    }
}

impl Task for ExecuteToolCall {
    type Output = AnyResult<()>;

    async fn run(self, context: &mut TaskContext) -> AnyResult<()> {
        self.process(context).await
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::app::App;
    use crate::db;
    use crate::session::{Session, Turn, TurnType};
    use crate::tools::mock::ToolCall;

    #[tokio::test]
    async fn test_execute_tool_call() {
        let mut app = App::new().unwrap();
        let mut conn = db::open(app.db_url()).unwrap();
        let session = Session::create(&mut conn, "Session").unwrap();
        let turn = Turn::create(&mut conn, session.id, TurnType::Assistant, None, None, None)
            .unwrap();

        let tool_call = Item::create(
            &mut conn,
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(turn.id),
                ty: Some(ItemType::ToolCall),
                upstream_id: Some("fc_1"),
                upstream_type: Some("function_call"),
                upstream_call_id: Some("call_1"),
                text: Some("add"),
                completed: Some(true),
                ..Default::default()
            },
        ).unwrap();
        Item::update_json(&mut conn, tool_call.id, r#"{"a":1,"b":2}"#).unwrap();

        let text_tool_call = Item::create(
            &mut conn,
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(turn.id),
                ty: Some(ItemType::ToolCall),
                upstream_id: Some("fc_2"),
                upstream_type: Some("function_call"),
                upstream_call_id: Some("call_2"),
                text: Some("echo"),
                completed: Some(true),
                ..Default::default()
            },
        ).unwrap();
        Item::update_json(&mut conn, text_tool_call.id, r#"{"message":"hi"}"#).unwrap();

        app.tools_mut().add_result("add", ToolResult::Json(json!({"result": 3})));
        app.tools_mut().add_result("echo", ToolResult::Text("hello".to_owned()));

        let mut context = app.context();
        ExecuteToolCall::new(tool_call.id).run(&mut context).await.unwrap();
        ExecuteToolCall::new(text_tool_call.id).run(&mut context).await.unwrap();

        let calls = app.tools().get_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], ToolCall::new("add", json!({"a": 1, "b": 2})));
        assert_eq!(calls[1], ToolCall::new("echo", json!({"message": "hi"})));

        let items = Item::list_by_session(&mut conn, session.id).unwrap();
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].id, tool_call.id);
        assert_eq!(items[1].id, text_tool_call.id);
        assert_eq!(items[2].ty().unwrap(), ItemType::ToolOutput);
        assert_eq!(items[2].upstream_call_id.as_deref(), Some("call_1"));
        assert_eq!(items[2].upstream_type.as_deref(), Some("function_call_output"));
        assert_eq!(items[2].json.as_deref(), Some(r#"{"result":3}"#));
        assert_eq!(items[2].text, None);
        assert_eq!(items[3].ty().unwrap(), ItemType::ToolOutput);
        assert_eq!(items[3].upstream_call_id.as_deref(), Some("call_2"));
        assert_eq!(items[3].text.as_deref(), Some("hello"));
        assert_eq!(items[3].json, None);

        let events = app.drain_events();
        for event in events {
            match event {
                AppEvent::ItemCreated { item } => assert_eq!(item.ty, "tool_output"),
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }
}
