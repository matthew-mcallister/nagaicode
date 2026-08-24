use diesel::SqliteConnection;
use futures::StreamExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::interface::{InferenceEvent, InferenceParams, build_history};
use crate::model::Model;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::{Chain, Content, ContentType, Item, ItemType, Session};

pub struct Agent {
    pub session: Session,
    pub provider: Provider,
    pub model: Model,
    pub sender: UnboundedSender<AppEvent>,
    pub client: DefaultClient,
    pub conn: SqliteConnection,
    pub cancel: CancellationToken,

    response_id: Option<String>,
    chain: Option<Chain>,
    model_item: Option<Item>,
    thought: Option<Content>,
    text: Option<Content>,
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
            response_id: None,
            chain: None,
            model_item: None,
            thought: None,
            text: None,
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
                    Some(Err(e)) => return Err(e),
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

    /// Loads the session's (item, content) pairs sorted by item and content id.
    fn load_history(&mut self) -> AnyResult<Vec<(Item, Content)>> {
        let items = Item::list_by_session(&mut self.conn, self.session.id)?;
        let mut pairs = Vec::new();
        for item in items {
            let contents = Content::list_by_item(&mut self.conn, item.id)?;
            for content in contents {
                pairs.push((item.clone(), content));
            }
        }
        Ok(pairs)
    }

    fn handle_event(&mut self, event: InferenceEvent) -> AnyResult<Option<AppEvent>> {
        match event {
            InferenceEvent::Created(created) => {
                self.response_id = Some(created.id);
                Ok(None)
            }
            InferenceEvent::ThinkingDelta(delta) => {
                self.ensure_model_item()?;
                Self::handle_delta(
                    &mut self.conn,
                    &self.model_item,
                    &mut self.thought,
                    delta,
                    ContentType::Thought,
                )
            }
            InferenceEvent::OutputDelta(delta) => {
                self.ensure_model_item()?;
                Self::handle_delta(
                    &mut self.conn,
                    &self.model_item,
                    &mut self.text,
                    delta,
                    ContentType::Text,
                )
            }
            InferenceEvent::Completed(_) => Ok(None),
        }
    }

    fn handle_delta(
        conn: &mut SqliteConnection,
        model_item: &Option<Item>,
        slot: &mut Option<Content>,
        delta: String,
        ty: ContentType,
    ) -> AnyResult<Option<AppEvent>> {
        let item_id = model_item.as_ref().expect("model item exists").id;
        if let Some(content) = slot {
            content.value.push_str(&delta);
            Content::update_value(conn, content.id, &content.value)?;
            let item = model_item.clone().expect("model item exists");
            Ok(Some(AppEvent::ContentUpdated {
                item,
                content: content.clone(),
            }))
        } else {
            let content = Content::create(conn, item_id, ty, &delta)?;
            let item = model_item.clone().expect("model item exists");
            *slot = Some(content.clone());
            Ok(Some(AppEvent::ContentCreated { item, content }))
        }
    }

    fn ensure_chain(&mut self) -> AnyResult<i32> {
        if let Some(chain) = &self.chain {
            return Ok(chain.id);
        }
        let chain = Chain::create(
            &mut self.conn,
            self.session.id,
            self.provider.id,
            &self.provider.name,
            &self.model.id,
        )?;
        let id = chain.id;
        self.chain = Some(chain);
        Ok(id)
    }

    fn ensure_model_item(&mut self) -> AnyResult<i32> {
        if let Some(item) = &self.model_item {
            return Ok(item.id);
        }
        let chain_id = self.ensure_chain()?;
        let item = Item::create(
            &mut self.conn,
            self.session.id,
            Some(chain_id),
            ItemType::Model,
            self.response_id.as_deref(),
        )?;
        let id = item.id;
        self.model_item = Some(item);
        Ok(id)
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
        let provider =
            Provider::create(&mut conn, "test", InterfaceId::Openai, "key123", None)
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
        assert!(recv.try_recv().is_err());
    }
}
