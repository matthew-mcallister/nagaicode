use diesel::SqliteConnection;

use crate::app::AppEvent;
use crate::model::Model;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::Session;
use crate::tasks::{Task, TaskContext, TaskError};
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
