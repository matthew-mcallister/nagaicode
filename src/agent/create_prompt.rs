use diesel::prelude::*;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::session::{DbItem, ItemType, NewItem, Turn, TurnType};
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
            let item = DbItem::create(
                conn,
                NewItem {
                    session_id: Some(session_id),
                    turn_id: Some(turn.id),
                    response_id: None,
                    provider_id: None,
                    ty: Some(ItemType::UserText),
                    upstream_id: None,
                    upstream_type: None,
                    upstream_call_id: None,
                    text: Some(&self.prompt),
                    seqno: None,
                },
            )?;
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
    use crate::app::App;
    use crate::session::Session;

    use super::*;

    #[tokio::test]
    async fn test_create_prompt() {
        let mut app = App::new().unwrap();

        let session = Session::create(app.conn(), "Session").unwrap();
        let turn = Turn::create(app.conn(), session.id, TurnType::Assistant, None, None, None)
            .unwrap();
        let tool_call = DbItem::create(
            app.conn(),
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(turn.id),
                provider_id: Some(1),
                ty: Some(ItemType::ToolCall),
                upstream_id: Some("fc_1"),
                upstream_type: Some("function_call"),
                upstream_call_id: Some("call_1"),
                text: Some("read_file"),
                ..Default::default()
            },
        ).unwrap();

        app.context().subtask(CreatePrompt::new(session.id, "hello")).await.unwrap();
        let events = app.drain_events();
        // TaskStarted, create prompt, TaskEnded
        assert_eq!(events.len(), 3);

        let turns = Turn::list_by_session(app.conn(), session.id).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].ty().unwrap(), TurnType::User);

        let items = DbItem::list_by_session(app.conn(), session.id).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, tool_call.id);
        assert_eq!(items[1].ty().unwrap(), ItemType::UserText);
        assert_eq!(items[1].text.as_deref(), Some("hello"));
        assert!(items[1].seqno > tool_call.seqno);
    }
}
