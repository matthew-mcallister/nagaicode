use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;

use crate::error::AnyResult;
use crate::session::{Item, ItemType, NewItem};

/// Backfills missing tool outputs for tool calls. OpenAI chokes when a tool
/// call is missing its output so we must do this before building history.
pub fn backfill_tool_outputs(conn: &mut SqliteConnection, session_id: i32) -> AnyResult<()> {
    use crate::schema::item;

    let (call, output) = diesel::alias!(item as call, item as output);

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
        Item::create(
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
                text: Some("error: tool call interrupted"),
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::session::{Session, Turn, TurnType};

    #[test]
    fn test_backfill_tool_outputs() {
        let mut conn = db::open_new().expect("failed to open in-memory db");

        let session = Session::create(&mut conn, "Session 1").unwrap();
        let turn = Turn::create(&mut conn, session.id, TurnType::Assistant, None, None, None)
            .unwrap();

        Item::create(
            &mut conn,
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
            },
        )
        .unwrap();
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
                upstream_call_id: Some("call_1"),
                text: Some("file contents"),
            },
        )
        .unwrap();

        let interrupted_call = Item::create(
            &mut conn,
            NewItem {
                session_id: session.id,
                turn_id: turn.id,
                response_id: None,
                provider_id: Some(2),
                ty: ItemType::ToolCall,
                upstream_id: Some("fc_2"),
                upstream_type: Some("function_call"),
                upstream_call_id: Some("call_2"),
                text: Some("write_file"),
            },
        )
        .unwrap();

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
                upstream_call_id: None,
                text: Some("echo"),
            },
        )
        .unwrap();

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
                upstream_id: Some("fc_3"),
                upstream_type: Some("function_call"),
                upstream_call_id: Some("call_3"),
                text: Some("read_file"),
            },
        )
        .unwrap();

        backfill_tool_outputs(&mut conn, session.id).unwrap();

        let items = Item::list_by_session(&mut conn, session.id).unwrap();
        let outputs: Vec<_> = items
            .iter()
            .filter(|i| i.ty == ItemType::ToolOutput.to_string())
            .collect();
        assert_eq!(outputs.len(), 2);

        let backfilled = outputs
            .iter()
            .find(|i| i.upstream_call_id.as_deref() == Some("call_2"))
            .expect("backfilled output");
        assert_eq!(backfilled.text.as_deref(), Some("error: tool call interrupted"));
        assert_eq!(backfilled.turn_id, interrupted_call.turn_id);
        assert_eq!(backfilled.session_id, session.id);
        assert_eq!(backfilled.provider_id, Some(2));
        assert_eq!(backfilled.upstream_type.as_deref(), Some("function_call_output"));
        assert_eq!(backfilled.response_id, None);

        let original = outputs
            .iter()
            .find(|i| i.upstream_call_id.as_deref() == Some("call_1"))
            .expect("original output");
        assert_eq!(original.text.as_deref(), Some("file contents"));

        let other_items = Item::list_by_session(&mut conn, other_session.id).unwrap();
        assert_eq!(other_items.len(), 1);

        backfill_tool_outputs(&mut conn, session.id).unwrap();
        let items = Item::list_by_session(&mut conn, session.id).unwrap();
        let outputs = items
            .iter()
            .filter(|i| i.ty == ItemType::ToolOutput.to_string())
            .count();
        assert_eq!(outputs, 2);
    }
}
