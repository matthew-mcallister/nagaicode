use crate::error::AnyResult;
use crate::interface::{InferenceParams, build_history};
use crate::interface::stream::StreamProcessor;
use crate::item::Item;
use crate::model::Model;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::Session;
use crate::system_prompt::build_system_prompt;
use crate::task::{Task, TaskContext};

pub struct StreamResponse {
    session: Session,
    provider: Provider,
    model: Model,
    client: DefaultClient,

    turn_id: Option<i32>,
}

impl StreamResponse {
    pub fn new(
        session: Session,
        provider: Provider,
        model: Model,
        client: DefaultClient,
        turn_id: Option<i32>,
    ) -> Self {
        Self {
            session,
            provider,
            model,
            client,
            turn_id,
        }
    }

    async fn process(self, context: &mut TaskContext) -> AnyResult<(i32, i32)> {
        let interface = self.provider.create_interface(&self.client)?;

        let history = Item::list_by_session(context.connection()?, self.session.id)?;
        let system_prompt_parts = build_system_prompt(context.cwd())?;
        let messages = build_history(
            context.tool_registry(),
            &system_prompt_parts,
            &history,
            interface.supports_reasoning_input(),
        );
        let params = InferenceParams {
            model_id: &self.model.id,
            system_prompt_parts,
            temperature: 1.0,
            reasoning_effort: None,
            tools: context.tool_registry().list_tool_infos(),
            input: &messages,
        };
        let stream = interface.generate(params);

        let base_seqno = Item::max_seqno(context.connection()?, self.session.id)?.unwrap_or(0) + 1;
        let mut processor = StreamProcessor::new(
            self.session,
            context.sender().clone(),
            context.connection()?,
            self.turn_id,
            base_seqno,
            stream,
            self.provider.id,
            self.provider.name.clone(),
            self.model.id.clone(),
        );
        processor.process().await
    }
}

impl Task for StreamResponse {
    type Output = AnyResult<(i32, i32)>;

    async fn run(self, context: &mut TaskContext) -> AnyResult<(i32, i32)> {
        self.process(context).await
    }
}