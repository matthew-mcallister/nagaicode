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
use crate::session::{DbItem, ItemType, NewItem, Response, Session, Turn, TurnType};

/// Consumes an inference event stream and persists it into a session.
pub struct StreamProcessor<'a, S> {
    stream: Pin<Box<S>>,
    session: Session,
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
    /// Creates a processor for `stream` targeting `session`.
    pub fn new(
        session: Session,
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

    fn handle_event(&mut self, event: InferenceEvent) -> AnyResult<Option<AppEvent>> {
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
                Ok(None)
            }
            InferenceEvent::OutputItemAdded(added) => self.handle_item_added(added),
            InferenceEvent::OutputItemDone(done) => self.handle_item_done(done),
            InferenceEvent::ReasoningTextDelta(delta) => self.handle_delta(delta),
            InferenceEvent::ReasoningSummaryDelta(delta) => self.handle_summary_delta(delta),
            InferenceEvent::OutputTextDelta(delta) => self.handle_delta(delta),
            InferenceEvent::FunctionCallArgsDelta(delta) => {
                self.handle_args_delta(delta)?;
                Ok(None)
            }
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
                Ok(None)
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
                Ok(Some(AppEvent::ErrorMessage(failed.error_message)))
            }
        }
    }

    fn handle_item_added(&mut self, added: OutputItemEvent) -> AnyResult<Option<AppEvent>> {
        let Some(ty) = ItemType::from_upstream(&added.ty) else { return Ok(None); };
        let text = if ty == ItemType::ToolCall { added.raw["name"].as_str() } else { None };
        let item = self.create_item(
            added.output_index,
            ty,
            &added.id,
            added.ty.as_str(),
            added.call_id.as_deref(),
            text,
        )?;
        Ok(Some(AppEvent::DbItemCreated { item }))
    }

    fn handle_item_done(&mut self, done: OutputItemEvent) -> AnyResult<Option<AppEvent>> {
        let Some(item) = self.items.get_mut(&done.output_index) else { return Ok(None) };
        if item.ty == "tool_call" {
            if let Some(args) = done.raw.get("arguments").and_then(|args| args.as_str()) {
                item.update_tool_args(self.conn, args)?;
            } else {
                log::warn!("tool call '{}' missing args", done.call_id.as_deref().unwrap_or(""));
                return Ok(None);
            }
        }
        DbItem::set_raw_data(self.conn, item.id, &done.raw)?;
        item.raw_data = Some(done.raw.to_string());
        Ok(Some(AppEvent::DbItemUpdated { item: item.clone() }))
    }

    fn handle_delta(&mut self, delta: ItemDelta) -> AnyResult<Option<AppEvent>> {
        let Some(item) = self.items.get_mut(&delta.output_index) else {
            log::warn!("received delta before item");
            return Ok(None)
        };
        let text = format!("{}{}", item.text.as_deref().unwrap_or(""), &delta.delta);
        item.update_text(self.conn, &text)?;
        Ok(Some(AppEvent::DbItemUpdated { item: item.clone() }))
    }

    fn handle_summary_delta(&mut self, delta: ItemDelta) -> AnyResult<Option<AppEvent>> {
        let Some(item) = self.items.get_mut(&delta.output_index) else {
            log::warn!("received delta before item");
            return Ok(None)
        };
        let summary = format!("{}{}", item.summary.as_deref().unwrap_or(""), &delta.delta);
        item.update_summary(self.conn, &summary)?;
        Ok(Some(AppEvent::DbItemUpdated { item: item.clone() }))
    }

    fn handle_args_delta(&mut self, delta: ItemDelta) -> AnyResult<()> {
        let Some(item) = self.items.get_mut(&delta.output_index) else {
            log::warn!("received delta before item");
            return Ok(())
        };
        let tool_args = format!("{}{}", item.tool_args.as_deref().unwrap_or(""), &delta.delta);
        item.tool_args = Some(tool_args);
        // Don't emit event here, args aren't finished and no one consumes them
        Ok(())
    }

    fn create_item(
        &mut self,
        output_index: i64,
        ty: ItemType,
        upstream_id: &str,
        upstream_type: &str,
        upstream_call_id: Option<&str>,
        text: Option<&str>,
    ) -> AnyResult<DbItem> {
        let turn_id = self.ensure_turn()?;
        let response_id = self.response.as_ref().map(|r| r.id);
        let item = DbItem::create(
            self.conn,
            NewItem {
                session_id: Some(self.session.id),
                turn_id: Some(turn_id),
                response_id,
                provider_id: Some(self.provider_id),
                ty: Some(ty),
                upstream_id: Some(upstream_id),
                upstream_type: Some(upstream_type),
                upstream_call_id,
                text,
                seqno: Some(self.base_seqno + output_index),
            },
        )?;
        self.items.insert(output_index, item.clone());
        Ok(item)
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
    pub async fn process(&mut self, sender: &UnboundedSender<AppEvent>) -> AnyResult<(i32, i32)> {
        while let Some(item) = self.stream.next().await {
            match item {
                Ok(event) => {
                    if let Some(event) = self.handle_event(event)? {
                        let _ = sender.send(event);
                    }
                }
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
