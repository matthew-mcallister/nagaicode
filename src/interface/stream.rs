// TODO: batch DB writes and flush on item completion
use std::pin::Pin;

use anyhow::anyhow;
use diesel::SqliteConnection;
use fnv::FnvHashMap;
use futures::{Stream, StreamExt};
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::interface::{InferenceEvent, ItemDelta, OutputItemEvent};
use crate::item::{DbItem, ItemType, NewDbItem};
use crate::session::{Response, Session, Turn, TurnType};

/// Dumb data pipe. Consumes events off the wire and writes to the DB. Sends
/// out change notifications.
pub struct StreamProcessor<'a, S> {
    stream: Pin<Box<S>>,
    session: Session,
    send: UnboundedSender<AppEvent>,
    conn: &'a mut SqliteConnection,
    provider_id: i32,
    provider_name: String,
    model_id: String,
    turn_id: Option<i32>,
    base_seqno: i64,
    response: Option<Response>,
    items: FnvHashMap<i64, DbItem>,
}

impl<'a, S> StreamProcessor<'a, S> {
    pub fn new(
        session: Session,
        send: UnboundedSender<AppEvent>,
        conn: &'a mut SqliteConnection,
        turn_id: Option<i32>,
        base_seqno: i64,
        stream: S,
        provider_id: i32,
        provider_name: String,
        model_id: String,
    ) -> Self {
        Self {
            stream: Box::pin(stream),
            session,
            send,
            conn,
            provider_id,
            provider_name,
            model_id,
            turn_id,
            base_seqno,
            response: None,
            items: FnvHashMap::default(),
        }
    }

    fn handle_event(&mut self, event: InferenceEvent) -> AnyResult<()> {
        match event {
            InferenceEvent::Created(created) => {
                let turn_id = self.ensure_turn()?;
                let response = Response::create(
                    self.conn,
                    self.session.id,
                    turn_id,
                    Some(&created.id),
                    Some(&created.status),
                )?;
                self.response = Some(response);
            }
            InferenceEvent::OutputItemAdded(added) => self.handle_item_added(added)?,
            InferenceEvent::OutputItemDone(done) => self.handle_item_done(done)?,
            InferenceEvent::ReasoningTextDelta(delta) => self.handle_delta(delta)?,
            InferenceEvent::ReasoningSummaryDelta(delta) => self.handle_summary_delta(delta)?,
            InferenceEvent::OutputTextDelta(delta) => self.handle_delta(delta)?,
            InferenceEvent::FunctionCallArgsDelta(_) => {},
            InferenceEvent::Completed(completed) => {
                if let Some(response) = &self.response {
                    Response::finish(
                        self.conn,
                        response.id,
                        &completed.status,
                        completed.usage.as_ref(),
                        Some(&completed.raw_response),
                    )?;
                }
            }
            InferenceEvent::Failed(failed) => {
                if let Some(response) = &self.response {
                    Response::finish(
                        self.conn,
                        response.id,
                        &failed.status,
                        failed.usage.as_ref(),
                        Some(&failed.raw_response),
                    )?;
                }
                let _ = self.send.send(AppEvent::ErrorMessage(failed.error_message));
            }
        }
        Ok(())
    }

    fn ensure_turn(&mut self) -> AnyResult<i32> {
        if let Some(turn_id) = self.turn_id {
            return Ok(turn_id);
        }
        let turn = Turn::create(
            self.conn,
            self.session.id,
            TurnType::Assistant,
            Some(self.provider_id),
            Some(&self.provider_name),
            Some(&self.model_id),
        )?;
        self.turn_id = Some(turn.id);
        Ok(turn.id)
    }

    // Creates a new item. Tool calls require args to be complete and present.
    fn handle_item_added(&mut self, added: OutputItemEvent) -> AnyResult<()> {
        let Some(ty) = ItemType::from_upstream(&added.ty) else { return Ok(()); };

        // Tool calls: don't write until arguments are complete
        let args = added.tool_args.as_deref().unwrap_or("");
        if ty == ItemType::ToolCall && args.is_empty() { return Ok(()); }

        let text = added.tool_name.as_deref();
        let turn_id = self.ensure_turn()?;
        let response_id = self.response.as_ref().map(|r| r.id);
        let raw = added.raw.to_string();
        let item = DbItem::create(
            self.conn,
            NewDbItem {
                session_id: Some(self.session.id),
                turn_id: Some(turn_id),
                response_id,
                provider_id: Some(self.provider_id),
                ty: Some(ty),
                upstream_id: Some(&added.id),
                upstream_type: Some(&added.ty),
                upstream_call_id: added.call_id.as_deref(),
                text,
                summary: None,
                encrypted_text: None,
                tool_args: added.tool_args.clone(),
                tool_output: None,
                seqno: Some(self.base_seqno + added.output_index),
                raw_data: Some(&raw),
            },
        )?;
        self.items.insert(added.output_index, item.clone());

        let _ = self.send.send(AppEvent::DbItemCreated { item });
        Ok(())
    }

    fn handle_item_done(&mut self, done: OutputItemEvent) -> AnyResult<()> {
        if ItemType::from_upstream(&done.ty) == Some(ItemType::ToolCall) {
            return self.handle_item_added(done);
        }
        let Some(item) = self.items.get_mut(&done.output_index) else { return Ok(()) };
        item.set_raw_data(self.conn, done.raw.to_string())?;
        let _ = self.send.send(AppEvent::DbItemUpdated { item: item.clone() });
        Ok(())
    }

    fn handle_delta(&mut self, delta: ItemDelta) -> AnyResult<()> {
        let Some(item) = self.items.get_mut(&delta.output_index) else {
            log::warn!("received delta before item");
            return Ok(())
        };
        let text = format!("{}{}", item.text.as_deref().unwrap_or(""), &delta.delta);
        item.update_text(self.conn, &text)?;
        let _ = self.send.send(AppEvent::DbItemUpdated { item: item.clone() });
        Ok(())
    }

    fn handle_summary_delta(&mut self, delta: ItemDelta) -> AnyResult<()> {
        let Some(item) = self.items.get_mut(&delta.output_index) else {
            log::warn!("received delta before item");
            return Ok(())
        };
        let summary = format!("{}{}", item.summary.as_deref().unwrap_or(""), &delta.delta);
        item.update_summary(self.conn, &summary)?;
        let _ = self.send.send(AppEvent::DbItemUpdated { item: item.clone() });
        Ok(())
    }

    fn fail_response(&mut self) -> AnyResult<()> {
        if let Some(response) = &self.response {
            Response::finish(self.conn, response.id, "failed", None, None)?;
        }
        Ok(())
    }
}

impl<'a, S> StreamProcessor<'a, S>
where
    S: Stream<Item = AnyResult<InferenceEvent>>,
{
    /// Pumps the stream to completion, persisting state and emitting events.
    pub async fn process(&mut self) -> AnyResult<(i32, i32)> {
        while let Some(item) = self.stream.next().await {
            match item {
                Ok(event) => self.handle_event(event)?,
                Err(e) => {
                    let e = match self.fail_response() {
                        Ok(()) => e,
                        Err(f) => anyhow!("multiple errors:\n{e}\n{f}"),
                    };
                    log::error!("agent failed: {e}");
                    return Err(e);
                }
            }
        }

        let turn_id = self.turn_id.ok_or_else(|| anyhow!("no turn created"))?;
        let response_id = self
            .response
            .as_ref()
            .map(|r| r.id)
            .ok_or_else(|| anyhow!("no response created"))?;
        Ok((turn_id, response_id))
    }
}
