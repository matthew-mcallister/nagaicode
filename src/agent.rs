use diesel::SqliteConnection;
use fnv::FnvHashMap;
use futures::StreamExt;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::interface::{
    InferenceEvent, InferenceParams, ItemDelta, OutputItemEvent, build_history,
};
use crate::model::Model;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::{Item, ItemType, NewItem, Response, Session, Turn, TurnType};
use crate::tasks::{Task, TaskContext};

pub struct Agent {
    pub session: Session,
    pub provider: Provider,
    pub model: Model,
    pub client: DefaultClient,
    pub conn: SqliteConnection,

    turn: Option<Turn>,
    response: Option<Response>,
    items: FnvHashMap<i64, Item>,
}

impl Agent {
    pub fn new(
        session: Session,
        provider: Provider,
        model: Model,
        client: DefaultClient,
        conn: SqliteConnection,
    ) -> Self {
        Self {
            session,
            provider,
            model,
            client,
            conn,
            turn: None,
            response: None,
            items: FnvHashMap::default(),
        }
    }

    async fn run(mut self, context: &TaskContext) -> AnyResult<()> {
        let interface = self.provider.create_interface(&self.client)?;

        let history = self.load_history()?;
        let messages = build_history(&history, interface.supports_reasoning_input())?;
        let mut params = self.build_params();
        params.input = &messages;
        let stream = interface.generate(params);
        tokio::pin!(stream);

        while let Some(item) = stream.next().await {
            match item {
                Ok(event) => {
                    if let Some(event) = self.handle_event(event)? {
                        context.send(event);
                    }
                }
                Err(e) => {
                    match self.fail_response() {
                        Ok(()) => return Err(e),
                        Err(f) => return Err(f.context(e)),
                    }
                }
            }
        }
        Ok(())
    }

