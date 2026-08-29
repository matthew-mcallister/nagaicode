use anyhow::anyhow;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::session::{Item, ItemType};
use crate::tasks::{Task, TaskContext};
use crate::tools::{ToolResult, ToolServer};

/// Executes a tool call item and completes its pre-created tool output item.
pub struct ExecuteToolCall {
    tool_call: Item,
    output: Item,
}

impl ExecuteToolCall {
    /// Creates a task that executes `tool_call` and completes `output`.
    pub fn new(tool_call: Item, output: Item) -> Self {
        Self { tool_call, output }
    }

    async fn process(self, context: &mut TaskContext) -> AnyResult<()> {
        if self.tool_call.ty()? != ItemType::ToolCall {
            anyhow::bail!("item {} is not a tool call", self.tool_call.id);
        }
        if self.output.ty()? != ItemType::ToolOutput {
            anyhow::bail!("item {} is not a tool output", self.output.id);
        }
        let name = self
            .tool_call
            .text
            .as_deref()
            .ok_or_else(|| anyhow!("tool call item {} has no tool name", self.tool_call.id))?;
        let args = self.tool_call.json()?.unwrap_or_default();

        let result = context.tools_mut().call(name, args).await;

        let (text, json) = match &result {
            ToolResult::Text(text) => (Some(text.clone()), None),
            ToolResult::Json(json) => (None, Some(json.to_string())),
        };
        let mut output = self.output;
        output.complete_output(context.connection()?, text, json)?;
        context.send(AppEvent::ItemUpdated { item: output });
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
    use crate::session::{NewItem, Session, Turn, TurnType};
    use crate::tools::mock::ToolCall;
    use diesel::sqlite::SqliteConnection;

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

        let tool_call = Item::get_by_id(&mut conn, tool_call.id)
            .unwrap()
            .unwrap();
        let text_tool_call = Item::get_by_id(&mut conn, text_tool_call.id)
            .unwrap()
            .unwrap();

        let make_output = |conn: &mut SqliteConnection, call_id: &str| {
            Item::create(
                conn,
                NewItem {
                    session_id: Some(session.id),
                    turn_id: Some(turn.id),
                    ty: Some(ItemType::ToolOutput),
                    upstream_type: Some("function_call_output"),
                    upstream_call_id: Some(call_id),
                    completed: Some(false),
                    ..Default::default()
                },
            )
            .unwrap()
        };
        let output = make_output(&mut conn, "call_1");
        let text_output = make_output(&mut conn, "call_2");
        assert!(!output.completed);
        assert!(!text_output.completed);

        app.tools_mut().add_result("add", ToolResult::Json(json!({"result": 3})));
        app.tools_mut().add_result("echo", ToolResult::Text("hello".to_owned()));

        let mut context = app.context();
        ExecuteToolCall::new(tool_call.clone(), output.clone())
            .run(&mut context)
            .await
            .unwrap();
        ExecuteToolCall::new(text_tool_call.clone(), text_output.clone())
            .run(&mut context)
            .await
            .unwrap();

        let calls = app.tools().get_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], ToolCall::new("add", json!({"a": 1, "b": 2})));
        assert_eq!(calls[1], ToolCall::new("echo", json!({"message": "hi"})));

        let items = Item::list_by_session(&mut conn, session.id).unwrap();
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].id, tool_call.id);
        assert_eq!(items[1].id, text_tool_call.id);
        assert_eq!(items[2].id, output.id);
        assert_eq!(items[3].id, text_output.id);
        assert!(items[2].completed);
        assert_eq!(items[2].upstream_call_id.as_deref(), Some("call_1"));
        assert_eq!(items[2].upstream_type.as_deref(), Some("function_call_output"));
        assert_eq!(items[2].json.as_deref(), Some(r#"{"result":3}"#));
        assert_eq!(items[2].text, None);
        assert!(items[3].completed);
        assert_eq!(items[3].upstream_call_id.as_deref(), Some("call_2"));
        assert_eq!(items[3].text.as_deref(), Some("hello"));
        assert_eq!(items[3].json, None);

        let events = app.drain_events();
        for event in events {
            match event {
                AppEvent::ItemUpdated { item } => {
                    assert_eq!(item.ty, "tool_output");
                    assert!(item.completed);
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }
}
