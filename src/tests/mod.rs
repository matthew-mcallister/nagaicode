use crossterm::event::{Event, KeyCode, KeyEvent};
use reqwest::{Method, Request};
use reqwest_eventsource::Event as SseEvent;
use serde_json::json;

use crate::app::{App, AppEvent};
use crate::error::AnyResult;
use crate::interface::InterfaceId;
use crate::model::Model;
use crate::provider::Provider;
use crate::query::DataQuery;
use crate::request::test_client::ResponseData;
use crate::testing::QueueStream;
use crate::tools::ToolResult;
use crate::tools::mock::ToolCall;
use crate::ui::canvas::render_canvas;

fn create_message_event(data: &str) -> SseEvent {
    SseEvent::Message(eventsource_stream::Event {
        event: "message".to_string(),
        data: data.to_string(),
        id: "".to_string(),
        retry: None,
    })
}

fn request_body_value(req: &Request) -> serde_json::Value {
    let bytes = req.body().and_then(|b| b.as_bytes()).unwrap_or(&[]);
    serde_json::from_slice(bytes).unwrap()
}

const EXPECTED_INITIAL_FRAME: &str = concat!(
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[38;2;168;162;158m  ▐ \x1b[3mWelcome to NagaiCode!                                                  \x1b[38;2;87;83;78m\x1b[23m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[38;2;168;162;158m  ▐ \x1b[3m                                                                       \x1b[38;2;87;83;78m\x1b[23m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[38;2;168;162;158m  ▐ \x1b[3mType /help for a list of commands.                                     \x1b[38;2;87;83;78m\x1b[23m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                             \x1b[38;2;87;83;78m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[48;2;41;37;36m                                                                           \x1b[38;2;56;189;248m\x1b[48;2;12;10;9m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[48;2;41;37;36m                                                                           \x1b[38;2;56;189;248m\x1b[48;2;12;10;9m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[48;2;41;37;36m                                                                           \x1b[38;2;56;189;248m\x1b[48;2;12;10;9m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                ",
);

#[test]
fn test_app_e2e() {
    let mut app = App::new().expect("failed to create app");

    let mut canvas = app.make_canvas();
    app.draw(&mut canvas);
    let frame = render_canvas(&mut canvas);

    assert_eq!(frame, EXPECTED_INITIAL_FRAME);
    assert!(!app.quit());

    app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char('/'))));
    app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char('q'))));
    app.handle_input(Event::Key(KeyEvent::from(KeyCode::Enter)));

    assert!(app.quit());
}

#[tokio::test]
async fn test_app_interrupt() {
    let mut app = App::new().unwrap();

    let provider = Provider::create(app.conn(), "test", InterfaceId::Openai, "key", None)
        .expect("create provider");
    let model = Model::create(app.conn(), provider.id, "gpt-4").expect("create model");
    app.switch_model(model);

    app.process_event(AppEvent::Interrupt);
    assert!(app.task_canceled());

    app.process_command("hello").unwrap();
    assert!(!app.task_canceled());

    app.process_event(AppEvent::Interrupt);
    assert!(app.task_canceled());
}

#[test]
fn test_app_process_command() {
    let mut app = App::new().unwrap();

    app.tools_mut().add_result(
        "sh",
        Ok(ToolResult::success(json!({
            "stdout": "output line\n",
            "stderr": "",
            "return_code": 0,
        }))),
    );

    app.process_command("!echo test").unwrap();

    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        ToolCall {
            name: "sh".to_owned(),
            args: json!("echo test"),
        }
    );

    app.tools_mut().add_result(
        "sh",
        Ok(ToolResult::success(json!("string output"))),
    );
    app.process_command("!pwd").unwrap();
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].args, json!("pwd"));

    app.tools_mut().add_result(
        "sh",
        Ok(ToolResult::error(json!({
            "stdout": "",
            "stderr": "error message\n",
            "return_code": 1,
        }))),
    );
    app.process_command("!false").unwrap();
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 3);

    app.tools_mut().add_result(
        "sh",
        Ok(ToolResult::success(json!({
            "stdout": "",
            "stderr": "",
            "return_code": 0,
        }))),
    );
    app.process_command("!true").unwrap();
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 4);

    app.tools_mut().add_result(
        "sh",
        Err("tool error".into()),
    );
    app.process_event(AppEvent::Command("!failing_tool".to_string()));
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 5);

    assert!(app.process_command("").is_ok());
    assert!(app.process_command("   ").is_ok());
}

#[tokio::test]
async fn test_app_prompt_agent() {
    use crate::session::{Chain, Content, Item, ItemType};

    let mut app = App::new().unwrap();

    let provider = Provider::create(
        app.conn(),
        "test",
        InterfaceId::Openai,
        "sk-test",
        Some("https://example.test/v1"),
    )
    .expect("create provider");
    let model = Model::create(app.conn(), provider.id, "gpt-4").expect("create model");
    app.switch_model(model);

    let url = "https://example.test/v1/responses";
    let events: Vec<AnyResult<SseEvent>> = vec![
        Ok(SseEvent::Open),
        Ok(create_message_event(
            r#"{"type":"response.created","response":{"id":"resp-1"}}"#,
        )),
        Ok(create_message_event(
            r#"{"type":"response.output_text.delta","delta":"Hello there, "}"#,
        )),
        Ok(create_message_event(
            r#"{"type":"response.output_text.delta","delta":"how can I help?"}"#,
        )),
        Ok(create_message_event(
            r#"{"type":"response.completed","response":{"id":"resp-1","usage":{"input_tokens":12,"output_tokens":18,"output_tokens_details":{"reasoning_tokens":7}}}}"#,
        )),
    ];
    app.client_mut()
        .add_response(url, ResponseData::Sse(QueueStream::from(events)));

    // Drive the terminal to input a prompt and submit it.
    for c in "hello".chars() {
        app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char(c))));
    }
    app.handle_input(Event::Key(KeyEvent::from(KeyCode::Enter)));

    // Await the spawned agent, then fold its events into the UI.
    app.await_task().await.expect("await agent task");
    app.process_pending_events();

    // The agent should have made a single inference request.
    let requests = app.client_mut().get_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method(), Method::POST);
    assert_eq!(requests[0].url().as_str(), url);
    let body = request_body_value(&requests[0]);
    assert_eq!(body["model"], "gpt-4");
    assert_eq!(body["stream"], true);
    assert_eq!(body["input"][0]["role"], "user");
    assert_eq!(body["input"][0]["content"], "hello");

    // The prompt should be recorded in the app state.
    assert_eq!(
        app.query("/chat/stacked/inner/input/command_history").unwrap(),
        json!(["hello"])
    );

    // The model response (distinct from the prompt) should appear in the chat
    // history alongside the prompt. The initial help message is also present.
    let history: Vec<String> = app
        .query("/chat/stacked/inner/history/history/items")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["content"].as_str().unwrap().to_owned())
        .collect();
    assert!(history.iter().any(|c| c == "hello"));
    assert_eq!(
        history.last().map(String::as_str),
        Some("Hello there, how can I help?")
    );

    // The prompt and the response should both be persisted in the database.
    let session_id = app.query("/session/id").unwrap().as_i64().unwrap() as i32;
    let items = Item::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(items.len(), 2, "expected one user item and one model item");

    assert_eq!(items[0].ty().unwrap(), ItemType::User);
    let user_contents = Content::list_by_item(app.conn(), items[0].id).unwrap();
    assert_eq!(user_contents.len(), 1);
    assert_eq!(user_contents[0].ty, "text");
    assert_eq!(user_contents[0].value, "hello");

    let model_item = &items[1];
    assert_eq!(model_item.ty().unwrap(), ItemType::Model);
    assert_eq!(model_item.response_id.as_deref(), Some("resp-1"));
    let chain_id = model_item.chain_id.expect("model item has a chain");
    let model_contents = Content::list_by_item(app.conn(), model_item.id).unwrap();
    assert_eq!(model_contents.len(), 1);
    assert_eq!(model_contents[0].value, "Hello there, how can I help?");

    let chain = Chain::get_by_id(app.conn(), chain_id)
        .unwrap()
        .expect("chain exists");
    assert_eq!(chain.session_id, session_id);
    assert_eq!(chain.provider_id, provider.id);
    assert_eq!(chain.model_id, "gpt-4");
}

#[cfg(test)]
mod tests {
    use crate::interface::InterfaceId;
    use crate::model::Model;
    use crate::provider::Provider;
    use crate::query::DataQuery;
    use crate::ui::chat::Chat;
    use crate::ui::style::THEME_DARK;
    use super::*;

    #[tokio::test]
    async fn test_app_query() {
        let mut app = App::new().unwrap();

        // Primitive and unset fields.
        let db_url = app.query("/db_url").unwrap();
        assert!(db_url.as_str().unwrap().starts_with("file:"));
        assert!(db_url.as_str().unwrap().contains("mode=memory"));
        assert_eq!(app.query("/selected_model").unwrap(), json!(null));
        assert_eq!(app.query("/session").unwrap(), json!(null));

        // Nested query into chat.
        let expected_chat = Chat::new(80, 24, &THEME_DARK).query("/").unwrap();
        assert_eq!(app.query("/chat").unwrap(), expected_chat);
        assert_eq!(app.query("/chat/stacked/h_padding").unwrap(), json!(2));
        assert_eq!(app.query("/chat/stacked/v_padding").unwrap(), json!(1));
        assert_eq!(app.query("/chat/stacked/inner/focus_state").unwrap(), json!("command_editor"));

        assert_eq!(app.query("/").unwrap(), json!({
            "chat": expected_chat,
            "selected_model": null,
            "db_url": db_url,
            "session": null,
        }));

        // Selecting a model exposes it as a nested query.
        let provider = Provider::create(app.conn(), "test", InterfaceId::Openai, "key123", None).unwrap();
        let model = Model::create(app.conn(), provider.id, "gpt-4").unwrap();
        app.switch_model(model.clone());
        assert_eq!(app.query("/selected_model/id").unwrap(), json!("gpt-4"));
        assert_eq!(app.query("/selected_model").unwrap(), model.query("/").unwrap());
    }
}
