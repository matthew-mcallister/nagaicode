use crate::error::AnyResult;
use crate::interface::{InferenceParams, build_history};
use crate::interface::stream::StreamProcessor;
use crate::model::Model;
use crate::provider::Provider;
use crate::request::DefaultClient;
use crate::session::{Item, Session};
use crate::tasks::{Task, TaskContext};

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
        let messages = build_history(&history, interface.supports_reasoning_input())?;
        let mut params = self.build_params();
        params.input = &messages;
        let stream = interface.generate(params, context.tools());

        let sender = context.sender().clone();
        let mut processor = StreamProcessor::new(
            self.session,
            context.connection()?,
            self.turn_id,
            stream,
            self.provider.id,
            self.provider.name.clone(),
            self.model.id.clone(),
        );
        processor.process(&sender).await
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

    async fn run(self, context: &mut TaskContext) -> AnyResult<(i32, i32)> {
        self.process(context).await
    }
}