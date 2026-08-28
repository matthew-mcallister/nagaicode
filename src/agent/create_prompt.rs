use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::session::{Item, ItemType, NewItem, Turn, TurnType};
use crate::tasks::{Task, TaskContext};

/// Backfills missing tool outputs for tool calls. OpenAI chokes when a tool
/// call is missing its output so we must do this before building history.
fn backfill_tool_outputs(conn: &mut SqliteConnection, session_id: i32) -> AnyResult<Vec<AppEvent>> {
    use crate::schema::item;

    let mut events = Vec::new();

    // select turn_id, provider_id, upstream_call_id
    // from item icall
    // join item iout
    //   on icall.upstream_call_id = iout.upstream_call_id
    //   and iout.type = 'tool_output'
    //   and iout.session_id = icall.session_id
    // where
    //   icall.session_id = $session_id
    //   and icall.type = 'tool_call'
    //   and icall.upstream_call_id is not null
    //   and iout.id is null
    let (call, output) = diesel::alias!(item as call, item as output);
    let unmatched: Vec<(i32, Option<i32>, String)> = call
        .left_join(output.on(
            output
                .field(item::upstream_call_id)
                .eq(call.field(item::upstream_call_id))
                .and(output.field(item::ty).eq(ItemType::ToolOutput.to_string()))
                .and(output.field(item::session_id).eq(call.field(item::session_id))),
        ))
        .filter(call.field(item::session_id).eq(session_id))
        .filter(call.field(item::ty).eq(ItemType::ToolCall.to_string()))
        .filter(call.field(item::upstream_call_id).is_not_null())
        .filter(output.field(item::id).is_null())
        .select((
            call.field(item::turn_id),
            call.field(item::provider_id),
            call.field(item::upstream_call_id).assume_not_null(),
        ))
        .load::<(i32, Option<i32>, String)>(conn)?;

    for (turn_id, provider_id, upstream_call_id) in unmatched {
        let item = Item::create(
            conn,
            NewItem {
                session_id,
                turn_id,
                response_id: None,
                provider_id,
                ty: ItemType::ToolOutput,
                upstream_id: None,
                upstream_type: Some("function_call_output"),
                upstream_call_id: Some(&upstream_call_id),
                text: None,
                seqno: None,
                completed: false,
            },
        )?;
        events.push(AppEvent::ItemCreated { item });
    }

    Ok(events)
}

/// Creates a Turn and Item for a prompt. Also backfills any missing tool
/// outputs before inserting the prompt.
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
            let mut events = backfill_tool_outputs(conn, session_id)?;
            let turn = Turn::create(conn, session_id, TurnType::User, None, None, None)?;
            let item = Item::create(
                conn,
                NewItem {
                    session_id,
                    turn_id: turn.id,
                    response_id: None,
                    provider_id: None,
                    ty: ItemType::UserText,
                    upstream_id: None,
                    upstream_type: None,
                    upstream_call_id: None,
                    text: Some(&self.prompt),
                    seqno: None,
                    completed: true,
                },
            )?;
            events.push(AppEvent::ItemCreated { item });
            Ok(events)
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

    #[test]
    fn test_backfill_tool_outputs() {
        let mut conn = crate::db::open_new().expect("failed to open in-memory db");

        let session = Session::create(&mut conn, "Session 1").unwrap();
        let turn = Turn::create(&mut conn, session.id, TurnType::Assistant, None, None, None)
            .unwrap();

        // Completed call
        Item::create(
            &mut conn,
            NewItem {
                session_id: session.id,
                turn_id: turn.id,
                response_id: None,
                provider_id: Some(1),
                ty: ItemType::ToolCall,
                upstream_id: Some("completed"),
                upstream_type: Some("function_call"),
                upstream_call_id: Some("completed"),
                text: Some("read_file"),
                seqno: None,
                completed: true,
            },
        ).unwrap();
        Item::create(
            &mut conn,
            NewItem {
                session_id: session.id,
                turn_id: turn.id,
                response_id: None,
                provider_id: Some(1),
                ty: ItemType::ToolOutput,
                upstream_id: None,
                upstream_type: Some("function_call_output"),
                upstream_call_id: Some("completed"),
                text: Some("file contents"),
                seqno: None,
                completed: true,
            },
        ).unwrap();

        // Interrupted before output item created
        Item::create(
            &mut conn,
            NewItem {
                session_id: session.id,
                turn_id: turn.id,
                response_id: None,
                provider_id: Some(2),
                ty: ItemType::ToolCall,
                upstream_id: Some("interrupted"),
                upstream_type: Some("function_call"),
                upstream_call_id: Some("interrupted"),
                text: Some("write_file"),
                seqno: None,
                completed: true,
            },
        ).unwrap();

        // Incomplete tool call
        Item::create(
            &mut conn,
            NewItem {
                session_id: session.id,
                turn_id: turn.id,
                response_id: None,
                provider_id: Some(3),
                ty: ItemType::ToolOutput,
                upstream_id: None,
                upstream_type: Some("function_call_output"),
                upstream_call_id: Some("incomplete"),
                text: None,
                seqno: None,
                completed: false,
            },
        ).unwrap();
        Item::create(
            &mut conn,
            NewItem {
                session_id: session.id,
                turn_id: turn.id,
                response_id: None,
                provider_id: None,
                ty: ItemType::ToolCall,
                upstream_id: None,
                upstream_type: Some("function_call"),
                upstream_call_id: Some("incomplete"),
                text: Some("echo"),
                seqno: None,
                completed: true,
            },
        ).unwrap();

        // Missing output but in another session
        let other_session = Session::create(&mut conn, "Session 2").unwrap();
        let other_turn =
            Turn::create(&mut conn, other_session.id, TurnType::Assistant, None, None, None)
                .unwrap();
        Item::create(
            &mut conn,
            NewItem {
                session_id: other_session.id,
                turn_id: other_turn.id,
                response_id: None,
                provider_id: None,
                ty: ItemType::ToolCall,
                upstream_id: Some("other"),
                upstream_type: Some("function_call"),
                upstream_call_id: Some("other"),
                text: Some("read_file"),
                seqno: None,
                completed: true,
            },
        ).unwrap();

        let mut events = backfill_tool_outputs(&mut conn, session.id).unwrap();
        assert_eq!(events.len(), 1);
        let Some(AppEvent::ItemCreated { item }) = events.pop() else { panic!("wrong event type") };
        assert_eq!(item.upstream_call_id.as_deref(), Some("interrupted"));
        assert_eq!(item.text.as_deref(), None);
        assert!(!item.completed);
    }

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
                ty: ItemType::ToolCall,
                upstream_id: Some("fc_1"),
                upstream_type: Some("function_call"),
                upstream_call_id: Some("call_1"),
                text: Some("read_file"),
                seqno: None,
                completed: true,
            },
        ).unwrap();

        app.context().subtask(CreatePrompt::new(session.id, "hello")).await.unwrap();
        let events = app.drain_events();
        // TaskStarted, create tool output, create prompt, TaskEnded
        assert_eq!(events.len(), 4);

        let turns = Turn::list_by_session(app.conn(), session.id).unwrap();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].ty().unwrap(), TurnType::User);

        let items = Item::list_by_session(app.conn(), session.id).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].id, tool_call.id);
        assert_eq!(items[1].ty().unwrap(), ItemType::ToolOutput);
        assert_eq!(items[1].upstream_call_id.as_deref(), Some("call_1"));
        assert!(items[1].seqno > tool_call.seqno);
        assert!(items[2].seqno > items[1].seqno);
    }
}
