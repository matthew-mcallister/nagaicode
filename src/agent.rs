use diesel::SqliteConnection;
use fnv::FnvHashMap;
use futures::StreamExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::interface::{
    InferenceEvent, InferenceParams, ItemDelta, OutputItemEvent, build_history,
};
use crate::model::Model;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::{Item, ItemType, NewItem, Response, Session, Turn, TurnType};

pub struct Agent {
    pub session: Session,
    pub provider: Provider,
    pub model: Model,
    pub sender: UnboundedSender<AppEvent>,
    pub client: DefaultClient,
    pub conn: SqliteConnection,
    pub cancel: CancellationToken,

    turn: Option<Turn>,
    response: Option<Response>,
    items: FnvHashMap<i64, Item>,
}

impl Agent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session: Session,
        provider: Provider,
        model: Model,
        sender: UnboundedSender<AppEvent>,
        client: DefaultClient,
        conn: SqliteConnection,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            session,
            provider,
            model,
            sender,
            client,
            conn,
            cancel,
            turn: None,
            response: None,
            items: FnvHashMap::default(),
        }
    }

    pub async fn run(mut self) -> AnyResult<()> {
        if self.cancel.is_cancelled() {
            return Ok(());
        }
        let interface = self.provider.create_interface(&self.client)?;

        let history = self.load_history()?;
        let messages = build_history(&history, interface.supports_reasoning_input())?;
        let mut params = self.build_params();
        params.input = &messages;
        let stream = interface.generate(params);
        tokio::pin!(stream);

        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => return Ok(()),
                item = stream.next() => match item {
                    Some(Ok(event)) => {
                        if let Some(event) = self.handle_event(event)? {
                            let _ = self.sender.send(event);
                        }
                    }
                    Some(Err(e)) => {
                        match self.fail_response() {
                            Ok(()) => return Err(e),
                            // FIXME: Switch to anyhow and add f to the error chain
                            Err(_f) => return Err(e),
                        }
                    }
                    None => return Ok(()),
                }
            }
        }
    }

    pub fn spawn(self) -> JoinHandle<()> {
        let sender = self.sender.clone();
        tokio::spawn(async move {
            if let Err(e) = self.run().await {
                let _ = sender.send(AppEvent::ErrorMessage(e.to_string()));
            }
            let _ = sender.send(AppEvent::TaskComplete);
        })
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
        let item = self.create_item(
            added.output_index,
            ty,
            (!added.id.is_empty()).then_some(added.id.as_str()),
            Some(added.ty.as_str()),
            None,
        )?;
        Ok(Some(AppEvent::ItemCreated { item }))
    }

    fn handle_item_done(&mut self, done: OutputItemEvent) -> AnyResult<Option<AppEvent>> {
        if let Some(item) = self.items.get_mut(&done.output_index) {
            Item::set_raw_data(&mut self.conn, item.id, &done.raw)?;
            item.raw_data = Some(done.raw.to_string());
        }
        Ok(None)
    }

    fn handle_delta(
        &mut self,
        delta: ItemDelta,
        ty: ItemType,
        summary: bool,
    ) -> AnyResult<Option<AppEvent>> {
        if !self.items.contains_key(&delta.output_index) {
            // Lazily create a new item in case we receive out-of-order
            self.create_item(delta.output_index, ty, None, None, None)?;
        }
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

    fn create_item(
        &mut self,
        output_index: i64,
        ty: ItemType,
        upstream_id: Option<&str>,
        upstream_type: Option<&str>,
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
                upstream_call_id: None,
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

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::unbounded_channel;

    use crate::interface::InterfaceId;
    use crate::model::Model;
    use crate::provider::Provider;
    use crate::session::Session;

    use super::*;

    #[tokio::test]
    async fn test_spawn_cancels() {
        let mut conn = crate::db::open_new().unwrap();
        let session = Session::create(&mut conn, "Session").expect("create session");
        let provider = Provider::create(&mut conn, "test", InterfaceId::Openai, "key123", None)
            .expect("create provider");
        let model = Model::create(&mut conn, provider.id, "gpt-4").expect("create model");

        let cancel = CancellationToken::new();
        let (sender, mut recv) = unbounded_channel();
        let agent = Agent::new(
            session,
            provider,
            model,
            sender,
            DefaultClient::default(),
            conn,
            cancel.clone(),
        );
        let task = agent.spawn();
        cancel.cancel();
        task.await.unwrap();
        assert_eq!(recv.try_recv().unwrap(), AppEvent::TaskComplete);
    }
}
