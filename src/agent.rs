use diesel::SqliteConnection;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::interface::stream::StreamProcessor;
use crate::interface::{InferenceParams, build_history};
use crate::model::Model;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::{Item, Session};
use crate::tasks::{Task, TaskContext, TaskError};
use crate::tools::DefaultToolServer;

pub struct Agent {
    pub session: Session,
    pub provider: Provider,
    pub model: Model,
    pub client: DefaultClient,
    pub tools: DefaultToolServer,
    pub conn: SqliteConnection,
}

impl Agent {
    pub fn new(
        session: Session,
        provider: Provider,
        model: Model,
        client: DefaultClient,
        tools: DefaultToolServer,
        conn: SqliteConnection,
    ) -> Self {
        Self {
            session,
            provider,
            model,
            client,
            tools,
            conn,
        }
    }
}

impl Task for Agent {
    type Output = ();

    async fn run(self, context: TaskContext) {
        let handle = context.spawn(StreamResponse::new(
            self.session,
            self.provider,
            self.model,
            self.client,
            self.tools,
            self.conn,
            None,
        ));
        match handle.join().await {
            Ok(Ok(result)) => {
                if let Err(e) = result {
                    context.send(AppEvent::ErrorMessage(e.to_string()));
                }
            }
            Ok(Err(TaskError::Canceled)) => {}
            Err(e) => log::error!("agent task failed: {e}"),
        }
    }
}

struct StreamResponse {
    session: Session,
    provider: Provider,
    model: Model,
    client: DefaultClient,
    tools: DefaultToolServer,
    conn: SqliteConnection,

    turn_id: Option<i32>,
}

impl StreamResponse {
    fn new(
        session: Session,
        provider: Provider,
        model: Model,
        client: DefaultClient,
        tools: DefaultToolServer,
        conn: SqliteConnection,
        turn_id: Option<i32>,
    ) -> Self {
        Self {
            session,
            provider,
            model,
            client,
            tools,
            conn,
            turn_id,
        }
    }

    async fn process(mut self, context: &TaskContext) -> AnyResult<(i32, i32)> {
        let interface = self.provider.create_interface(&self.client)?;

        let history = Item::list_by_session(&mut self.conn, self.session.id)?;
        let messages = build_history(&history, interface.supports_reasoning_input())?;
        let mut params = self.build_params();
        params.input = &messages;
        let stream = interface.generate(params, &self.tools);

        let mut processor = StreamProcessor::new(
            self.session,
            self.conn,
            self.turn_id,
            stream,
            self.provider.id,
            self.provider.name.clone(),
            self.model.id.clone(),
        );
        processor.process(context.sender()).await
    }

    fn build_params(&self) -> InferenceParams<'_> {
        InferenceParams {
            model_id: &self.model.id,
            system_prompt: "",
            temperature: 1.0,
            reasoning_effort: None,
            input: &[],
        }
    }
}

impl Task for StreamResponse {
    type Output = AnyResult<(i32, i32)>;

    async fn run(self, context: TaskContext) -> AnyResult<(i32, i32)> {
        self.process(&context).await
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
            DefaultToolServer::default(),
            conn,
        ));
        handle.cancel();
        let result = handle.join().await.unwrap();
        assert_eq!(result, Err(TaskError::Canceled));
        assert_eq!(recv.try_recv().unwrap(), AppEvent::TaskStarted(0));
        assert_eq!(recv.try_recv().unwrap(), AppEvent::TaskEnded(0));
    }
}
