use anyhow::anyhow;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::session::{Item, ItemType};
use crate::task::{Task, TaskContext};
use crate::tool::ToolServer;

/// Executes a tool call item and stores its output on the item.
pub struct ExecuteToolCall {
    tool_call: Item,
}

impl ExecuteToolCall {
    /// Creates a task that executes `tool_call` and stores its output.
    pub fn new(tool_call: Item) -> Self {
        Self { tool_call }
    }

    async fn process(self, context: &mut TaskContext) -> AnyResult<()> {
        if self.tool_call.ty()? != ItemType::ToolCall {
            anyhow::bail!("item {} is not a tool call", self.tool_call.id);
        }
        let name = self
            .tool_call
            .text
            .as_deref()
            .ok_or_else(|| anyhow!("tool call item {} has no tool name", self.tool_call.id))?;
        let args = self.tool_call.tool_args_json()?.unwrap_or_default();

        let result = context.tools_mut().call(name, args).await;

        let mut tool_call = self.tool_call;
        tool_call.set_tool_output(context.connection()?, &result)?;
        context.send(AppEvent::ItemUpdated { item: tool_call });
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
    use crate::tool::mock::ToolCall;

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
                ..Default::default()
            },
        ).unwrap();
        Item::update_tool_args(&mut conn, tool_call.id, r#"{"a":1,"b":2}"#).unwrap();

        let echo_tool_call = Item::create(
            &mut conn,
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(turn.id),
                ty: Some(ItemType::ToolCall),
                upstream_id: Some("fc_2"),
                upstream_type: Some("function_call"),
                upstream_call_id: Some("call_2"),
                text: Some("echo"),
                ..Default::default()
            },
        ).unwrap();
        Item::update_tool_args(&mut conn, echo_tool_call.id, r#"{"message":"hi"}"#).unwrap();

        let tool_call = Item::get_by_id(&mut conn, tool_call.id)
            .unwrap()
            .unwrap();
        let echo_tool_call = Item::get_by_id(&mut conn, echo_tool_call.id)
            .unwrap()
            .unwrap();

        app.tools_mut().add_result("add", json!({"result": 3}));
        app.tools_mut().add_result("echo", json!({"reply": "hello"}));

        let mut context = app.context();
        ExecuteToolCall::new(tool_call.clone())
            .run(&mut context)
            .await
            .unwrap();
        ExecuteToolCall::new(echo_tool_call.clone())
            .run(&mut context)
            .await
            .unwrap();

        let calls = app.tools().get_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], ToolCall::new("add", json!({"a": 1, "b": 2})));
        assert_eq!(calls[1], ToolCall::new("echo", json!({"message": "hi"})));

        let items = Item::list_by_session(&mut conn, session.id).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, tool_call.id);
        assert_eq!(items[1].id, echo_tool_call.id);
        assert_eq!(items[0].tool_output().unwrap(), Some(json!({"result": 3})));
        assert_eq!(items[1].tool_output().unwrap(), Some(json!({"reply": "hello"})));

        let events = app.drain_events();
        for event in events {
            match event {
                AppEvent::ItemUpdated { item } => {
                    assert_eq!(item.ty, "tool_call");
                    assert!(item.tool_output().unwrap().is_some());
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }
}
