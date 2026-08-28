use diesel::SqliteConnection;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::model::Model;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::Session;
use crate::tasks::{Task, TaskContext};
use crate::tools::DefaultToolServer;
use self::stream_response::StreamResponse;

mod stream_response;
mod execute_tool_call;

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

    async fn process(self, context: &TaskContext) -> AnyResult<()> {
        let (_turn_id, _response_id) = context.subtask(StreamResponse::new(
            self.session,
            self.provider,
            self.model,
            self.client,
            self.tools,
            self.conn,
            None,
        )).await?;
        Ok(())
    }
}

impl Task for Agent {
    type Output = ();

    async fn run(self, context: &mut TaskContext) {
        if let Err(e) = self.process(&context).await {
            log::error!("agent task failed: {e}");
            context.send(AppEvent::ErrorMessage(e.to_string()));
        }
    }
}
