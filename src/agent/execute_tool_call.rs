use diesel::Connection;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::item::{Item, ItemContent, NewItem, ToolOutputContent};
use crate::task::{Task, TaskContext};
use crate::tools::ToolCallContext;

pub struct ExecuteToolCall {
    item: Item,
}

impl ExecuteToolCall {
    pub fn new(item: Item) -> Self {
        Self { item }
    }

    async fn process(mut self, context: &mut TaskContext) -> AnyResult<()> {
        let tc = self.item.content.as_tool_call().unwrap().clone();
        let tool_context = ToolCallContext {
            session_id: self.item.session_id,
        };
        let output = context
            .tool_registry()
            .call(&tool_context, &tc.tool_name, &tc.args)
            .await;

        let new_item = context.connection()?.transaction(|cxn| -> AnyResult<_> {
            self.item.set_output(cxn, output.clone())?;
            let new_item = Item::create(cxn, NewItem {
                session_id: self.item.session_id,
                turn_id: self.item.turn_id,
                response_id: None,
                provider_id: None,
                upstream_id: self.item.upstream_id.clone(),
                seqno: None,
                content: ItemContent::ToolOutput(ToolOutputContent {
                    tool_name: tc.tool_name,
                    call_id: tc.call_id,
                    args: tc.args,
                    output,
                }),
            })?;
            Ok(new_item)
        })?;

        context.send(AppEvent::ItemCreated { item: new_item });
        context.send(AppEvent::ItemUpdated { item: self.item.clone() });

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
    use crate::item::{ItemContent, ToolOutput};
    use crate::query::DataQuery;
    use crate::testing::{normalize, session_turn, tool_call};

    #[tokio::test]
    async fn test_execute_tool_call() {
        let mut app = App::new().unwrap();
        let mut conn = db::open(app.db_url()).unwrap();
        let (session, turn) = session_turn(&mut conn);

        // Tools run on a live shell.
        let sh = |command| json!({ "command": command });
        let stdout_call = tool_call(&mut conn, &turn, "sh", "call1", sh("echo hi"), None);
        let failing_call =
            tool_call(&mut conn, &turn, "sh", "call2", sh("no_such_command_xyz"), None);
        // Failed calls record their error as output.
        let bad_args_call = tool_call(&mut conn, &turn, "sh", "call3", json!({ "command": 123 }), None);
        let unknown_call = tool_call(&mut conn, &turn, "no_such_tool", "call4", json!({}), None);

        let mut context = app.context();
        for item in [stdout_call, failing_call, bad_args_call, unknown_call] {
            ExecuteToolCall::new(item).run(&mut context).await.unwrap();
        }

        let items = Item::list_by_session(&mut conn, session.id).unwrap();
        let outputs: Vec<_> = items
            .iter()
            .map(|item| item.query("/content/output").unwrap())
            .collect();

        assert_eq!(outputs[0]["status"], json!("completed"));
        assert_eq!(outputs[0]["content"]["stderr"], json!(""));
        assert_eq!(outputs[0]["content"]["return_code"], json!(0));
        assert_eq!(
            normalize(outputs[0]["content"]["stdout"].as_str().unwrap()),
            "hi\n"
        );

        assert_eq!(outputs[1]["status"], json!("completed"));
        assert_ne!(outputs[1]["content"]["return_code"], json!(0));
        assert!(!outputs[1]["content"]["stderr"].as_str().unwrap().is_empty());

        assert_eq!(
            outputs[2],
            json!({
                "status": "failed",
                "content": "invalid arguments for 'sh': expected {\"command\": \"...\"}",
            })
        );
        assert_eq!(
            outputs[3],
            json!({
                "status": "failed",
                "content": "no such tool 'no_such_tool'",
            })
        );

        let events = app.drain_events();
        assert_eq!(events.len(), 8);
        let calls: Vec<_> = items.iter().filter(|item| item.content.as_tool_call().is_some()).collect();
        let tool_outputs: Vec<_> = items.iter().filter(|item| item.content.as_tool_output().is_some()).collect();
        assert_eq!(calls.len(), 4);
        assert_eq!(tool_outputs.len(), 4);
        for (pair, (call, tool_output)) in events.chunks(2).zip(calls.iter().zip(&tool_outputs)) {
            let [AppEvent::ItemCreated { item: created }, AppEvent::ItemUpdated { item: updated }] = pair else {
                panic!("unexpected events: {pair:?}");
            };
            assert_eq!(created.id, tool_output.id);
            assert_eq!(created.content, tool_output.content);
            assert_eq!(updated.id, call.id);
            assert_eq!(updated.content, call.content);
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

        let events = app.drain_events();
        let AppEvent::ItemCreated { item } = &events[0] else {
            panic!("unexpected event: {:?}", events[0]);
        };
        assert_eq!(
            item.content,
            ItemContent::ToolOutput(ToolOutputContent {
                tool_name: "no_such_tool".to_owned(),
                call_id: "call5".to_owned(),
                args: json!({}),
                output: ToolOutput::Failed {
                    error: "no such tool 'no_such_tool'".to_owned(),
                },
            }),
        );
    }
}