    fn build_params(&self) -> InferenceParams<'_> {
        InferenceParams {
            model_id: &self.model.id,
            system_prompt: "",
            temperature: 0.7,
            reasoning_effort: None,
            input: &[],
        }
    }

    /// Loads the session's items sorted by item id.
    fn load_history(&mut self) -> AnyResult<Vec<Item>> {
        Item::list_by_session(&mut self.conn, self.session.id)
    }

    fn handle_event(&mut self, event: InferenceEvent) -> AnyResult<Option<AppEvent>> {
        match event {
            InferenceEvent::Created(created) => {
                let turn = self.ensure_turn()?;
                let response = Response::create(
                    &mut self.conn,
                    self.session.id,
                    turn.id,
                    Some(&created.id),
                    Some(&created.status),
                )?;
                self.response = Some(response);
                Ok(None)
            }
            InferenceEvent::OutputItemAdded(added) => self.handle_item_added(added),
            InferenceEvent::OutputItemDone(done) => self.handle_item_done(done),
            InferenceEvent::ReasoningTextDelta(delta) => {
                self.handle_delta(delta, ItemType::Reasoning, false)
            }
            InferenceEvent::ReasoningSummaryDelta(delta) => {
                self.handle_delta(delta, ItemType::Reasoning, true)
            }
            InferenceEvent::OutputTextDelta(delta) => {
                self.handle_delta(delta, ItemType::ResponseText, false)
            }
            InferenceEvent::FunctionCallArgsDelta(delta) => self.handle_args_delta(delta),
            InferenceEvent::Completed(completed) => {
                if let Some(response) = &self.response {
                    Response::finish(
                        &mut self.conn,
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
                        &mut self.conn,
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
        let Some(ty) = ItemType::from_upstream(&added.ty) else {
            return Ok(None);
        };
        let name = if ty == ItemType::ToolCall {
            added.raw["name"].as_str()
        } else {
            None
        };
        let item = self.create_item(
            added.output_index,
            ty,
            (!added.id.is_empty()).then_some(added.id.as_str()),
            Some(added.ty.as_str()),
            added.call_id.as_deref(),
            name,
        )?;
        Ok(Some(AppEvent::ItemCreated { item }))
    }

    fn handle_item_done(&mut self, done: OutputItemEvent) -> AnyResult<Option<AppEvent>> {
        if let Some(item) = self.items.get_mut(&done.output_index) {
            if let Some(json) = item.json.as_deref() {
                Item::update_json(&mut self.conn, item.id, json)?;
            }
            Item::set_raw_data(&mut self.conn, item.id, &done.raw)?;
            item.raw_data = Some(done.raw.to_string());
        }
        Ok(None)
    }

    fn ensure_item(&mut self, output_index: i64, ty: ItemType) -> AnyResult<()> {
        if !self.items.contains_key(&output_index) {
            // Lazily create a new item in case we receive out-of-order
            self.create_item(output_index, ty, None, None, None, None)?;
        }
        Ok(())
    }

    // TODO: batch DB writes and flush on item completion
    fn handle_delta(
        &mut self,
        delta: ItemDelta,
        ty: ItemType,
        summary: bool,
    ) -> AnyResult<Option<AppEvent>> {
        self.ensure_item(delta.output_index, ty)?;
        let item = self
            .items
            .get_mut(&delta.output_index)
            .expect("item exists");
        if summary {
            let summary = format!("{}{}", item.summary.as_deref().unwrap_or(""), &delta.delta);
            Item::update_summary(&mut self.conn, item.id, &summary)?;
            item.summary = Some(summary);
        } else {
            let text = format!("{}{}", item.text.as_deref().unwrap_or(""), &delta.delta);
            Item::update_text(&mut self.conn, item.id, &text)?;
            item.text = Some(text);
        }
        Ok(Some(AppEvent::ItemUpdated { item: item.clone() }))
    }

    /// Appends args but does not write to DB until item is fully finished
    /// so we don't commit incomplete JSON
    fn handle_args_delta(&mut self, delta: ItemDelta) -> AnyResult<Option<AppEvent>> {
        self.ensure_item(delta.output_index, ItemType::ToolCall)?;
        let item = self
            .items
            .get_mut(&delta.output_index)
            .expect("item exists");
        let json = format!("{}{}", item.json.as_deref().unwrap_or(""), &delta.delta);
        item.json = Some(json);
        Ok(None)
    }

    fn create_item(
        &mut self,
        output_index: i64,
        ty: ItemType,
        upstream_id: Option<&str>,
        upstream_type: Option<&str>,
        upstream_call_id: Option<&str>,
        text: Option<&str>,
    ) -> AnyResult<Item> {
        let turn = self.ensure_turn()?;
        let response_id = self.response.as_ref().map(|r| r.id);
        let item = Item::create(
            &mut self.conn,
            NewItem {
                session_id: self.session.id,
                turn_id: turn.id,
                response_id,
                provider_id: Some(self.provider.id),
                ty,
                upstream_id,
                upstream_type,
                upstream_call_id,
                text,
            },
        )?;
        self.items.insert(output_index, item.clone());
        Ok(item)
    }

    fn ensure_turn(&mut self) -> AnyResult<Turn> {
        if let Some(turn) = &self.turn {
            return Ok(turn.clone());
        }
        let turn = Turn::create(
            &mut self.conn,
            self.session.id,
            TurnType::Assistant,
            Some(self.provider.id),
            Some(&self.provider.name),
            Some(&self.model.id),
        )?;
        self.turn = Some(turn.clone());
        Ok(turn)
    }

    /// Marks the active response as failed
    fn fail_response(&mut self) -> AnyResult<()> {
        if let Some(response) = &self.response {
            Response::finish(&mut self.conn, response.id, "failed", None, None)?;
        }
        Ok(())
    }
}

impl Task for Agent {
    type Output = ();

    async fn run(self, context: TaskContext) {
        if let Err(e) = self.run(&context).await {
            context.send(AppEvent::ErrorMessage(e.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    use tokio::sync::mpsc::unbounded_channel;

    use crate::interface::InterfaceId;
    use crate::model::Model;
    use crate::provider::Provider;
    use crate::session::Session;
    use crate::tasks::{TaskContext, TaskError};

    use super::*;

    #[tokio::test]
    async fn test_spawn_cancels() {
        let mut conn = crate::db::open_new().unwrap();
        let session = Session::create(&mut conn, "Session").expect("create session");
        let provider = Provider::create(&mut conn, "test", InterfaceId::Openai, "key123", None)
            .expect("create provider");
        let model = Model::create(&mut conn, provider.id, "gpt-4").expect("create model");

        let (sender, mut recv) = unbounded_channel();
        let context = TaskContext::root(Arc::new(AtomicU64::new(0)), sender);
        let handle = context.spawn(Agent::new(
            session,
            provider,
            model,
            DefaultClient::default(),
            conn,
        ));
        handle.cancel();
        let result = handle.join().await.unwrap();
        assert_eq!(result, Err(TaskError::Canceled));
        assert_eq!(recv.try_recv().unwrap(), AppEvent::TaskStarted(0));
        assert_eq!(recv.try_recv().unwrap(), AppEvent::TaskEnded(0));
    }
}
