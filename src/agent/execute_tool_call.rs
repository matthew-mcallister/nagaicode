use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::session::DbItem;
use crate::task::{Task, TaskContext};

/// Executes a tool call item and stores its output on the item.
pub struct ExecuteToolCall {
    tool_call: DbItem,
}

impl ExecuteToolCall {
    /// Creates a task that executes `tool_call` and stores its output.
    pub fn new(tool_call: DbItem) -> Self {
        Self { tool_call }
    }

    async fn process(mut self, context: &mut TaskContext) -> AnyResult<()> {
        let result = context.tool_registry().call(&mut self.tool_call).await;
        self.tool_call.set_tool_output(context.connection()?, &result)?;
        context.send(AppEvent::ItemUpdated { item: self.tool_call });
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
    use crate::testing::{session_turn, tool_call};

    #[tokio::test]
    async fn test_execute_tool_call() {
        let mut app = App::new().unwrap();
        let mut conn = db::open(app.db_url()).unwrap();
        let (session, turn) = session_turn(&mut conn);

        // Tools run on a live shell.
        let sh = |command| json!({ "command": command });
        let stdout_call = tool_call(&mut conn, &turn, "sh", sh("printf 'hi'"), None);
        let stderr_call = tool_call(&mut conn, &turn, "sh", sh("printf 'bye' >&2"), None);
        // Failed calls record their error as output.
        let bad_args_call = tool_call(&mut conn, &turn, "sh", json!({ "command": 123 }), None);
        let unknown_call = tool_call(&mut conn, &turn, "no_such_tool", json!({}), None);

        let mut context = app.context();
        for item in [stdout_call, stderr_call, bad_args_call, unknown_call] {
            ExecuteToolCall::new(item).run(&mut context).await.unwrap();
        }

        let items = DbItem::list_by_session(&mut conn, session.id).unwrap();
        assert_eq!(items.len(), 4);
        assert_eq!(
            items[0].tool_output().unwrap(),
            Some(json!({ "stdout": "hi", "stderr": "", "return_code": 0 }))
        );
        assert_eq!(
            items[1].tool_output().unwrap(),
            Some(json!({ "stdout": "", "stderr": "bye", "return_code": 0 }))
        );
        assert_eq!(
            items[2].tool_output().unwrap(),
            Some(json!({
                "tool_name": "sh",
                "error": "invalid arguments for 'sh': expected {\"command\": \"...\"}",
            }))
        );
        assert_eq!(
            items[3].tool_output().unwrap(),
            Some(json!({ "tool_name": "no_such_tool", "error": "unknown tool" }))
        );

        let events = app.drain_events();
        assert_eq!(events.len(), 4);
        for (event, item) in events.into_iter().zip(&items) {
            match event {
                AppEvent::ItemUpdated { item: updated } => {
                    assert_eq!(updated.id, item.id);
                    assert_eq!(updated.ty, "tool_call");
                    assert_eq!(updated.tool_output, item.tool_output);
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }
}
