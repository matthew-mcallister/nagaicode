use futures::future::join_all;

use crate::app::AppEvent;
use crate::error::AnyResult;
use crate::model::Model;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::{Item, Session};
use crate::task::{Task, TaskContext};
use self::create_prompt::CreatePrompt;
use self::execute_tool_call::ExecuteToolCall;
use self::stream_response::StreamResponse;

mod create_prompt;
mod execute_tool_call;
mod stream_response;

pub struct Agent {
    pub session: Session,
    pub provider: Provider,
    pub model: Model,
    pub client: DefaultClient,
    pub prompt: String,
}

impl Agent {
    pub fn new(
        session: Session,
        provider: Provider,
        model: Model,
        client: DefaultClient,
        prompt: String,
    ) -> Self {
        Self {
            session,
            provider,
            model,
            client,
            prompt,
        }
    }

    /// Runs the model/tool loop until the model stops requesting tools.
    async fn process(self, context: &mut TaskContext) -> AnyResult<()> {
        context
            .subtask(CreatePrompt::new(self.session.id, &self.prompt))
            .await?;
        let mut turn_id = None;
        loop {
            let (next_turn_id, response_id) = context
                .subtask(StreamResponse::new(
                    self.session.clone(),
                    self.provider.clone(),
                    self.model.clone(),
                    self.client.clone(),
                    turn_id,
                ))
                .await?;
            turn_id = Some(next_turn_id);

            let tool_calls =
                Item::tool_calls_by_response(context.connection()?, response_id)?;
            if tool_calls.is_empty() {
                return Ok(());
            }

            let executions: Vec<_> = tool_calls
                .into_iter()
                .map(|tool_call| {
                    Box::pin(context.subtask(ExecuteToolCall::new(tool_call)))
                })
                .collect();
            for result in join_all(executions).await {
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
