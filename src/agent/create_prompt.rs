use diesel::prelude::*;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::item::{Item, ItemContent, NewItem};
use crate::session::{Turn, TurnType};
use crate::task::{Task, TaskContext};

/// Creates a Turn and Item for a prompt.
pub struct CreatePrompt {
    session_id: i32,
    prompt: String,
}

impl CreatePrompt {
    pub fn new(session_id: i32, prompt: &str) -> Self {
        Self {
            session_id,
            prompt: prompt.to_string(),
        }
    }

    async fn process(self, context: &mut TaskContext) -> AnyResult<()> {
        let events: AnyResult<_> = context.connection()?.transaction(|conn| {
            let session_id = self.session_id;
            let turn = Turn::create(conn, session_id, TurnType::User, None, None, None)?;
            let item = Item::create(conn, NewItem {
                session_id,
                turn_id: turn.id,
                response_id: None,
                provider_id: None,
                upstream_id: None,
                seqno: None,
                content: ItemContent::UserText(self.prompt.clone()),
            })?;
            Ok(vec![AppEvent::ItemCreated { item }])
        });
        for event in events? {
            context.send(event);
        }
        Ok(())
    }
}

impl Task for CreatePrompt {
    type Output = AnyResult<()>;

    async fn run(self, context: &mut TaskContext) -> AnyResult<()> {
        self.process(context).await
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::app::App;
    use crate::item::ToolCallContent;
    use crate::session::Session;

    use super::*;

    #[tokio::test]
    async fn test_create_prompt() {
        let mut app = App::new().unwrap();

        let session = Session::create(app.conn(), "Session").unwrap();
        let turn = Turn::create(app.conn(), session.id, TurnType::Assistant, None, None, None)
            .unwrap();
        let tool_call = Item::create(
            app.conn(),
            NewItem {
                session_id: session.id,
                turn_id: turn.id,
                response_id: None,
                provider_id: Some(1),
                upstream_id: Some("fc_1".to_owned()),
                seqno: None,
                content: ItemContent::ToolCall(ToolCallContent {
                    tool_name: "read_file".to_owned(),
                    call_id: "call_1".to_owned(),
                    args: json!({}),
                    output: None,
                }),
            },
        ).unwrap();

        app.context().subtask(CreatePrompt::new(session.id, "hello")).await.unwrap();
        let events = app.drain_events();
        // TaskStarted, create prompt, TaskEnded
        assert_eq!(events.len(), 3);

        let turns = Turn::list_by_session(app.conn(), session.id).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].ty().unwrap(), TurnType::User);

        let items = Item::list_by_session(app.conn(), session.id).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, tool_call.id);
        assert_eq!(items[1].content, ItemContent::UserText("hello".to_owned()));
        assert!(items[1].seqno > tool_call.seqno);
    }
}
