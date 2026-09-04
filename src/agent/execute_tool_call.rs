use anyhow::anyhow;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::item::Item;
use crate::session::DbItem;
use crate::task::{Task, TaskContext};

pub struct ExecuteToolCall {
    item: Item,
}

impl ExecuteToolCall {
    pub fn new(item: Item) -> Self {
        Self { item }
    }

    async fn process(mut self, context: &mut TaskContext) -> AnyResult<()> {
        let tc = self.item.content.as_tool_call().expect("tried to execute non-tool-call");
        let output = context.tool_registry().call(&tc.tool_name, &tc.args).await;
        self.item.set_output(context.connection()?, output)?;

        // FIXME: Stop sending DbItemUpdated
        let id = self.item.id;
        let row = DbItem::get_by_id(context.connection()?, id)?
            .ok_or_else(|| anyhow!("item {id} is missing"))?;
        context.send(AppEvent::DbItemUpdated { item: row });

        context.send(AppEvent::ItemUpdated {
            item: self.item,
        });

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
    use crate::item::{ItemContent, ToolCallContent, ToolOutput};
    use crate::query::DataQuery;
    use crate::testing::{session_turn, tool_call};

    #[tokio::test]
    async fn test_execute_tool_call() {
        let mut app = App::new().unwrap();
        let mut conn = db::open(app.db_url()).unwrap();
        let (session, turn) = session_turn(&mut conn);

        // Tools run on a live shell.
        let sh = |command| json!({ "command": command });
        let stdout_call = tool_call(&mut conn, &turn, "sh", "call1", sh("printf 'hi'"), None);
        let stderr_call = tool_call(&mut conn, &turn, "sh", "call2", sh("printf 'bye' >&2"), None);
        // Failed calls record their error as output.
        let bad_args_call = tool_call(&mut conn, &turn, "sh", "call3", json!({ "command": 123 }), None);
        let unknown_call = tool_call(&mut conn, &turn, "no_such_tool", "call4", json!({}), None);

        let mut context = app.context();
        for item in [stdout_call, stderr_call, bad_args_call, unknown_call] {
            ExecuteToolCall::new(item).run(&mut context).await.unwrap();
        }

        let items = Item::list_by_session(&mut conn, session.id).unwrap();
        let outputs: Vec<_> = items
            .iter()
            .map(|item| item.query("/content/output").unwrap())
            .collect();
        assert_eq!(
            outputs,
            [
                json!({
                    "status": "completed",
                    "content": { "stdout": "hi", "stderr": "", "return_code": 0 },
                }),
                json!({
                    "status": "completed",
                    "content": { "stdout": "", "stderr": "bye", "return_code": 0 },
                }),
                json!({
                    "status": "failed",
                    "content": "invalid arguments for 'sh': expected {\"command\": \"...\"}",
                }),
                json!({
                    "status": "failed",
                    "content": "no such tool 'no_such_tool'",
                }),
            ]
        );

        let events: Vec<_> = app
            .drain_events()
            .into_iter()
            .filter(|event| !matches!(event, AppEvent::DbItemUpdated { .. }))
            .collect();
        assert_eq!(events.len(), items.len());
        for (event, item) in events.iter().zip(&items) {
            match event {
                AppEvent::ItemUpdated { item: updated } => {
                    assert_eq!(updated.id, item.id);
                    assert_eq!(updated.content, item.content);
                }
                other => panic!("unexpected event: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn test_execute_failed_tool_call() {
        let mut app = App::new().unwrap();
        let mut conn = db::open(app.db_url()).unwrap();
        let (_, turn) = session_turn(&mut conn);
        let call = tool_call(&mut conn, &turn, "no_such_tool", "call5", json!({}), None);

        let mut context = app.context();
        ExecuteToolCall::new(call).run(&mut context).await.unwrap();

        let events: Vec<_> = app
            .drain_events()
            .into_iter()
            .filter(|event| !matches!(event, AppEvent::DbItemUpdated { .. }))
            .collect();
        let AppEvent::ItemUpdated { item } = &events[0] else {
            panic!("unexpected event: {:?}", events[0]);
        };
        assert_eq!(
            item.content,
            ItemContent::ToolCall(ToolCallContent {
                tool_name: "no_such_tool".to_owned(),
                call_id: "call5".to_owned(),
                args: json!({}),
                output: Some(ToolOutput::Failed {
                    error: "no such tool 'no_such_tool'".to_owned(),
                }),
            }),
        );
    }
}
