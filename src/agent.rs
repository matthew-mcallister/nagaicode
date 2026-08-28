use futures::future::join_all;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::model::Model;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::{Item, Session};
use crate::tasks::{Task, TaskContext};
use crate::tools::DefaultToolServer;
use self::execute_tool_call::ExecuteToolCall;
use self::stream_response::StreamResponse;

mod execute_tool_call;
mod stream_response;

pub struct Agent {
    pub session: Session,
    pub provider: Provider,
    pub model: Model,
    pub client: DefaultClient,
    pub tools: DefaultToolServer,
}

impl Agent {
    pub fn new(
        session: Session,
        provider: Provider,
        model: Model,
        client: DefaultClient,
        tools: DefaultToolServer,
    ) -> Self {
        Self {
            session,
            provider,
            model,
            client,
            tools,
        }
    }

    /// Runs the model/tool loop until the model stops requesting tools.
    async fn process(self, context: &mut TaskContext) -> AnyResult<()> {
        let mut turn_id = None;
        loop {
            let (next_turn_id, response_id) = context
                .subtask(StreamResponse::new(
                    self.session.clone(),
                    self.provider.clone(),
                    self.model.clone(),
                    self.client.clone(),
                    self.tools.clone(),
                    turn_id,
                ))
                .await?;
            turn_id = Some(next_turn_id);

            let tool_call_ids =
                Item::tool_call_ids_by_response(context.connection()?, response_id)?;
            if tool_call_ids.is_empty() {
                return Ok(());
            }

            let tool_calls: Vec<_> = tool_call_ids
                .into_iter()
                .map(|id| {
                    Box::pin(context.subtask(ExecuteToolCall::new(id, self.tools.clone())))
                })
                .collect();
            for result in join_all(tool_calls).await {
                result?;
            }
        }
    }
}

impl Task for Agent {
    type Output = ();

    async fn run(self, context: &mut TaskContext) {
        if let Err(e) = self.process(context).await {
            log::error!("agent task failed: {e}");
            context.send(AppEvent::ErrorMessage(e.to_string()));
        }
    }
}
